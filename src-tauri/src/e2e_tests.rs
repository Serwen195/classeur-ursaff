//! Tests d'intégration de la coque : les vraies commandes IPC, le vrai état partagé, de vraies
//! synchronisations en arrière-plan, sur de vrais dépôts Git locaux (un « dépôt distant » nu et
//! deux « appareils » qui le clonent). Seuls le trousseau (mémoire) et la fenêtre (runtime de
//! test de Tauri) sont simulés.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use classeur_core::sync::DataRepo;
use git2::Repository;
use serde_json::{json, Value};
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{Manager, WebviewWindow};
use tempfile::{tempdir, TempDir};

use crate::app::{worker_tick, AppState, Paths};
use crate::config::Config;
use crate::secrets;

const PASS: &str = "correct horse battery staple";
/// Origine « locale » de la webview : `tauri://localhost` (Linux, macOS) ou `http://tauri.localhost` (Windows).
const LOCAL_ORIGIN: &str = if cfg!(windows) { "http://tauri.localhost" } else { "tauri://localhost" };

struct Remote {
    _dir: TempDir,
    url: String,
}

impl Remote {
    fn new() -> Remote {
        let dir = tempdir().unwrap();
        let repo = Repository::init_bare(dir.path()).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        let url = dir.path().to_string_lossy().into_owned();
        Remote { _dir: dir, url }
    }

    fn path(&self) -> &Path {
        self._dir.path()
    }

    /// Tous les objets Git du dépôt distant : (type, octets).
    fn objects(&self) -> Vec<(String, Vec<u8>)> {
        let repo = Repository::open_bare(self.path()).unwrap();
        let odb = repo.odb().unwrap();
        let mut ids = vec![];
        odb.foreach(|id| {
            ids.push(*id);
            true
        })
        .unwrap();
        ids.into_iter()
            .map(|id| {
                let o = odb.read(id).unwrap();
                (format!("{:?}", o.kind()), o.data().to_vec())
            })
            .collect()
    }

    fn head_commit_count(&self) -> usize {
        let repo = Repository::open_bare(self.path()).unwrap();
        let Ok(head) = repo.find_reference("refs/heads/main") else { return 0 };
        let mut walk = repo.revwalk().unwrap();
        walk.push(head.target().unwrap()).unwrap();
        walk.count()
    }
}

struct Device {
    _tmp: TempDir,
    app: tauri::App<MockRuntime>,
    window: WebviewWindow<MockRuntime>,
    tmp_path: std::path::PathBuf,
}

fn device(remote_url: &str) -> Device {
    secrets::testing::enable_memory_store();
    let tmp = tempdir().unwrap();
    let paths = Paths {
        data_dir: tmp.path().join("data"),
        config_file: tmp.path().join("config").join("config.json"),
        open_dir: tmp.path().join("open"),
    };
    let repo = DataRepo::clone_from(remote_url, &paths.data_dir, None).unwrap();
    let app =
        mock_builder().invoke_handler(crate::app_handlers!()).build(mock_context(noop_assets())).unwrap();
    app.manage(AppState::new(
        paths,
        Config { remote_url: Some(remote_url.to_string()), ..Config::default() },
        Some(repo),
    ));
    let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    let tmp_path = tmp.path().to_path_buf();
    Device { _tmp: tmp, app, window, tmp_path }
}

impl Device {
    fn raw(&self, cmd: &str, body: Value) -> Result<InvokeResponseBody, Value> {
        get_ipc_response(
            &self.window,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: LOCAL_ORIGIN.parse().unwrap(),
                body: InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
    }

    fn call(&self, cmd: &str, body: Value) -> Result<Value, Value> {
        match self.raw(cmd, body)? {
            InvokeResponseBody::Json(s) => Ok(serde_json::from_str(&s).unwrap()),
            InvokeResponseBody::Raw(b) => Ok(json!({ "rawLen": b.len() })),
        }
    }

    fn ok(&self, cmd: &str, body: Value) -> Value {
        self.call(cmd, body).unwrap_or_else(|e| panic!("{cmd} a échoué : {e}"))
    }

    fn err_code(&self, cmd: &str, body: Value) -> String {
        match self.call(cmd, body) {
            Err(e) => e["code"].as_str().unwrap_or("?").to_string(),
            Ok(v) => panic!("{cmd} aurait dû échouer, a renvoyé {v}"),
        }
    }

    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }

