//! Commandes IPC appelées par l'interface. Chaque commande qui touche aux données passe par
//! `with_vault` : verrou du dossier de travail → opération → commit local → synchronisation en
//! arrière-plan. Les opérations lourdes (Argon2, Git, disque) s'exécutent hors du fil de l'interface.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use classeur_core::crypto::{random_bytes, KdfParams};
use classeur_core::store::{extension_for_mime, LoadOutcome, MonthInput, MonthRecord};
use classeur_core::sync::DataRepo;
use classeur_core::Vault;
use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use zeroize::Zeroizing;

use crate::app::{
    blocking, lock_vault, request_sync, wipe_open_dir, AppState, CmdError, CmdResult, SyncState,
};
use crate::config::MAX_AUTO_LOCK_MINUTES;
use crate::remote_url::validate_remote_url;
use crate::secrets;

const COMMIT_MESSAGE: &str = "Mise à jour du classeur";

// ───────────────────────── état général ─────────────────────────

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Pas encore de dépôt de données : assistant de première utilisation.
    NotConfigured,
    /// Dépôt cloné mais vide : il faut choisir une phrase secrète.
    NeedsVaultCreation,
    Locked,
    Unlocked,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub phase: Phase,
    pub remote_url: Option<String>,
    pub version: String,
    pub auto_lock_minutes: u32,
    pub sync: SyncState,
}

fn status<R: Runtime>(app: &AppHandle<R>, state: &AppState) -> AppStatus {
    let phase = if state.is_unlocked() {
        Phase::Unlocked
    } else {
        match state.repo() {
            Err(_) => Phase::NotConfigured,
            Ok(repo) if Vault::exists(repo.path()) => Phase::Locked,
            Ok(_) => Phase::NeedsVaultCreation,
        }
    };
    let cfg = state.config();
    AppStatus {
        phase,
        remote_url: cfg.remote_url,
        version: app.package_info().version.to_string(),
        auto_lock_minutes: cfg.auto_lock_minutes,
        sync: state.sync_state(),
    }
}

#[tauri::command]
pub fn get_status<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>) -> AppStatus {
    status(&app, &state)
}

#[tauri::command]
pub fn touch(state: State<'_, AppState>) {
    state.touch();
}

// ───────────────────────── installation ─────────────────────────

