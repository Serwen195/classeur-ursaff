//! Synchronisation avec le dépôt Git distant (libgit2 embarqué : l'utilisateur n'a pas besoin
//! d'installer Git).
//!
//! Garanties :
//! * seuls les chemins de la liste blanche (`store::is_committable_path`) sont jamais indexés ;
//! * avant tout `push`, chaque commit sortant est relu : un chemin hors liste blanche, ou un
//!   fichier `.enc` qui ne commence pas par l'en-tête de format chiffré, bloque l'envoi ;
//! * le jeton d'accès n'est ni écrit dans la configuration Git ni dans l'URL : il est fourni à
//!   libgit2 au moment de l'authentification, depuis la mémoire.

use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use git2::{
    build::{CheckoutBuilder, RepoBuilder},
    Cred, CredentialType, Delta, FetchOptions, IndexEntry, IndexTime, MergeOptions, Oid, ProxyOptions,
    PushOptions, RemoteCallbacks, Repository, Signature, StatusOptions,
};

use crate::crypto::MAGIC;
use crate::error::{Error, Result};
use crate::store::{is_committable_path, FILES_DIR, RECORDS_DIR};

const REMOTE: &str = "origin";
const DEFAULT_BRANCH: &str = "main";
const AUTHOR_NAME: &str = "Classeur URSSAF";
const AUTHOR_EMAIL: &str = "classeur-urssaf@users.noreply.github.com";
const MAX_PUSH_ATTEMPTS: usize = 3;
/// Nombre maximal de fois où le jeton est proposé *au cours d'une même opération réseau*. Un échange
/// Git en HTTP comprend plusieurs requêtes (annonce des références, puis envoi ou réception) et
/// chacune peut redemander des identifiants : une seule proposition ne suffit donc pas, mais une
/// borne évite de marteler le serveur avec un jeton refusé.
const MAX_CREDENTIAL_OFFERS: u8 = 3;

const README: &str = "# Données de Classeur URSSAF\n\n\
Ce dépôt contient des données **chiffrées** (AES-256-GCM, clé dérivée d'une phrase secrète avec \
Argon2id). Il est géré automatiquement par l'application : ne modifiez pas ces fichiers à la main.\n\n\
Sans la phrase secrète, les données sont irrécupérables.\n";
const GITATTRIBUTES: &str = "* -text\n*.enc binary\n";
const GITIGNORE: &str = "*.tmp\n.DS_Store\nThumbs.db\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConflict {
    pub path: String,
    pub kept: Side,
}

/// Arbitre un fichier modifié des deux côtés. `None` = le fichier est supprimé de ce côté.
/// Doit renvoyer le côté à conserver (si ce côté est une suppression, le fichier disparaît).
pub trait ConflictResolver {
    fn resolve(&mut self, path: &str, ours: Option<&[u8]>, theirs: Option<&[u8]>) -> Result<Side>;
}

impl<F> ConflictResolver for F
where
    F: FnMut(&str, Option<&[u8]>, Option<&[u8]>) -> Result<Side>,
{
    fn resolve(&mut self, path: &str, ours: Option<&[u8]>, theirs: Option<&[u8]>) -> Result<Side> {
        self(path, ours, theirs)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncReport {
    /// Des changements distants ont été appliqués au dossier de travail.
    pub pulled: bool,
    /// Des commits locaux ont été envoyés.
    pub pushed: bool,
    pub merged: bool,
    pub resolved: Vec<ResolvedConflict>,
    /// Historiques divergents mais aucun arbitre fourni (coffre verrouillé) : à refaire une fois
    /// déverrouillé. Rien n'a été modifié ni envoyé.
    pub deferred: bool,
    /// Fichiers non reconnus laissés dans le dossier, volontairement ignorés.
    pub ignored_files: Vec<String>,
}

/// Dépôt local des données. Une seule instance (partagée) doit exister par dossier : elle porte le
/// verrou du dossier de travail, qui empêche une sauvegarde d'être écrasée par le checkout d'une
/// fusion en cours. Le verrou n'est jamais tenu pendant un échange réseau.
#[derive(Debug)]
pub struct DataRepo {
    path: PathBuf,
    worktree: Mutex<()>,
}

/// Preuve que le dossier de travail est verrouillé : permet d'écrire des fichiers puis de les
/// valider (commit) sans qu'une synchronisation s'intercale.
pub struct WorktreeGuard<'a> {
    repo: &'a DataRepo,
    _guard: MutexGuard<'a, ()>,
}

impl WorktreeGuard<'_> {
    pub fn commit_pending(&self, message: &str, ignored: &mut Vec<String>) -> Result<bool> {
        self.repo.commit_pending_locked(message, ignored)
    }
}

/// Délais réseau (connexion 15 s, échange 30 s) pour qu'un lancement hors ligne n'attende pas
/// indéfiniment. Réglage global de libgit2 : à appeler une fois au démarrage.
pub fn init_network_timeouts() {
    // SAFETY : simple écriture de deux entiers dans la configuration globale de libgit2,
    // appelée avant toute opération réseau.
    unsafe {
        let _ = git2::opts::set_server_connect_timeout_in_milliseconds(15_000);
        let _ = git2::opts::set_server_timeout_in_milliseconds(30_000);
    }
}