    /// Attend la fin des synchronisations en arrière-plan.
    fn wait_quiet(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            // Laisse le temps à une synchronisation fraîchement demandée de démarrer.
            std::thread::sleep(Duration::from_millis(80));
            if self.state().sync_is_quiet() {
                std::thread::sleep(Duration::from_millis(80));
                if self.state().sync_is_quiet() {
                    return;
                }
            }
            assert!(Instant::now() < deadline, "la synchronisation ne se termine pas");
        }
    }

    fn sync_and_wait(&self) {
        self.ok("sync_now", json!({}));
        self.wait_quiet();
    }

    fn phase(&self) -> String {
        self.ok("get_status", json!({}))["phase"].as_str().unwrap().to_string()
    }

    fn unlock(&self) -> Value {
        self.ok("unlock", json!({ "passphrase": PASS }))
    }

    fn month(&self, year: i32, month: u8, cents: i64, status: &str, notes: &str) -> Value {
        self.ok(
            "save_month",
            json!({ "input": {
                "year": year, "month": month, "amountCents": cents,
                "declaredOn": format!("{year}-{month:02}-10"), "status": status, "notes": notes
            }}),
        )
    }

    fn records(&self) -> Vec<Value> {
        self.ok("list_records", json!({}))["records"].as_array().unwrap().clone()
    }
}

/// Premier appareil : dépôt vide → coffre créé et envoyé.
fn first_device(remote: &Remote) -> Device {
    let a = device(&remote.url);
    assert_eq!(a.phase(), "needs_vault_creation");
    a.ok("create_vault", json!({ "passphrase": PASS }));
    a.wait_quiet();
    a
}

#[test]
fn full_lifecycle_through_the_real_commands() {
    let remote = Remote::new();
    let a = device(&remote.url);
    assert_eq!(a.phase(), "needs_vault_creation");

    // Phrase trop courte : refusée, rien n'est créé.
    assert_eq!(a.err_code("create_vault", json!({ "passphrase": "court" })), "invalid");
    assert_eq!(a.phase(), "needs_vault_creation");

    let created = a.ok("create_vault", json!({ "passphrase": PASS }));
    assert_eq!(created["records"], json!([]));
    assert_eq!(a.phase(), "unlocked");
    a.wait_quiet();
    assert!(remote.head_commit_count() >= 1, "le coffre doit être envoyé au dépôt distant");
    assert!(!a.state().sync_state().pending);

    let saved = a.month(2025, 3, 123_456, "declared", "NOTE-ULTRA-SECRETE");
    assert_eq!(saved["amountCents"], 123_456);

    // Pièce jointe depuis un vrai fichier.
    let pdf = a.tmp_path.join("accuse.pdf");
    fs::write(&pdf, b"%PDF-1.4 CONTENU-PDF-ULTRA-SECRET").unwrap();
    let exe = a.tmp_path.join("virus.exe");
    fs::write(&exe, b"MZ").unwrap();
    let added = a.ok(
        "add_attachments",
        json!({ "year": 2025, "month": 3, "paths": [pdf.to_string_lossy(), exe.to_string_lossy()] }),
    );
    assert_eq!(added["record"]["attachments"].as_array().unwrap().len(), 1);
    assert_eq!(added["errors"].as_array().unwrap().len(), 1, "l'exécutable est refusé, le PDF accepté");
    let att_id = added["record"]["attachments"][0]["id"].as_str().unwrap().to_string();

    a.wait_quiet();
    assert!(!a.state().sync_state().pending, "tout est envoyé : {:?}", a.state().sync_state());

    // Rien de lisible côté serveur, ni dans les objets ni dans les messages de commit.
    let needles = ["ULTRA-SECRET", "123456", "accuse", "2025", "declared", "amountCents", "Mars"];
    let objects = remote.objects();
    assert!(objects.len() >= 6);
    for (kind, data) in &objects {
        let text = String::from_utf8_lossy(data);
        for n in needles {
            assert!(!text.contains(n), "« {n} » lisible dans un objet {kind} du dépôt distant");
        }
    }

    // Aperçu de la pièce jointe (l'export passe par une boîte de dialogue native, non testable ici).
    let raw = a.raw("attachment_bytes", json!({ "year": 2025, "month": 3, "id": att_id })).unwrap();
    assert!(matches!(raw, InvokeResponseBody::Raw(ref b) if b == b"%PDF-1.4 CONTENU-PDF-ULTRA-SECRET"));

    // Verrouillage : plus aucun accès aux données.
    let open_dir = a.state().paths.open_dir.clone();
    fs::create_dir_all(open_dir.join("x")).unwrap();
    fs::write(open_dir.join("x").join("copie-dechiffree.pdf"), b"clair").unwrap();
    a.ok("lock", json!({}));
    assert_eq!(a.phase(), "locked");
    assert!(!open_dir.exists(), "les copies déchiffrées temporaires doivent disparaître au verrouillage");
    for cmd in ["list_records", "sync_now_but_needs_vault"] {
        if cmd == "list_records" {
            assert_eq!(a.err_code(cmd, json!({})), "locked");
        }
    }
    assert_eq!(
        a.err_code(
            "save_month",
            json!({ "input": {
        "year": 2025, "month": 4, "amountCents": 1, "declaredOn": null, "status": "paid", "notes": "" } })
        ),
        "locked"
    );
    assert_eq!(a.err_code("attachment_bytes", json!({ "year": 2025, "month": 3, "id": att_id })), "locked");

    // Mauvaise phrase, puis bonne phrase : les données sont intactes.
    assert_eq!(
        a.err_code("unlock", json!({ "passphrase": "pas la bonne phrase du tout" })),
        "wrong_passphrase"
    );
    assert_eq!(a.phase(), "locked");
    let unlocked = a.unlock();
    assert_eq!(unlocked["records"][0]["notes"], "NOTE-ULTRA-SECRETE");
    assert_eq!(unlocked["records"][0]["attachments"][0]["name"], "accuse.pdf");
}

