//! État partagé de l'application, erreurs destinées à l'interface et orchestration de la
//! synchronisation. Aucune logique métier ici : tout vient de `classeur-core`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use classeur_core::store::{now_ms, MonthRecord};
use classeur_core::sync::{ConflictResolver, DataRepo, Side};
use classeur_core::Vault;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::config::Config;
use crate::secrets;

// ───────────────────────── erreurs ─────────────────────────

/// Erreur renvoyée à l'interface : `code` pilote le comportement, `message` est affiché.
#[derive(Debug, Serialize, Clone)]
pub struct CmdError {
    pub code: &'static str,
    pub message: String,
}

impl CmdError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        CmdError { code, message: message.into() }
    }
    pub fn locked() -> Self {
        CmdError::new("locked", "Le classeur est verrouillé.")
    }
    pub fn other(message: impl Into<String>) -> Self {
        CmdError::new("other", message)
    }
}

impl From<classeur_core::Error> for CmdError {
    fn from(e: classeur_core::Error) -> Self {
        use classeur_core::Error as E;
        let code = match &e {
            E::WrongPassphrase => "wrong_passphrase",
            E::InvalidInput(_) => "invalid",
            E::AuthFailed => "auth",
            E::Network(_) => "network",
            E::Conflict(_) => "conflict",
            E::NoVault => "no_vault",
            E::PlaintextRefused(_) => "plaintext_refused",
            _ => "other",
        };
        CmdError { code, message: e.to_string() }
    }
}

pub type CmdResult<T> = Result<T, CmdError>;

// ───────────────────────── état ─────────────────────────