impl DataRepo {
    pub fn open(path: &Path) -> Result<DataRepo> {
        Repository::open(path)?;
        Ok(DataRepo { path: path.to_path_buf(), worktree: Mutex::new(()) })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Verrouille le dossier de travail (bloque pendant une fusion ou un fast-forward en cours).
    pub fn lock(&self) -> WorktreeGuard<'_> {
        WorktreeGuard { repo: self, _guard: self.worktree.lock().unwrap_or_else(|e| e.into_inner()) }
    }

    /// Clone le dépôt distant dans `path` (dossier absent ou vide).
    pub fn clone_from(url: &str, path: &Path, token: Option<&str>) -> Result<DataRepo> {
        if path.exists() && fs::read_dir(path)?.next().is_some() {
            return Err(Error::InvalidInput("Le dossier local du classeur n'est pas vide.".into()));
        }
        let auth = AuthState::new(token);
        let mut fo = FetchOptions::new();
        fo.remote_callbacks(auth.callbacks());
        fo.proxy_options(proxy());
        let repo = RepoBuilder::new().fetch_options(fo).clone(url, path).map_err(|e| auth.map_error(e))?;
        configure(&repo)?;
        if repo.head().is_err() {
            adopt_remote_branch_or_start_main(&repo)?;
        }
        Ok(DataRepo { path: path.to_path_buf(), worktree: Mutex::new(()) })
    }

    /// Écrit les fichiers d'accompagnement (README, .gitattributes, .gitignore) s'ils manquent.
    pub fn write_scaffold(&self) -> Result<()> {
        for (name, content) in
            [("README.md", README), (".gitattributes", GITATTRIBUTES), (".gitignore", GITIGNORE)]
        {
            let p = self.path.join(name);
            if !p.exists() {
                crate::fsutil::atomic_write(&p, content.as_bytes())?;
            }
        }
        Ok(())
    }

    pub fn remote_url(&self) -> Result<Option<String>> {
        let repo = Repository::open(&self.path)?;
        let url = repo.find_remote(REMOTE).ok().and_then(|r| r.url().ok().map(str::to_string));
        Ok(url)
    }

    /// Des commits locaux ne sont pas encore sur le serveur (d'après la dernière récupération).
    pub fn has_unpushed(&self) -> Result<bool> {
        let repo = Repository::open(&self.path)?;
        let Some(local) = head_oid(&repo) else { return Ok(false) };
        let branch = branch_name(&repo)?;
        Ok(tracking_oid(&repo, &branch) != Some(local) || has_dirty_allowed(&repo)?)
    }

    /// Valide (commit) les changements des fichiers de la liste blanche. Retourne `true` si un
    /// commit a été créé. Les autres fichiers ne sont jamais indexés ; ils sont listés dans `ignored`.
    pub fn commit_pending(&self, message: &str, ignored: &mut Vec<String>) -> Result<bool> {
        self.lock().commit_pending(message, ignored)
    }

    fn commit_pending_locked(&self, message: &str, ignored: &mut Vec<String>) -> Result<bool> {
        let repo = Repository::open(&self.path)?;
        let mut opts = StatusOptions::new();
        opts.include_untracked(true).recurse_untracked_dirs(true).include_ignored(false);
        let statuses = repo.statuses(Some(&mut opts))?;
        let mut index = repo.index()?;
        let mut changed = false;
        for entry in statuses.iter() {
            let Ok(path) = entry.path() else { continue };
            if !is_committable_path(path) {
                if !ignored.iter().any(|p| p == path) {
                    ignored.push(path.to_string());
                }
                continue;
            }
            if self.path.join(path).exists() {
                index.add_path(Path::new(path))?;
            } else {
                // `remove_path` échoue si le chemin n'est pas indexé (ex. fichier créé puis supprimé).
                let _ = index.remove_path(Path::new(path));
            }
            changed = true;
        }
        if !changed {
            return Ok(false);
        }
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let parent = head_oid(&repo).map(|o| repo.find_commit(o)).transpose()?;
        if let Some(p) = &parent {
            if p.tree_id() == tree_id {
                return Ok(false); // état déjà identique au dernier commit
            }
        }
        let sig = signature()?;
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)?;
        Ok(true)
    }

    /// Récupère, fusionne si besoin, puis envoie. `resolver` = `None` quand le coffre est
    /// verrouillé : on avance uniquement par fast-forward et on pousse si possible.
    pub fn sync(
        &self,
        token: Option<&str>,
        mut resolver: Option<&mut dyn ConflictResolver>,
    ) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        self.commit_pending("Mise à jour du classeur", &mut report.ignored_files)?;

        let repo = Repository::open(&self.path)?;
        let branch = branch_name(&repo)?;

        for _ in 0..MAX_PUSH_ATTEMPTS {
            fetch(&repo, &branch, token)?;
            let local = head_oid(&repo);
            let remote = tracking_oid(&repo, &branch);

            match (local, remote) {
                (None, None) => return Ok(report),
                (None, Some(r)) => {
                    let _g = self.lock();
                    if head_oid(&repo).is_some() {
                        continue; // une sauvegarde vient d'avoir lieu : on réévalue
                    }
                    fast_forward(&repo, &branch, r)?;
                    report.pulled = true;
                    return Ok(report);
                }
                (Some(l), None) => {
                    verify_outgoing(&repo, l, None)?;
                    if push(&repo, &branch, token)? {
                        report.pushed = true;
                        return Ok(report);
                    }
                }
                (Some(l), Some(r)) if l == r => return Ok(report),
                (Some(l), Some(r)) => {
                    if repo.graph_descendant_of(l, r)? {
                        // En avance : il ne reste qu'à envoyer.
                        verify_outgoing(&repo, l, Some(r))?;
                        if push(&repo, &branch, token)? {
                            report.pushed = true;
                            return Ok(report);
                        }
                    } else if repo.graph_descendant_of(r, l)? {
                        let _g = self.lock();
                        if head_oid(&repo) != Some(l) {
                            continue;
                        }
                        fast_forward(&repo, &branch, r)?;
                        report.pulled = true;
                        return Ok(report);
                    } else {
                        let Some(res) = resolver.as_deref_mut() else {
                            report.deferred = true;
                            return Ok(report);
                        };
                        let head = {
                            let _g = self.lock();
                            if head_oid(&repo) != Some(l) {
                                continue;
                            }
                            let merged = merge(&repo, &branch, l, r, res)?;
                            report.merged = true;
                            report.pulled = true;
                            report.resolved.extend(merged);
                            head_oid(&repo).expect("la fusion vient de créer un commit")
                        };
                        verify_outgoing(&repo, head, Some(r))?;
                        if push(&repo, &branch, token)? {
                            report.pushed = true;
                            return Ok(report);
                        }
                    }
                }
            }
            // Push refusé : quelqu'un d'autre a poussé entre-temps. On recommence.
        }
        Err(Error::Network(
            "le dépôt distant change trop souvent pour terminer l'envoi, réessayez dans un instant".into(),
        ))
    }
}