#[test]
fn second_device_gets_the_data_and_changes_flow_back() {
    let remote = Remote::new();
    let a = first_device(&remote);
    a.month(2025, 3, 100_000, "paid", "depuis A");
    a.wait_quiet();

    let b = device(&remote.url);
    assert_eq!(
        b.phase(),
        "locked",
        "le coffre existe déjà : on demande la phrase, on n'en crée pas un autre"
    );
    assert_eq!(b.err_code("create_vault", json!({ "passphrase": PASS })), "other");
    let u = b.unlock();
    assert_eq!(u["records"][0]["notes"], "depuis A");

    b.month(2025, 4, 250_000, "declared", "depuis B");
    b.wait_quiet();

    a.sync_and_wait();
    let notes: Vec<_> = a.records().iter().map(|r| r["notes"].as_str().unwrap().to_string()).collect();
    assert_eq!(notes, vec!["depuis A", "depuis B"]);
}

#[test]
fn launch_pull_without_the_passphrase_then_unlock_sees_new_data() {
    let remote = Remote::new();
    let a = first_device(&remote);
    let b = device(&remote.url); // verrouillé
    a.month(2025, 1, 5_000, "paid", "nouveau");
    a.wait_quiet();

    // « Lancement » de B : récupération sans clé (fast-forward seulement).
    b.sync_and_wait();
    assert_eq!(b.phase(), "locked");
    let u = b.unlock();
    assert_eq!(u["records"].as_array().unwrap().len(), 1);
}

#[test]
fn concurrent_edits_of_the_same_month_converge_on_the_most_recent() {
    let remote = Remote::new();
    let a = first_device(&remote);
    a.month(2025, 3, 1, "to_declare", "version initiale");
    a.wait_quiet();
    let b = device(&remote.url);
    b.unlock();

    // Les deux appareils modifient le même mois sans s'être synchronisés.
    a.month(2025, 3, 111, "declared", "écrit par A");
    std::thread::sleep(Duration::from_millis(15));
    b.month(2025, 3, 222, "paid", "écrit par B (plus récent)");
    a.wait_quiet();
    b.wait_quiet();
    a.sync_and_wait();
    b.sync_and_wait();
    a.sync_and_wait();

    let (ra, rb) = (a.records(), b.records());
    assert_eq!(ra, rb, "les deux appareils doivent converger");
    assert_eq!(ra.len(), 1);
    assert_eq!(ra[0]["notes"], "écrit par B (plus récent)");
    assert_eq!(ra[0]["amountCents"], 222);
    assert!(!a.state().sync_state().pending && !b.state().sync_state().pending);
}