pub struct Paths {
    pub data_dir: PathBuf,
    pub config_file: PathBuf,
    /// Fichiers déchiffrés temporairement pour « Ouvrir » ; vidé au verrouillage et au démarrage.
    pub open_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    Idle,
    Syncing,
    Offline,
    AuthError,
    Error,
    /// Historiques divergents : la fusion se fera au prochain déverrouillage.
    Deferred,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConflictInfo {
    pub year: i32,
    pub month: u8,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    pub status: SyncStatus,
    pub message: Option<String>,
    pub last_sync_at: Option<i64>,
    /// Des modifications locales ne sont pas encore sur le serveur.
    pub pending: bool,
    /// Mois pour lesquels deux appareils avaient modifié la même chose (version récente gardée).
    pub conflicts: Vec<ConflictInfo>,
    pub ignored_files: Vec<String>,
}

impl SyncState {
    fn initial() -> Self {
        SyncState {
            status: SyncStatus::Idle,
            message: None,
            last_sync_at: None,
            pending: false,
            conflicts: vec![],
            ignored_files: vec![],
        }
    }
}

pub struct AppState {
    pub paths: Paths,
    pub config: Mutex<Config>,
    pub repo: Mutex<Option<Arc<DataRepo>>>,
    pub vault: Mutex<Option<Arc<Vault>>>,
    last_activity: Mutex<Instant>,
    sync_state: Mutex<SyncState>,
    sync_running: AtomicBool,
    sync_again: AtomicBool,
    /// Tenu pendant toute une passe de synchronisation : permet à « Oublier ce poste » d'attendre
    /// qu'elle soit terminée avant de supprimer le dossier de données.
    pub sync_gate: Mutex<()>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl AppState {
    pub fn new(paths: Paths, config: Config, repo: Option<DataRepo>) -> Self {
        AppState {
            paths,
            config: Mutex::new(config),
            repo: Mutex::new(repo.map(Arc::new)),
            vault: Mutex::new(None),
            last_activity: Mutex::new(Instant::now()),
            sync_state: Mutex::new(SyncState::initial()),
            sync_running: AtomicBool::new(false),
            sync_again: AtomicBool::new(false),
            sync_gate: Mutex::new(()),
        }
    }

    pub fn touch(&self) {
        *lock(&self.last_activity) = Instant::now();
    }

    pub fn config(&self) -> Config {
        lock(&self.config).clone()
    }

    pub fn repo(&self) -> CmdResult<Arc<DataRepo>> {
        lock(&self.repo)
            .clone()
            .ok_or_else(|| CmdError::new("not_configured", "Aucun dépôt de données n'est configuré."))
    }

    pub fn vault(&self) -> CmdResult<Arc<Vault>> {
        self.touch();
        lock(&self.vault).clone().ok_or_else(CmdError::locked)
    }

    pub fn set_vault(&self, v: Option<Arc<Vault>>) {
        *lock(&self.vault) = v;
    }

    pub fn set_repo(&self, r: Option<Arc<DataRepo>>) {
        *lock(&self.repo) = r;
    }

    pub fn is_unlocked(&self) -> bool {
        lock(&self.vault).is_some()
    }

    pub fn sync_state(&self) -> SyncState {
        lock(&self.sync_state).clone()
    }

    fn set_sync_state(&self, s: SyncState) {
        *lock(&self.sync_state) = s;
    }

    pub fn idle_for(&self) -> Duration {
        lock(&self.last_activity).elapsed()
    }

    /// Aucune synchronisation en cours ni en attente (utilisé par les tests pour attendre).
    #[cfg(test)]
    pub fn sync_is_quiet(&self) -> bool {
        !self.sync_running.load(Ordering::SeqCst) && !self.sync_again.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub fn pretend_idle_for(&self, d: Duration) {
        *lock(&self.last_activity) = Instant::now() - d;
    }
}

/// Exécute un travail bloquant (Argon2, Git, disque) hors du fil de l'interface.
pub async fn blocking<T, F>(f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CmdResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| CmdError::other(format!("Tâche interrompue : {e}")))?
}

// ───────────────────────── verrouillage ─────────────────────────

/// Verrouille : oublie la clé (effacée de la mémoire à la fin des opérations en cours) et supprime
/// les fichiers temporaires déchiffrés. `reason` est transmis à l'interface.
pub fn lock_vault<R: Runtime>(app: &AppHandle<R>, reason: &'static str) {
    let state = app.state::<AppState>();
    let was_unlocked = state.is_unlocked();
    state.set_vault(None);
    wipe_open_dir(&state.paths.open_dir);
    if was_unlocked {
        let _ = app.emit("locked", reason);
    }
}

pub fn wipe_open_dir(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

/// Vrai si le classeur doit se verrouiller (0 = jamais).
pub fn auto_lock_due(minutes: u32, idle: Duration) -> bool {
    minutes > 0 && idle >= Duration::from_secs(u64::from(minutes) * 60)
}

const WORKER_TICK: Duration = Duration::from_secs(5);
const PERIODIC_SYNC: Duration = Duration::from_secs(10 * 60);

/// Un pas du surveillant : verrouille après inactivité, ou lance la synchronisation périodique.
pub fn worker_tick<R: Runtime>(app: &AppHandle<R>, since_last_sync: &mut Duration) {
    let state = app.state::<AppState>();
    if !state.is_unlocked() {
        *since_last_sync = Duration::ZERO;
        return;
    }
    if auto_lock_due(state.config().auto_lock_minutes, state.idle_for()) {
        lock_vault(app, "inactivity");
        return;
    }
    *since_last_sync += WORKER_TICK;
    if *since_last_sync >= PERIODIC_SYNC {
        *since_last_sync = Duration::ZERO;
        request_sync(app);
    }
}

/// Surveille l'inactivité et déclenche une synchronisation périodique tant que le classeur est ouvert.
pub fn spawn_background_worker<R: Runtime>(app: AppHandle<R>) {
    std::thread::spawn(move || {
        let mut since_last_sync = Duration::ZERO;
        loop {
            std::thread::sleep(WORKER_TICK);
            worker_tick(&app, &mut since_last_sync);
        }
    });
}

// ───────────────────────── synchronisation ─────────────────────────

/// Demande une synchronisation. Les demandes qui arrivent pendant qu'une synchronisation tourne
/// sont fusionnées en une seule passe supplémentaire.
pub fn request_sync<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    if state.repo().is_err() {
        return;
    }
    state.sync_again.store(true, Ordering::SeqCst);
    if state.sync_running.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        loop {
            while state.sync_again.swap(false, Ordering::SeqCst) {
                run_sync_once(&app, &state);
            }
            state.sync_running.store(false, Ordering::SeqCst);
            // Une demande a pu arriver entre la dernière vérification et la libération.
            if state.sync_again.load(Ordering::SeqCst) && !state.sync_running.swap(true, Ordering::SeqCst) {
                continue;
            }
            break;
        }
    });
}