fn configure(repo: &Repository) -> Result<()> {
    let mut cfg = repo.config()?;
    // Des fichiers chiffrés ne doivent subir aucune conversion de fin de ligne.
    cfg.set_str("core.autocrlf", "false")?;
    cfg.set_str("core.safecrlf", "false")?;
    cfg.set_str("user.name", AUTHOR_NAME)?;
    cfg.set_str("user.email", AUTHOR_EMAIL)?;
    Ok(())
}

/// Clone dont `HEAD` pointe vers une branche absente. Deux cas : le dépôt distant est vide
/// (on démarre sur `main`) ou ses données sont sur une autre branche que celle annoncée par
/// défaut (on adopte `main` si elle existe, sinon l'unique branche distante).
fn adopt_remote_branch_or_start_main(repo: &Repository) -> Result<()> {
    let mut remote_branches: Vec<String> = Vec::new();
    for r in repo.references_glob(&format!("refs/remotes/{REMOTE}/*"))? {
        let name = r?.name().unwrap_or_default().to_string();
        if let Some(b) = name.strip_prefix(&format!("refs/remotes/{REMOTE}/")) {
            if b != "HEAD" {
                remote_branches.push(b.to_string());
            }
        }
    }
    let chosen = if remote_branches.iter().any(|b| b == DEFAULT_BRANCH) {
        Some(DEFAULT_BRANCH.to_string())
    } else if remote_branches.len() == 1 {
        remote_branches.pop()
    } else {
        None
    };
    match chosen {
        Some(b) => {
            let target = tracking_oid(repo, &b).expect("la référence distante vient d'être listée");
            fast_forward(repo, &b, target)
        }
        None => Ok(repo.set_head(&format!("refs/heads/{DEFAULT_BRANCH}"))?),
    }
}

fn signature() -> Result<Signature<'static>> {
    Ok(Signature::now(AUTHOR_NAME, AUTHOR_EMAIL)?)
}

fn proxy() -> ProxyOptions<'static> {
    let mut p = ProxyOptions::new();
    p.auto();
    p
}

fn head_oid(repo: &Repository) -> Option<Oid> {
    repo.head().ok().and_then(|h| h.target())
}

fn branch_name(repo: &Repository) -> Result<String> {
    let head = repo.find_reference("HEAD")?;
    let detached = || Error::Git("HEAD détaché : état inattendu du dossier de données".into());
    let target = head.symbolic_target().map_err(|_| detached())?.ok_or_else(detached)?;
    Ok(target.strip_prefix("refs/heads/").unwrap_or(target).to_string())
}

fn tracking_oid(repo: &Repository, branch: &str) -> Option<Oid> {
    repo.find_reference(&format!("refs/remotes/{REMOTE}/{branch}")).ok().and_then(|r| r.target())
}

fn has_dirty_allowed(repo: &Repository) -> Result<bool> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    Ok(repo.statuses(Some(&mut opts))?.iter().any(|e| e.path().is_ok_and(is_committable_path)))
}

/// Gère les identifiants d'*une* opération réseau (clone, fetch ou push) : en créer un par
/// opération. Le jeton est proposé au plus `MAX_CREDENTIAL_OFFERS` fois, pour éviter la boucle
/// infinie de libgit2 quand le serveur le refuse.
struct AuthState<'a> {
    token: Option<&'a str>,
    offered: Cell<u8>,
    rejected: Cell<bool>,
}