#[test]
fn deletion_never_beats_a_concurrent_edit() {
    let remote = Remote::new();
    let a = first_device(&remote);
    a.month(2025, 3, 1, "declared", "à supprimer");
    a.wait_quiet();
    let b = device(&remote.url);
    b.unlock();

    a.ok("delete_month", json!({ "year": 2025, "month": 3 }));
    b.month(2025, 3, 999, "paid", "modifié pendant ce temps");
    a.wait_quiet();
    b.wait_quiet();
    a.sync_and_wait();
    b.sync_and_wait();
    a.sync_and_wait();

    assert_eq!(a.records(), b.records());
    assert_eq!(a.records()[0]["notes"], "modifié pendant ce temps");
}

#[test]
fn auto_lock_after_inactivity_and_never_when_disabled() {
    let remote = Remote::new();
    let a = first_device(&remote);
    let handle = a.app.handle().clone();
    let mut since = Duration::ZERO;

    a.ok("set_auto_lock", json!({ "minutes": 5 }));
    a.state().pretend_idle_for(Duration::from_secs(4 * 60));
    worker_tick(&handle, &mut since);
    assert_eq!(a.phase(), "unlocked", "pas encore 5 minutes d'inactivité");

    a.state().pretend_idle_for(Duration::from_secs(5 * 60 + 1));
    worker_tick(&handle, &mut since);
    assert_eq!(a.phase(), "locked");
    assert_eq!(a.err_code("list_records", json!({})), "locked");

    a.unlock();
    a.ok("set_auto_lock", json!({ "minutes": 0 }));
    a.state().pretend_idle_for(Duration::from_secs(48 * 3600));
    worker_tick(&handle, &mut since);
    assert_eq!(a.phase(), "unlocked", "0 = jamais de verrouillage automatique");

    // L'activité de l'utilisateur repousse le verrouillage.
    a.ok("set_auto_lock", json!({ "minutes": 1 }));
    a.state().pretend_idle_for(Duration::from_secs(59));
    a.ok("touch", json!({}));
    worker_tick(&handle, &mut since);
    assert_eq!(a.phase(), "unlocked");
}

#[test]
fn auto_lock_setting_is_persisted_and_clamped() {
    let remote = Remote::new();
    let a = first_device(&remote);
    a.ok("set_auto_lock", json!({ "minutes": 999_999 }));
    let cfg_path = a.state().paths.config_file.clone();
    let saved = Config::load(&cfg_path);
    assert_eq!(saved.auto_lock_minutes, crate::config::MAX_AUTO_LOCK_MINUTES);
    assert_eq!(saved.remote_url.as_deref(), Some(remote.url.as_str()));
    let text = fs::read_to_string(cfg_path).unwrap();
    assert!(!text.contains(PASS));
}

#[test]
fn reset_local_forgets_the_device_but_protects_unsent_changes() {
    let remote = Remote::new();
    let a = first_device(&remote);
    a.month(2025, 1, 1, "paid", "x");
    a.wait_quiet();

    // Rien en attente : l'oubli passe directement.
    let b = device(&remote.url);
    b.unlock();
    let status = b.ok("reset_local", json!({ "force": false }));
    assert_eq!(status["phase"], "not_configured");
    assert!(!b.state().paths.data_dir.exists());
    assert!(secrets::load_token().unwrap().is_none());
    assert_eq!(Config::load(&b.state().paths.config_file).remote_url, None);
    assert_eq!(b.err_code("list_records", json!({})), "locked");

    // Modification locale impossible à envoyer (serveur indisponible) : on prévient avant de perdre.
    let remote2 = Remote::new();
    let c = first_device(&remote2);
    let url2 = remote2.path().to_path_buf();
    drop(remote2); // supprime le dépôt distant
    assert!(!url2.exists());
    c.month(2025, 2, 2, "paid", "jamais envoyé");
    c.wait_quiet();
    assert!(c.state().sync_state().pending);
    assert_eq!(c.err_code("reset_local", json!({ "force": false })), "unpushed");
    assert!(c.state().paths.data_dir.exists(), "refus = rien n'est supprimé");
    let status = c.ok("reset_local", json!({ "force": true }));
    assert_eq!(status["phase"], "not_configured");
}