fn emit_state<R: Runtime>(app: &AppHandle<R>, state: &AppState, s: SyncState) {
    state.set_sync_state(s.clone());
    let _ = app.emit("sync-state", s);
}

fn run_sync_once<R: Runtime>(app: &AppHandle<R>, state: &AppState) {
    let _gate = lock(&state.sync_gate);
    let Ok(repo) = state.repo() else { return };
    let mut current = state.sync_state();
    current.status = SyncStatus::Syncing;
    current.message = None;
    emit_state(app, state, current.clone());

    let token = match secrets::load_token() {
        Ok(t) => t,
        Err(msg) => {
            current.status = SyncStatus::Error;
            current.message = Some(msg);
            current.pending = repo.has_unpushed().unwrap_or(true);
            emit_state(app, state, current);
            return;
        }
    };

    let vault = lock(&state.vault).clone();
    let mut conflicts: Vec<ConflictInfo> = Vec::new();
    let result = match vault.as_deref() {
        Some(v) => {
            let mut resolver = make_resolver(v, &mut conflicts);
            repo.sync(token.as_deref().map(String::as_str), Some(&mut resolver))
        }
        None => repo.sync(token.as_deref().map(String::as_str), None),
    };

    let pending = repo.has_unpushed().unwrap_or(true);
    let mut next = SyncState {
        status: SyncStatus::Idle,
        message: None,
        last_sync_at: current.last_sync_at,
        pending,
        conflicts: current.conflicts.clone(),
        ignored_files: current.ignored_files.clone(),
    };
    match result {
        Ok(report) => {
            next.ignored_files = report.ignored_files.clone();
            if report.deferred {
                next.status = SyncStatus::Deferred;
                next.message = Some(
                    "Des modifications faites sur un autre appareil seront fusionnées après déverrouillage."
                        .into(),
                );
            } else {
                next.last_sync_at = Some(now_ms());
            }
            if !conflicts.is_empty() {
                next.conflicts = conflicts;
            }
            if report.pulled && state.is_unlocked() {
                let _ = app.emit("records-changed", ());
            }
            if report.pulled && !state.is_unlocked() {
                let _ = app.emit("remote-updated", ());
            }
        }
        Err(e) => {
            let ce: CmdError = e.into();
            next.status = match ce.code {
                "network" => SyncStatus::Offline,
                "auth" => SyncStatus::AuthError,
                _ => SyncStatus::Error,
            };
            next.message = Some(match ce.code {
                "network" => {
                    "Hors ligne : vos modifications sont conservées et seront envoyées dès que possible."
                        .into()
                }
                _ => ce.message,
            });
        }
    }
    emit_state(app, state, next);
}

/// Arbitrage des conflits : la version modifiée le plus récemment l'emporte ; une suppression ne
/// l'emporte jamais sur une modification (on préfère garder une donnée en trop que la perdre).
/// L'ancienne version reste dans l'historique Git (chiffrée).
fn make_resolver<'a>(vault: &'a Vault, seen: &'a mut Vec<ConflictInfo>) -> impl ConflictResolver + 'a {
    move |path: &str, ours: Option<&[u8]>, theirs: Option<&[u8]>| -> classeur_core::Result<Side> {
        if !path.starts_with("records/") {
            return Err(classeur_core::Error::Conflict(
                "le fichier de coffre diffère entre les appareils (deux coffres créés séparément ?)".into(),
            ));
        }
        match (ours, theirs) {
            (Some(o), Some(t)) => {
                let (o, t): (MonthRecord, MonthRecord) =
                    (vault.open_record_blob(path, o)?, vault.open_record_blob(path, t)?);
                seen.push(ConflictInfo { year: o.year, month: o.month });
                Ok(if t.updated_at > o.updated_at { Side::Theirs } else { Side::Ours })
            }
            (Some(o), None) => {
                if let Ok(r) = vault.open_record_blob(path, o) {
                    seen.push(ConflictInfo { year: r.year, month: r.month });
                }
                Ok(Side::Ours)
            }
            (None, Some(t)) => {
                if let Ok(r) = vault.open_record_blob(path, t) {
                    seen.push(ConflictInfo { year: r.year, month: r.month });
                }
                Ok(Side::Theirs)
            }
            (None, None) => Err(classeur_core::Error::Conflict(path.into())),
        }
    }
}