/// Étape 1 : clone le dépôt de données avec le jeton fourni, puis range le jeton dans le trousseau.
#[tauri::command]
pub async fn setup_remote<R: Runtime>(
    app: AppHandle<R>,
    url: String,
    token: Zeroizing<String>,
) -> CmdResult<AppStatus> {
    let app2 = app.clone();
    blocking(move || {
        let state = app2.state::<AppState>();
        if state.repo().is_ok() {
            return Err(CmdError::other("Un dépôt de données est déjà configuré sur ce poste."));
        }
        let url = validate_remote_url(&url).map_err(|m| CmdError::new("invalid", m))?;
        let token = Zeroizing::new(token.trim().to_string());
        if token.is_empty() {
            return Err(CmdError::new("invalid", "Le jeton d'accès GitHub est requis."));
        }
        let dir = &state.paths.data_dir;
        // Reste d'une tentative précédente interrompue : ce dossier nous appartient.
        if dir.exists() {
            std::fs::remove_dir_all(dir)
                .map_err(|e| CmdError::other(format!("Nettoyage impossible : {e}")))?;
        }
        let repo = match DataRepo::clone_from(&url, dir, Some(&token)) {
            Ok(r) => r,
            Err(e) => {
                let _ = std::fs::remove_dir_all(dir);
                return Err(e.into());
            }
        };
        if let Err(msg) = secrets::store_token(&token) {
            let _ = std::fs::remove_dir_all(dir);
            return Err(CmdError::new("keychain", msg));
        }
        {
            let mut cfg = state.config.lock().unwrap_or_else(|e| e.into_inner());
            cfg.remote_url = Some(url);
            cfg.save(&state.paths.config_file).map_err(CmdError::other)?;
        }
        state.set_repo(Some(Arc::new(repo)));
        Ok(status(&app2, &state))
    })
    .await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockResult {
    pub records: Vec<MonthRecord>,
    pub skipped: Vec<String>,
}

impl From<LoadOutcome> for UnlockResult {
    fn from(o: LoadOutcome) -> Self {
        UnlockResult { records: o.records, skipped: o.skipped }
    }
}

/// Étape 2 (première utilisation) : crée le coffre et l'envoie au dépôt. Le classeur est
/// déverrouillé directement ; l'état de synchronisation dit si l'envoi a réussi.
#[tauri::command]
pub async fn create_vault<R: Runtime>(
    app: AppHandle<R>,
    passphrase: Zeroizing<String>,
) -> CmdResult<UnlockResult> {
    let app2 = app.clone();
    let out = blocking(move || {
        let state = app2.state::<AppState>();
        let repo = state.repo()?;
        // Un autre appareil a peut-être déjà créé le coffre : on vérifie avant de créer le nôtre.
        let token = secrets::load_token().map_err(|m| CmdError::new("keychain", m))?;
        match repo.sync(token.as_deref().map(String::as_str), None) {
            Ok(_) => {}
            Err(e) => {
                let ce: CmdError = e.into();
                return Err(if ce.code == "network" {
                    CmdError::new(
                        "network",
                        "Connexion requise pour créer le coffre (il faut vérifier qu'aucun autre appareil n'en a déjà créé un).",
                    )
                } else {
                    ce
                });
            }
        }
        if Vault::exists(repo.path()) {
            return Err(CmdError::other(
                "Un coffre existe déjà dans ce dépôt : saisissez sa phrase secrète pour le déverrouiller.",
            ));
        }
        let guard = repo.lock();
        let vault = Vault::create(repo.path(), &passphrase, KdfParams::DEFAULT)?;
        repo.write_scaffold()?;
        guard.commit_pending(COMMIT_MESSAGE, &mut Vec::new())?;
        drop(guard);
        let vault = Arc::new(vault);
        let outcome = vault.load_all()?;
        state.set_vault(Some(vault));
        state.touch();
        Ok(UnlockResult::from(outcome))
    })
    .await?;
    request_sync(&app);
    Ok(out)
}

// ───────────────────────── verrouillage ─────────────────────────

#[tauri::command]
pub async fn unlock<R: Runtime>(app: AppHandle<R>, passphrase: Zeroizing<String>) -> CmdResult<UnlockResult> {
    let app2 = app.clone();
    let out = blocking(move || {
        let state = app2.state::<AppState>();
        let repo = state.repo()?;
        let vault = Arc::new(Vault::open(repo.path(), &passphrase)?);
        let guard = repo.lock();
        let outcome = vault.load_all()?;
        drop(guard);
        state.set_vault(Some(vault));
        state.touch();
        Ok(UnlockResult::from(outcome))
    })
    .await?;
    // La clé est disponible : on peut maintenant fusionner d'éventuelles divergences.
    request_sync(&app);
    Ok(out)
}

#[tauri::command]
pub fn lock<R: Runtime>(app: AppHandle<R>) {
    lock_vault(&app, "manual");
}

// ───────────────────────── données ─────────────────────────

/// Exécute `f` sur le coffre ouvert, sous le verrou du dossier de travail ; si `commit`, valide
/// ensuite les fichiers modifiés et lance la synchronisation.
async fn with_vault<R: Runtime, T, F>(app: &AppHandle<R>, commit: bool, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&Vault, &AppState) -> CmdResult<T> + Send + 'static,
{
    let app2 = app.clone();
    let out = blocking(move || {
        let state = app2.state::<AppState>();
        let vault = state.vault()?;
        let repo = state.repo()?;
        let guard = repo.lock();
        let out = f(&vault, &state)?;
        if commit {
            guard.commit_pending(COMMIT_MESSAGE, &mut Vec::new())?;
        }
        Ok(out)
    })
    .await?;
    if commit {
        request_sync(app);
    }
    Ok(out)
}

#[tauri::command]
pub async fn list_records<R: Runtime>(app: AppHandle<R>) -> CmdResult<UnlockResult> {
    with_vault(&app, false, |v, _| Ok(v.load_all()?.into())).await
}

#[tauri::command]
pub async fn save_month<R: Runtime>(app: AppHandle<R>, input: MonthInput) -> CmdResult<MonthRecord> {
    with_vault(&app, true, move |v, _| Ok(v.save_month(&input)?)).await
}

#[tauri::command]
pub async fn delete_month<R: Runtime>(app: AppHandle<R>, year: i32, month: u8) -> CmdResult<()> {
    with_vault(&app, true, move |v, _| Ok(v.delete_month(year, month)?)).await
}

#[derive(Debug, Serialize)]
pub struct AddAttachmentsResult {
    pub record: Option<MonthRecord>,
    /// Un message par fichier refusé (type non pris en charge, trop volumineux…).
    pub errors: Vec<String>,
}

/// Extensions proposées dans le sélecteur (le cœur refuse de toute façon tout le reste).
const ATTACHMENT_EXTENSIONS: &[&str] =
    &["pdf", "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif"];

async fn add_attachments_impl<R: Runtime>(
    app: &AppHandle<R>,
    year: i32,
    month: u8,
    paths: Vec<PathBuf>,
) -> CmdResult<AddAttachmentsResult> {
    with_vault(app, true, move |v, _| {
        let mut record = None;
        let mut errors = Vec::new();
        for path in paths {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match v.add_attachment_from_path(year, month, &path) {
                Ok(rec) => record = Some(rec),
                Err(e) => errors.push(format!("{name} : {e}")),
            }
        }
        Ok(AddAttachmentsResult { record, errors })
    })
    .await
}

/// Ajoute des fichiers déposés dans la fenêtre (chemins fournis par l'événement de glisser-déposer).
/// Seuls des PDF et des images sont acceptés, quel que soit le chemin.
#[tauri::command]
pub async fn add_attachments<R: Runtime>(
    app: AppHandle<R>,
    year: i32,
    month: u8,
    paths: Vec<String>,
) -> CmdResult<AddAttachmentsResult> {
    add_attachments_impl(&app, year, month, paths.into_iter().map(PathBuf::from).collect()).await
}

/// Ouvre le sélecteur de fichiers *natif* (côté Rust : la page ne choisit jamais de chemin) puis
/// ajoute les fichiers choisis. `None` si l'utilisateur annule.
#[tauri::command]
pub async fn pick_attachments<R: Runtime>(
    app: AppHandle<R>,
    year: i32,
    month: u8,
) -> CmdResult<Option<AddAttachmentsResult>> {
    app.state::<AppState>().vault()?; // refuse si verrouillé, avant d'ouvrir une boîte de dialogue
    let app2 = app.clone();
    let picked = blocking(move || {
        let files = app2
            .dialog()
            .file()
            .set_title("Ajouter des pièces jointes")
            .add_filter("PDF et images", ATTACHMENT_EXTENSIONS)
            .blocking_pick_files();
        app2.state::<AppState>().touch();
        Ok(files.map(|v| v.into_iter().filter_map(|f| f.into_path().ok()).collect::<Vec<PathBuf>>()))
    })
    .await?;
    match picked {
        Some(paths) if !paths.is_empty() => add_attachments_impl(&app, year, month, paths).await.map(Some),
        _ => Ok(None),
    }
}

#[tauri::command]
pub async fn remove_attachment<R: Runtime>(
    app: AppHandle<R>,
    year: i32,
    month: u8,
    id: String,
) -> CmdResult<MonthRecord> {
    with_vault(&app, true, move |v, _| Ok(v.remove_attachment(year, month, &id)?)).await
}

/// Octets déchiffrés d'une pièce jointe (pour l'aperçu des images, en mémoire uniquement).
#[tauri::command]
pub async fn attachment_bytes<R: Runtime>(
    app: AppHandle<R>,
    year: i32,
    month: u8,
    id: String,
) -> CmdResult<tauri::ipc::Response> {
    with_vault(&app, false, move |v, _| {
        let (_, bytes) = v.read_attachment(year, month, &id)?;
        Ok(tauri::ipc::Response::new(bytes.to_vec()))
    })
    .await
}

/// Déchiffre vers un fichier temporaire privé et l'ouvre avec l'application du système.
/// Le dossier temporaire est vidé au verrouillage, à la fermeture et au prochain démarrage.
#[tauri::command]
pub async fn open_attachment<R: Runtime>(
    app: AppHandle<R>,
    year: i32,
    month: u8,
    id: String,
) -> CmdResult<()> {
    let app2 = app.clone();
    let path = with_vault(&app, false, move |v, state| {
        let (att, bytes) = v.read_attachment(year, month, &id)?;
        write_temp_copy(&state.paths.open_dir, &att.name, &att.mime, &bytes)
    })
    .await?;
    app2.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| CmdError::other(format!("Impossible d'ouvrir le fichier : {e}")))
}