#[test]
fn working_offline_keeps_changes_locally_and_flags_them_as_pending() {
    let remote = Remote::new();
    let a = first_device(&remote);
    let url = remote.path().to_path_buf();
    drop(remote);
    assert!(!url.exists());

    a.month(2025, 5, 4_200, "declared", "hors ligne");
    a.wait_quiet();
    let s = a.state().sync_state();
    assert!(s.pending, "la modification est en attente d'envoi");
    assert!(s.status != crate::app::SyncStatus::Idle);
    assert_eq!(a.records()[0]["notes"], "hors ligne", "mais elle est bien enregistrée localement");
}

#[test]
fn changing_the_passphrase_works_across_devices() {
    let remote = Remote::new();
    let a = first_device(&remote);
    a.month(2025, 1, 777, "paid", "avant");
    a.wait_quiet();

    assert_eq!(
        a.err_code("change_passphrase", json!({ "oldPassphrase": "mauvaise ancienne phrase", "newPassphrase": "une nouvelle phrase longue" })),
        "wrong_passphrase"
    );
    assert_eq!(
        a.err_code("change_passphrase", json!({ "oldPassphrase": PASS, "newPassphrase": "court" })),
        "invalid"
    );
    a.ok(
        "change_passphrase",
        json!({ "oldPassphrase": PASS, "newPassphrase": "une nouvelle phrase longue" }),
    );
    a.wait_quiet();

    // Un autre appareil, après synchronisation, n'accepte plus l'ancienne phrase.
    let b = device(&remote.url);
    assert_eq!(b.err_code("unlock", json!({ "passphrase": PASS })), "wrong_passphrase");
    let u = b.ok("unlock", json!({ "passphrase": "une nouvelle phrase longue" }));
    assert_eq!(u["records"][0]["notes"], "avant");
}

#[test]
fn setup_rejects_bad_urls_and_cleans_up_after_a_failed_clone() {
    let remote = Remote::new();
    let a = device(&remote.url);
    // Appareil non configuré : on repart d'un état vide.
    a.ok("reset_local", json!({ "force": true }));

    assert_eq!(a.phase(), "not_configured");
    for bad in ["http://github.com/a/b", "git@github.com:a/b.git", "https://tok@github.com/a/b", "/tmp/depot"]
    {
        assert_eq!(a.err_code("setup_remote", json!({ "url": bad, "token": "t" })), "invalid", "{bad}");
    }
    assert_eq!(
        a.err_code("setup_remote", json!({ "url": "https://github.com/a/b", "token": "  " })),
        "invalid"
    );

    // Port fermé : échec réseau, et rien ne subsiste (ni dossier, ni configuration).
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let code = a.err_code(
        "setup_remote",
        json!({ "url": format!("https://127.0.0.1:{port}/a/b"), "token": "jeton" }),
    );
    assert_eq!(code, "network");
    assert_eq!(a.phase(), "not_configured");
    assert!(!a.state().paths.data_dir.exists());
    assert_eq!(Config::load(&a.state().paths.config_file).remote_url, None);
}

#[test]
fn unrecognised_files_in_the_data_folder_are_never_pushed() {
    let remote = Remote::new();
    let a = first_device(&remote);
    fs::write(a.state().paths.data_dir.join("notes-perso.txt"), "SECRET-EN-CLAIR").unwrap();
    a.month(2025, 6, 1, "paid", "x");
    a.wait_quiet();
    for (kind, data) in remote.objects() {
        assert!(!String::from_utf8_lossy(&data).contains("SECRET-EN-CLAIR"), "fuite dans un objet {kind}");
    }
    assert!(a.state().sync_state().ignored_files.contains(&"notes-perso.txt".to_string()));
}