impl<'a> AuthState<'a> {
    fn new(token: Option<&'a str>) -> Self {
        AuthState { token, offered: Cell::new(0), rejected: Cell::new(false) }
    }

    fn callbacks(&self) -> RemoteCallbacks<'_> {
        let mut cb = RemoteCallbacks::new();
        cb.credentials(move |_url, _user, allowed| {
            if allowed.contains(CredentialType::USER_PASS_PLAINTEXT) {
                if let (Some(token), true) = (self.token, self.offered.get() < MAX_CREDENTIAL_OFFERS) {
                    self.offered.set(self.offered.get() + 1);
                    // « x-access-token » est accepté par GitHub avec n'importe quel type de jeton.
                    return Cred::userpass_plaintext("x-access-token", token);
                }
            }
            self.rejected.set(true);
            Err(git2::Error::new(git2::ErrorCode::Auth, git2::ErrorClass::Http, "authentification refusée"))
        });
        cb
    }

    fn map_error(&self, e: git2::Error) -> Error {
        if self.rejected.get() {
            Error::AuthFailed
        } else {
            e.into()
        }
    }
}

fn fetch(repo: &Repository, branch: &str, token: Option<&str>) -> Result<()> {
    let auth = AuthState::new(token);
    let mut remote = repo.find_remote(REMOTE)?;
    let mut fo = FetchOptions::new();
    fo.remote_callbacks(auth.callbacks());
    fo.proxy_options(proxy());
    let spec = format!("+refs/heads/{branch}:refs/remotes/{REMOTE}/{branch}");
    remote.fetch(&[spec.as_str()], Some(&mut fo), None).map_err(|e| auth.map_error(e))?;
    Ok(())
}

/// Retourne `true` si le serveur a accepté la mise à jour, `false` s'il l'a refusée parce que la
/// branche distante a avancé (course avec un autre appareil).
fn push(repo: &Repository, branch: &str, token: Option<&str>) -> Result<bool> {
    let auth = AuthState::new(token);
    let mut remote = repo.find_remote(REMOTE)?;
    let rejection: RefCell<Option<String>> = RefCell::new(None);
    let mut cb = auth.callbacks();
    cb.push_update_reference(|_refname, status| {
        if let Some(msg) = status {
            *rejection.borrow_mut() = Some(msg.to_string());
        }
        Ok(())
    });
    let mut po = PushOptions::new();
    po.remote_callbacks(cb);
    po.proxy_options(proxy());
    let spec = format!("refs/heads/{branch}:refs/heads/{branch}");
    let result = remote.push(&[spec.as_str()], Some(&mut po));
    drop(po);
    if let Err(e) = result {
        // libgit2 détecte lui-même la plupart des non-fast-forward et les renvoie en erreur.
        if e.code() == git2::ErrorCode::NotFastForward || e.message().contains("not present locally") {
            return Ok(false);
        }
        return Err(auth.map_error(e));
    }
    match rejection.into_inner() {
        None => Ok(true),
        Some(msg) => {
            let m = msg.to_lowercase();
            if m.contains("non-fast-forward")
                || m.contains("fetch first")
                || m.contains("stale")
                || m.contains("fast-forward")
            {
                Ok(false)
            } else if m.contains("denied")
                || m.contains("permission")
                || m.contains("protected")
                || m.contains("forbidden")
            {
                Err(Error::AuthFailed)
            } else {
                Err(Error::Git(format!("envoi refusé par le serveur : {msg}")))
            }
        }
    }
}

fn fast_forward(repo: &Repository, branch: &str, target: Oid) -> Result<()> {
    let refname = format!("refs/heads/{branch}");
    match repo.find_reference(&refname) {
        Ok(mut r) => {
            r.set_target(target, "fast-forward")?;
        }
        Err(_) => {
            repo.reference(&refname, target, true, "fast-forward")?;
        }
    }
    repo.set_head(&refname)?;
    repo.checkout_head(Some(CheckoutBuilder::new().force()))?;
    Ok(())
}