fn write_temp_copy(base: &Path, name: &str, mime: &str, bytes: &[u8]) -> CmdResult<PathBuf> {
    use std::io::Write;
    let dir = base.join(hex_id());
    std::fs::create_dir_all(&dir).map_err(|e| CmdError::other(format!("Dossier temporaire : {e}")))?;
    restrict_dir(&dir);
    // Le nom d'origine a déjà été assaini ; l'extension, elle, vient du type validé, pas du nom.
    let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = if stem.is_empty() { "piece-jointe".to_string() } else { stem };
    let path = dir.join(format!("{stem}.{}", extension_for_mime(mime)));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&path).map_err(|e| CmdError::other(format!("Fichier temporaire : {e}")))?;
    f.write_all(bytes).map_err(|e| CmdError::other(format!("Fichier temporaire : {e}")))?;
    Ok(path)
}

fn hex_id() -> String {
    hex_encode(&random_bytes::<8>())
}

fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(unix)]
fn restrict_dir(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}
#[cfg(not(unix))]
fn restrict_dir(_dir: &Path) {}

/// Enregistre une copie déchiffrée à l'endroit choisi dans la boîte « Enregistrer sous » *native*
/// (ouverte côté Rust). Renvoie le chemin écrit, ou `None` si l'utilisateur annule.
#[tauri::command]
pub async fn save_attachment_as<R: Runtime>(
    app: AppHandle<R>,
    year: i32,
    month: u8,
    id: String,
) -> CmdResult<Option<String>> {
    // 1. Déchiffrer en mémoire, sous verrou, puis relâcher le verrou avant d'attendre l'utilisateur.
    let (name, bytes) = with_vault(&app, false, move |v, _| {
        let (att, bytes) = v.read_attachment(year, month, &id)?;
        Ok((att.name, bytes))
    })
    .await?;
    // 2. Choix de la destination.
    let app2 = app.clone();
    let dest = blocking(move || {
        let chosen = app2
            .dialog()
            .file()
            .set_title("Enregistrer la pièce jointe")
            .set_file_name(name)
            .blocking_save_file();
        app2.state::<AppState>().touch();
        Ok(chosen.and_then(|f| f.into_path().ok()))
    })
    .await?;
    let Some(dest) = dest else { return Ok(None) };
    // 3. Écriture.
    blocking(move || {
        if dest.is_dir() {
            return Err(CmdError::new("invalid", "Destination invalide."));
        }
        std::fs::write(&dest, bytes.as_slice())
            .map_err(|e| CmdError::other(format!("Enregistrement impossible : {e}")))?;
        Ok(Some(dest.to_string_lossy().into_owned()))
    })
    .await
}