#[test]
fn many_rapid_saves_all_end_up_on_the_server() {
    // Les demandes de synchronisation arrivent en rafale : elles doivent être regroupées
    // sans jamais perdre une modification.
    let remote = Remote::new();
    let a = first_device(&remote);
    for m in 1..=12u8 {
        a.month(2025, m, i64::from(m) * 100, "declared", &format!("mois {m}"));
    }
    a.wait_quiet();
    assert!(!a.state().sync_state().pending);

    let b = device(&remote.url);
    let u = b.unlock();
    assert_eq!(u["records"].as_array().unwrap().len(), 12);
    let total: i64 =
        u["records"].as_array().unwrap().iter().map(|r| r["amountCents"].as_i64().unwrap()).sum();
    assert_eq!(total, (1..=12).map(|m| m * 100).sum::<i64>());
}

/// Le contrat JSON partagé avec l'interface (`src/lib/contract.json`, vérifié côté TypeScript par
/// `src/lib/contract.test.ts`) doit correspondre à la sérialisation réelle.
#[test]
fn json_shapes_match_the_shared_contract() {
    let contract: Value = serde_json::from_str(include_str!("../../src/lib/contract.json")).unwrap();
    let keys = |v: &Value| -> Vec<String> {
        let mut k: Vec<String> = v.as_object().expect("objet JSON attendu").keys().cloned().collect();
        k.sort();
        k
    };
    let expected = |name: &str| -> Vec<String> {
        let mut k: Vec<String> = contract["objects"][name]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect();
        k.sort();
        k
    };

    let remote = Remote::new();
    let a = first_device(&remote);
    let pdf = a.tmp_path.join("a.pdf");
    fs::write(&pdf, b"%PDF-1").unwrap();
    a.month(2025, 3, 100, "declared", "n");
    let added = a.ok(
        "add_attachments",
        json!({ "year": 2025, "month": 3, "paths": [pdf.to_string_lossy(), "/n/existe/pas.exe"] }),
    );
    a.wait_quiet();

    let status = a.ok("get_status", json!({}));
    assert_eq!(keys(&status), expected("AppStatus"));
    assert_eq!(keys(&status["sync"]), expected("SyncState"));
    let list = a.ok("list_records", json!({}));
    assert_eq!(keys(&list), expected("UnlockResult"));
    assert_eq!(keys(&list["records"][0]), expected("MonthRecord"));
    assert_eq!(keys(&list["records"][0]["attachments"][0]), expected("Attachment"));
    assert_eq!(keys(&added), expected("AddAttachmentsResult"));

    // Erreur renvoyée à l'interface : { code, message }.
    let err = a
        .call(
            "save_month",
            json!({ "input": { "year": 1, "month": 1, "amountCents": null,
        "declaredOn": null, "status": "paid", "notes": "" } }),
        )
        .unwrap_err();
    assert_eq!(keys(&err), vec!["code".to_string(), "message".to_string()]);

    // Conflits : { year, month }.
    let c = serde_json::to_value(crate::app::ConflictInfo { year: 2025, month: 3 }).unwrap();
    assert_eq!(keys(&c), expected("ConflictInfo"));

    // Valeurs des énumérations.
    let enum_values = |name: &str| -> Vec<String> {
        let mut v: Vec<String> = contract["enums"][name]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect();
        v.sort();
        v
    };
    let ser = |v: Value| v.as_str().unwrap().to_string();
    use crate::app::SyncStatus as S;
    use crate::commands::Phase as P;
    use classeur_core::store::Status as St;
    let mut sync_status: Vec<String> = [S::Idle, S::Syncing, S::Offline, S::AuthError, S::Error, S::Deferred]
        .iter()
        .map(|x| ser(serde_json::to_value(x).unwrap()))
        .collect();
    sync_status.sort();
    assert_eq!(sync_status, enum_values("SyncStatus"));
    let mut phases: Vec<String> = [P::NotConfigured, P::NeedsVaultCreation, P::Locked, P::Unlocked]
        .iter()
        .map(|x| ser(serde_json::to_value(x).unwrap()))
        .collect();
    phases.sort();
    assert_eq!(phases, enum_values("Phase"));
    let mut statuses: Vec<String> = [St::ToDeclare, St::Declared, St::Paid]
        .iter()
        .map(|x| ser(serde_json::to_value(x).unwrap()))
        .collect();
    statuses.sort();
    assert_eq!(statuses, enum_values("Status"));
}