fn merge(
    repo: &Repository,
    branch: &str,
    ours_id: Oid,
    theirs_id: Oid,
    resolver: &mut dyn ConflictResolver,
) -> Result<Vec<ResolvedConflict>> {
    let ours = repo.find_commit(ours_id)?;
    let theirs = repo.find_commit(theirs_id)?;
    let mut opts = MergeOptions::new();
    opts.find_renames(false);
    let mut index = repo.merge_commits(&ours, &theirs, Some(&opts))?;

    let mut resolved = Vec::new();
    if index.has_conflicts() {
        let conflicts: Vec<_> = index.conflicts()?.collect::<std::result::Result<_, _>>()?;
        for c in conflicts {
            let entry_path = c
                .our
                .as_ref()
                .or(c.their.as_ref())
                .or(c.ancestor.as_ref())
                .map(|e| e.path.clone())
                .ok_or_else(|| Error::Git("conflit sans chemin".into()))?;
            let path = String::from_utf8(entry_path.clone())
                .map_err(|_| Error::Conflict("nom de fichier non UTF-8 dans le dépôt".into()))?;
            if !is_committable_path(&path) {
                return Err(Error::PlaintextRefused(path));
            }
            let blob = |e: &Option<IndexEntry>| -> Result<Option<Vec<u8>>> {
                e.as_ref().map(|e| Ok(repo.find_blob(e.id)?.content().to_vec())).transpose()
            };
            let (our_bytes, their_bytes) = (blob(&c.our)?, blob(&c.their)?);
            let kept = resolver.resolve(&path, our_bytes.as_deref(), their_bytes.as_deref())?;
            let chosen = match kept {
                Side::Ours => c.our,
                Side::Theirs => c.their,
            };
            index.conflict_remove(Path::new(&path))?;
            if let Some(e) = chosen {
                index.add(&IndexEntry {
                    ctime: IndexTime::new(0, 0),
                    mtime: IndexTime::new(0, 0),
                    dev: 0,
                    ino: 0,
                    mode: e.mode,
                    uid: 0,
                    gid: 0,
                    file_size: e.file_size,
                    id: e.id,
                    flags: 0,
                    flags_extended: 0,
                    path: entry_path,
                })?;
            }
            resolved.push(ResolvedConflict { path, kept });
        }
    }
    if index.has_conflicts() {
        return Err(Error::Conflict("des conflits subsistent après arbitrage".into()));
    }
    let tree_id = index.write_tree_to(repo)?;
    let tree = repo.find_tree(tree_id)?;
    let sig = signature()?;
    repo.commit(
        Some(&format!("refs/heads/{branch}")),
        &sig,
        &sig,
        "Fusion des modifications de plusieurs appareils",
        &tree,
        &[&ours, &theirs],
    )?;
    repo.checkout_head(Some(CheckoutBuilder::new().force()))?;
    Ok(resolved)
}