// ───────────────────────── synchronisation & réglages ─────────────────────────

#[tauri::command]
pub fn sync_now<R: Runtime>(app: AppHandle<R>) {
    request_sync(&app);
}

/// Remplace le jeton d'accès (expiré, révoqué…) puis relance une synchronisation.
#[tauri::command]
pub async fn set_token<R: Runtime>(app: AppHandle<R>, token: Zeroizing<String>) -> CmdResult<()> {
    blocking(move || {
        let token = Zeroizing::new(token.trim().to_string());
        if token.is_empty() {
            return Err(CmdError::new("invalid", "Le jeton d'accès GitHub est requis."));
        }
        secrets::store_token(&token).map_err(|m| CmdError::new("keychain", m))
    })
    .await?;
    request_sync(&app);
    Ok(())
}

#[tauri::command]
pub fn set_auto_lock(state: State<'_, AppState>, minutes: u32) -> CmdResult<()> {
    let mut cfg = state.config.lock().unwrap_or_else(|e| e.into_inner());
    cfg.auto_lock_minutes = minutes.min(MAX_AUTO_LOCK_MINUTES);
    cfg.save(&state.paths.config_file).map_err(CmdError::other)?;
    drop(cfg);
    state.touch();
    Ok(())
}

#[tauri::command]
pub async fn change_passphrase<R: Runtime>(
    app: AppHandle<R>,
    old_passphrase: Zeroizing<String>,
    new_passphrase: Zeroizing<String>,
) -> CmdResult<()> {
    with_vault(&app, true, move |v, _| {
        Ok(v.change_passphrase(&old_passphrase, &new_passphrase, KdfParams::DEFAULT)?)
    })
    .await
}

/// « Oublier ce poste » : supprime la copie locale, le jeton et la configuration. Le dépôt
/// distant n'est pas touché. Refuse s'il reste des modifications non envoyées, sauf `force`.
#[tauri::command]
pub async fn reset_local<R: Runtime>(app: AppHandle<R>, force: bool) -> CmdResult<AppStatus> {
    let app2 = app.clone();
    blocking(move || {
        let state = app2.state::<AppState>();
        if !force {
            if let Ok(repo) = state.repo() {
                if repo.has_unpushed().unwrap_or(false) {
                    return Err(CmdError::new(
                        "unpushed",
                        "Des modifications n'ont pas encore été envoyées au dépôt : elles seraient perdues.",
                    ));
                }
            }
        }
        lock_vault(&app2, "reset");
        // Plus aucune nouvelle synchronisation ne démarre ; on attend la fin de celle qui tourne
        // avant de supprimer le dossier, sinon elle le recréerait en y écrivant.
        state.set_repo(None);
        let _gate = state.sync_gate.lock().unwrap_or_else(|e| e.into_inner());
        let _ = std::fs::remove_dir_all(&state.paths.data_dir);
        wipe_open_dir(&state.paths.open_dir);
        let _ = secrets::delete_token();
        {
            let mut cfg = state.config.lock().unwrap_or_else(|e| e.into_inner());
            cfg.remote_url = None;
            cfg.save(&state.paths.config_file).map_err(CmdError::other)?;
        }
        Ok(status(&app2, &state))
    })
    .await
}