/// Dernier rempart avant l'envoi : relit chaque commit sortant (de `from` jusqu'à `until` exclu).
fn verify_outgoing(repo: &Repository, from: Oid, until: Option<Oid>) -> Result<()> {
    let mut walk = repo.revwalk()?;
    walk.push(from)?;
    if let Some(u) = until {
        walk.hide(u)?;
    }
    for oid in walk {
        let commit = repo.find_commit(oid?)?;
        let tree = commit.tree()?;
        let parent_tree = commit.parent(0).ok().map(|p| p.tree()).transpose()?;
        let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        for delta in diff.deltas() {
            if delta.status() == Delta::Deleted {
                continue;
            }
            let file = delta.new_file();
            let path = file.path().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            if !is_committable_path(&path) {
                return Err(Error::PlaintextRefused(path));
            }
            if path.starts_with(&format!("{RECORDS_DIR}/")) || path.starts_with(&format!("{FILES_DIR}/")) {
                let blob = repo.find_blob(file.id())?;
                if !blob.content().starts_with(MAGIC) {
                    return Err(Error::PlaintextRefused(format!("{path} (contenu non chiffré)")));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::KdfParams;
    use crate::store::{MonthInput, MonthRecord, Status};
    use crate::vault::Vault;
    use tempfile::{tempdir, TempDir};

    const PASS: &str = "correct horse battery staple";
    const FAST: KdfParams = KdfParams::FAST_FOR_TESTS;

    struct Device {
        _dir: TempDir,
        repo: DataRepo,
    }

    impl Device {
        fn path(&self) -> &Path {
            self.repo.path()
        }
        fn vault(&self) -> Vault {
            Vault::open(self.path(), PASS).unwrap()
        }
        fn sync(&self) -> SyncReport {
            let v = self.vault();
            let mut resolver = latest_wins(&v);
            self.repo.sync(None, Some(&mut resolver)).unwrap()
        }
        fn commit(&self) {
            self.repo.commit_pending("Mise à jour du classeur", &mut Vec::new()).unwrap();
        }
    }

    /// Arbitre comme l'application : le `updated_at` le plus récent gagne ; une suppression ne
    /// l'emporte jamais sur une modification.
    fn latest_wins(v: &Vault) -> impl FnMut(&str, Option<&[u8]>, Option<&[u8]>) -> Result<Side> + '_ {
        move |path, ours, theirs| match (ours, theirs) {
            (Some(o), Some(t)) => {
                let (o, t): (MonthRecord, MonthRecord) =
                    (v.open_record_blob(path, o)?, v.open_record_blob(path, t)?);
                Ok(if t.updated_at > o.updated_at { Side::Theirs } else { Side::Ours })
            }
            (Some(_), None) => Ok(Side::Ours),
            (None, Some(_)) => Ok(Side::Theirs),
            (None, None) => Err(Error::Conflict(path.into())),
        }
    }

    /// Dépôt distant vide dont la branche par défaut est `main`, comme sur GitHub.
    fn bare() -> (TempDir, String) {
        let dir = tempdir().unwrap();
        let repo = Repository::init_bare(dir.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let url = dir.path().to_string_lossy().into_owned();
        (dir, url)
    }

    fn clone_device(url: &str) -> Device {
        let dir = tempdir().unwrap();
        let path = dir.path().join("data");
        let repo = DataRepo::clone_from(url, &path, None).unwrap();
        Device { _dir: dir, repo }
    }

    /// Premier appareil : crée le coffre dans un dépôt distant vide et le pousse.
    fn first_device(url: &str) -> Device {
        let d = clone_device(url);
        Vault::create(d.path(), PASS, FAST).unwrap();
        d.repo.write_scaffold().unwrap();
        let r = d.repo.sync(None, None).unwrap();
        assert!(r.pushed);
        d
    }

    fn month(y: i32, m: u8, cents: i64, notes: &str) -> MonthInput {
        MonthInput {
            year: y,
            month: m,
            amount_cents: Some(cents),
            declared_on: Some(format!("{y}-{m:02}-10")),
            status: Status::Declared,
            notes: notes.into(),
        }
    }

    fn all_objects_text(bare_path: &Path) -> Vec<(String, Vec<u8>)> {
        let repo = Repository::open_bare(bare_path).unwrap();
        let odb = repo.odb().unwrap();
        let mut ids = Vec::new();
        odb.foreach(|id| {
            ids.push(*id);
            true
        })
        .unwrap();
        ids.into_iter()
            .map(|id| {
                let o = odb.read(id).unwrap();
                (format!("{:?} {id}", o.kind()), o.data().to_vec())
            })
            .collect()
    }

    #[test]
    fn first_push_to_empty_remote_then_second_device_clones_and_unlocks() {
        let (bare_dir, url) = bare();
        let a = first_device(&url);
        a.vault().save_month(&month(2025, 3, 150_000, "note A")).unwrap();
        a.vault().add_attachment(2025, 3, "ar.pdf", b"%PDF-1.4 contenu").unwrap();
        let r = a.sync();
        assert!(r.pushed && !r.merged);
        assert!(!a.repo.has_unpushed().unwrap());

        let b = clone_device(&url);
        let rec = b.vault().load_month(2025, 3).unwrap().unwrap();
        assert_eq!(rec.amount_cents, Some(150_000));
        let (_, bytes) = b.vault().read_attachment(2025, 3, &rec.attachments[0].id).unwrap();
        assert_eq!(bytes.as_slice(), b"%PDF-1.4 contenu");
        drop(bare_dir);
    }

    #[test]
    fn nothing_in_clear_ever_reaches_the_remote() {
        let (bare_dir, url) = bare();
        let a = first_device(&url);
        let v = a.vault();
        v.save_month(&month(2025, 3, 987_654, "SECRETNOTE-ALPHA")).unwrap();
        v.add_attachment(2025, 3, "justificatif-SECRETNAME.pdf", b"%PDF SECRETCONTENT-BETA").unwrap();
        v.save_month(&month(2025, 3, 111_111, "SECRETNOTE-GAMMA")).unwrap(); // ancienne version dans l'historique
                                                                             // Un fichier en clair posé par erreur dans le dossier ne doit pas partir.
        fs::write(a.path().join("notes.txt"), "SECRETNOTE-DELTA").unwrap();
        fs::write(a.path().join("releve.pdf"), "SECRETCONTENT-EPSILON").unwrap();
        let report = a.sync();
        assert!(report.pushed);
        assert!(report.ignored_files.contains(&"notes.txt".to_string()));

        let needles = [
            "SECRETNOTE",
            "SECRETNAME",
            "SECRETCONTENT",
            "987654",
            "987 654",
            "111111",
            "justificatif",
            "2025",
            "declared",
            "Declared",
            "amountCents",
            "notes",
        ];
        let objects = all_objects_text(bare_dir.path());
        assert!(objects.len() > 5);
        // Garde-fou du test lui-même : le README, volontairement en clair, doit être détecté.
        assert!(objects.iter().any(|(_, d)| String::from_utf8_lossy(d).contains("Classeur URSSAF")));
        for (what, data) in &objects {
            let text = String::from_utf8_lossy(data);
            for n in needles {
                assert!(!text.contains(n), "« {n} » trouvé dans l'objet Git {what}");
            }
        }
        // Les messages de commit sont génériques.
        let repo = Repository::open_bare(bare_dir.path()).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_glob("refs/heads/*").unwrap();
        for id in walk {
            let msg = repo.find_commit(id.unwrap()).unwrap().message().unwrap().to_string();
            assert!(msg.starts_with("Mise à jour") || msg.starts_with("Fusion"), "{msg}");
        }
    }

    #[test]
    fn plaintext_forced_into_a_commit_is_refused_before_push() {
        let (bare_dir, url) = bare();
        let a = first_device(&url);
        // On contourne `commit_pending` et on commite un fichier en clair à la main.
        let repo = Repository::open(a.path()).unwrap();
        fs::write(a.path().join("plain.txt"), "SECRETNOTE-ZETA").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(Path::new("plain.txt")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let parent = repo.find_commit(head_oid(&repo).unwrap()).unwrap();
        let sig = signature().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "oops", &tree, &[&parent]).unwrap();

        let err = a.repo.sync(None, None).unwrap_err();
        assert!(matches!(err, Error::PlaintextRefused(ref p) if p == "plain.txt"), "{err:?}");
        for (what, data) in all_objects_text(bare_dir.path()) {
            assert!(!String::from_utf8_lossy(&data).contains("SECRETNOTE-ZETA"), "{what}");
        }
    }

    #[test]
    fn enc_file_that_is_not_encrypted_is_refused_before_push() {
        let (_bare_dir, url) = bare();
        let a = first_device(&url);
        let name = format!("records/{}.enc", "ab".repeat(16));
        fs::create_dir_all(a.path().join("records")).unwrap();
        fs::write(a.path().join(&name), "{\"amountCents\": 1}").unwrap();
        let err = a.repo.sync(None, None).unwrap_err();
        assert!(matches!(err, Error::PlaintextRefused(_)), "{err:?}");
    }

    #[test]
    fn fast_forward_pull_at_launch_works_without_the_key() {
        let (_b, url) = bare();
        let a = first_device(&url);
        let b = clone_device(&url);
        a.vault().save_month(&month(2025, 1, 10_000, "")).unwrap();
        a.sync();
        let report = b.repo.sync(None, None).unwrap(); // pas de coffre ouvert
        assert!(report.pulled && !report.merged && !report.deferred);
        assert_eq!(b.vault().load_all().unwrap().records.len(), 1);
    }

    #[test]
    fn divergent_edits_on_different_months_merge_cleanly() {
        let (_b, url) = bare();
        let a = first_device(&url);
        let b = clone_device(&url);
        a.vault().save_month(&month(2025, 1, 10_000, "A")).unwrap();
        b.vault().save_month(&month(2025, 2, 20_000, "B")).unwrap();
        assert!(a.sync().pushed);
        let rb = b.sync();
        assert!(rb.merged && rb.pushed && rb.resolved.is_empty());
        let ra = a.sync();
        assert!(ra.pulled);
        for d in [&a, &b] {
            let recs = d.vault().load_all().unwrap().records;
            assert_eq!(recs.iter().map(|r| r.month).collect::<Vec<_>>(), vec![1, 2]);
        }
    }

    #[test]
    fn same_month_conflict_keeps_most_recent_edit() {
        let (_b, url) = bare();
        let a = first_device(&url);
        let b = clone_device(&url);
        a.vault().save_month(&month(2025, 3, 10_000, "version A")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        b.vault().save_month(&month(2025, 3, 20_000, "version B (plus récente)")).unwrap();
        assert!(a.sync().pushed);
        let rb = b.sync();
        assert!(rb.merged && rb.pushed);
        assert_eq!(rb.resolved.len(), 1);
        assert_eq!(rb.resolved[0].kept, Side::Ours); // B est « ours » et plus récent
        a.sync();
        for d in [&a, &b] {
            let r = d.vault().load_month(2025, 3).unwrap().unwrap();
            assert_eq!(r.notes, "version B (plus récente)");
        }
    }

    #[test]
    fn delete_versus_edit_conflict_keeps_the_data() {
        let (_b, url) = bare();
        let a = first_device(&url);
        a.vault().save_month(&month(2025, 3, 10_000, "initial")).unwrap();
        a.sync();
        let b = clone_device(&url);
        a.vault().delete_month(2025, 3).unwrap();
        b.vault().save_month(&month(2025, 3, 99_999, "modifié sur B")).unwrap();
        assert!(a.sync().pushed);
        let rb = b.sync();
        assert!(rb.merged);
        a.sync();
        for d in [&a, &b] {
            assert_eq!(d.vault().load_month(2025, 3).unwrap().unwrap().notes, "modifié sur B");
        }
    }

    #[test]
    fn divergence_without_key_is_deferred_and_changes_nothing() {
        let (_b, url) = bare();
        let a = first_device(&url);
        let b = clone_device(&url);
        a.vault().save_month(&month(2025, 1, 1, "A")).unwrap();
        b.vault().save_month(&month(2025, 1, 2, "B")).unwrap();
        a.sync();
        b.commit();
        let before = head_oid(&Repository::open(b.path()).unwrap());
        let r = b.repo.sync(None, None).unwrap();
        assert!(r.deferred && !r.pushed && !r.merged);
        assert_eq!(head_oid(&Repository::open(b.path()).unwrap()), before);
        assert!(b.repo.has_unpushed().unwrap());
        // Une fois déverrouillé, la synchronisation aboutit.
        let r = b.sync();
        assert!(r.merged && r.pushed);
    }

    #[test]
    fn uncommitted_changes_are_recovered_and_pushed() {
        // Plantage entre l'écriture d'un fichier et son commit.
        let (_b, url) = bare();
        let a = first_device(&url);
        a.vault().save_month(&month(2025, 5, 5_000, "oublié")).unwrap();
        assert!(a.repo.has_unpushed().unwrap());
        let r = a.repo.sync(None, None).unwrap();
        assert!(r.pushed);
        let b = clone_device(&url);
        assert!(b.vault().load_month(2025, 5).unwrap().is_some());
    }

    #[test]
    fn sync_catches_up_with_a_remote_that_moved_since_the_last_fetch() {
        let (_b, url) = bare();
        let a = first_device(&url);
        let b = clone_device(&url);
        a.vault().save_month(&month(2025, 1, 1, "A")).unwrap();
        b.vault().save_month(&month(2025, 2, 2, "B")).unwrap();
        // B commite, A pousse, puis B (qui croit être à jour car il n'a pas refetché) synchronise :
        // le fetch interne le remet à niveau avant le push.
        b.commit();
        a.sync();
        let r = b.sync();
        assert!(r.pushed && r.merged);
    }

    #[test]
    fn no_changes_means_no_commit_and_no_push() {
        let (_b, url) = bare();
        let a = first_device(&url);
        let r = a.repo.sync(None, None).unwrap();
        assert_eq!(r, SyncReport::default());
    }

    #[test]
    fn unreachable_remote_is_a_network_error_and_local_data_is_kept() {
        let (bare_dir, url) = bare();
        let a = first_device(&url);
        a.vault().save_month(&month(2025, 1, 1, "hors ligne")).unwrap();
        drop(bare_dir); // le « serveur » disparaît
        let err = a.repo.sync(None, None).unwrap_err();
        assert!(matches!(err, Error::Network(_) | Error::Git(_)), "{err:?}");
        // La modification est validée localement malgré l'échec réseau.
        assert!(a.repo.has_unpushed().unwrap());
        assert!(a.vault().load_month(2025, 1).unwrap().is_some());
    }

    #[test]
    fn clone_adopts_the_only_branch_when_remote_head_points_elsewhere() {
        // Dépôt dont HEAD annonce « master » alors que les données sont sur « main ».
        let (bare_dir, url) = bare();
        let a = first_device(&url);
        Repository::open_bare(bare_dir.path()).unwrap().set_head("refs/heads/master").unwrap();
        let b = clone_device(&url);
        assert!(Vault::exists(b.path()));
        assert_eq!(branch_name(&Repository::open(b.path()).unwrap()).unwrap(), "main");
        drop(a);
    }

    #[test]
    fn push_rejected_as_non_fast_forward_is_reported_not_swallowed() {
        let (_b, url) = bare();
        let a = first_device(&url);
        let b = clone_device(&url);
        a.vault().save_month(&month(2025, 1, 1, "A")).unwrap();
        b.vault().save_month(&month(2025, 2, 2, "B")).unwrap();
        b.commit();
        a.sync(); // le serveur avance sans que B le sache
        let repo = Repository::open(b.path()).unwrap();
        assert!(
            !push(&repo, "main", None).unwrap(),
            "le rejet doit produire Ok(false) pour déclencher une nouvelle tentative"
        );
        // Et le serveur n'a pas reçu le commit de B.
        let remote = Repository::open(&url).unwrap();
        let remote_head = remote.find_reference("refs/heads/main").unwrap().target().unwrap();
        assert_ne!(Some(remote_head), head_oid(&repo));
    }

    /// Faux serveur HTTP qui répond toujours 401 et enregistre les en-têtes `Authorization`.
    fn serve_401() -> (String, std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
        let seen_srv = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let seen = seen_srv.clone();
                std::thread::spawn(move || {
                    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                        while let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&buf[..end]).to_string();
                            buf.drain(..end + 4);
                            let auth = head
                                .lines()
                                .find(|l| l.to_lowercase().starts_with("authorization:"))
                                .map(|l| l.split_once(':').unwrap().1.trim().to_string());
                            seen.lock().unwrap().push(auth);
                            let resp = "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"test\"\r\nContent-Length: 0\r\n\r\n";
                            if stream.write_all(resp.as_bytes()).is_err() {
                                return;
                            }
                        }
                    }
                });
            }
        });
        (format!("http://127.0.0.1:{port}/owner/data.git"), seen)
    }

    #[test]
    fn rejected_token_fails_with_auth_error_after_a_bounded_number_of_basic_auth_attempts() {
        use base64::Engine;
        let (url, seen) = serve_401();
        let dir = tempdir().unwrap();
        let err = DataRepo::clone_from(&url, &dir.path().join("d"), Some("tok_SECRET123")).unwrap_err();
        assert!(matches!(err, Error::AuthFailed), "{err:?}");
        let seen = seen.lock().unwrap();
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("x-access-token:tok_SECRET123")
        );
        let with_auth = seen.iter().flatten().filter(|a| **a == expected).count();
        assert!(
            (1..=usize::from(MAX_CREDENTIAL_OFFERS)).contains(&with_auth),
            "le jeton doit être proposé, mais de façon bornée : {seen:?}"
        );
        assert!(seen.len() <= 8, "pas de boucle d'authentification : {} requêtes", seen.len());
    }

    #[test]
    fn missing_token_is_an_auth_error_not_a_hang() {
        let (url, seen) = serve_401();
        let dir = tempdir().unwrap();
        let err = DataRepo::clone_from(&url, &dir.path().join("d"), None).unwrap_err();
        assert!(matches!(err, Error::AuthFailed), "{err:?}");
        assert!(seen.lock().unwrap().iter().all(Option::is_none));
    }

    #[test]
    fn connection_refused_is_classified_as_network_error() {
        // Port local fermé : même chemin d'erreur qu'un ordinateur hors ligne.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let dir = tempdir().unwrap();
        let err =
            DataRepo::clone_from(&format!("http://127.0.0.1:{port}/x.git"), &dir.path().join("d"), None)
                .unwrap_err();
        assert!(matches!(err, Error::Network(_)), "{err:?}");
    }

    #[test]
    fn clone_refuses_non_empty_directory() {
        let (_b, url) = bare();
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("x"), "y").unwrap();
        assert!(DataRepo::clone_from(&url, dir.path(), None).is_err());
    }
}
