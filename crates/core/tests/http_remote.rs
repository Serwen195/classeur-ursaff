//! Échanges Git réels en HTTP avec authentification par jeton, face à un vrai `git http-backend`
//! (pas un simulacre) : c'est le chemin qu'emprunte l'application avec GitHub.
//!
//! Ignoré par défaut car il exige `git` et `python3` :
//!   cargo test -p classeur-core --test http_remote -- --ignored

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use classeur_core::crypto::KdfParams;
use classeur_core::store::{MonthInput, Status};
use classeur_core::sync::DataRepo;
use classeur_core::{Error, Vault};
use tempfile::tempdir;

const TOKEN: &str = "github_pat_TEST-jeton-0123456789";
const PASS: &str = "correct horse battery staple";

struct Server {
    child: Child,
    port: u16,
}

impl Server {
    fn start(root: &Path) -> Server {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/git_http_server.py");
        let mut child = Command::new("python3")
            .arg(script)
            .arg(root)
            .arg(TOKEN)
            .stdout(Stdio::piped())
            .spawn()
            .expect("python3 requis");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        Server { child, port: line.trim().parse().expect("port du serveur de test") }
    }

    fn url(&self, repo: &str) -> String {
        format!("http://127.0.0.1:{}/{repo}", self.port)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn month(m: u8, cents: i64, notes: &str) -> MonthInput {
    MonthInput {
        year: 2025,
        month: m,
        amount_cents: Some(cents),
        declared_on: None,
        status: Status::Declared,
        notes: notes.into(),
    }
}

#[test]
#[ignore = "nécessite git et python3"]
fn clone_push_and_pull_over_http_with_a_token() {
    let root = tempdir().unwrap();
    let bare = root.path().join("data.git");
    let repo = git2::Repository::init_bare(&bare).unwrap();
    repo.set_head("refs/heads/main").unwrap();
    let server = Server::start(root.path());
    let url = server.url("data.git");

    // Appareil A : clone d'un dépôt vide, crée le coffre, pousse.
    let a_dir = tempdir().unwrap();
    let a = DataRepo::clone_from(&url, &a_dir.path().join("d"), Some(TOKEN)).expect("clone avec jeton");
    let a_path = a_dir.path().join("d");
    let vault = Vault::create(&a_path, PASS, KdfParams::FAST_FOR_TESTS).unwrap();
    a.write_scaffold().unwrap();
    vault.save_month(&month(3, 123_456, "secret-http")).unwrap();
    let report = a.sync(Some(TOKEN), None).expect("push avec jeton");
    assert!(report.pushed, "{report:?}");
    assert!(!a.has_unpushed().unwrap());

    // Le jeton n'a été écrit nulle part dans le dépôt local.
    let git_dir = a_path.join(".git");
    for entry in walk(&git_dir) {
        if let Ok(text) = std::fs::read_to_string(&entry) {
            assert!(!text.contains(TOKEN), "jeton retrouvé dans {}", entry.display());
        }
    }

    // Appareil B : clone, lit, modifie, pousse ; A récupère.
    let b_dir = tempdir().unwrap();
    let b_path = b_dir.path().join("d");
    let b = DataRepo::clone_from(&url, &b_path, Some(TOKEN)).unwrap();
    let vb = Vault::open(&b_path, PASS).unwrap();
    assert_eq!(vb.load_month(2025, 3).unwrap().unwrap().notes, "secret-http");
    vb.save_month(&month(4, 999, "depuis B")).unwrap();
    assert!(b.sync(Some(TOKEN), None).unwrap().pushed);

    let r = a.sync(Some(TOKEN), None).unwrap();
    assert!(r.pulled && !r.pushed, "{r:?}");
    assert_eq!(Vault::open(&a_path, PASS).unwrap().load_all().unwrap().records.len(), 2);

    // Modifications concurrentes sur le même mois : fusion arbitrée puis envoi.
    Vault::open(&a_path, PASS).unwrap().save_month(&month(5, 1, "A")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    vb.save_month(&month(5, 2, "B plus récent")).unwrap();
    assert!(a.sync(Some(TOKEN), None).unwrap().pushed);
    let vault_b = Vault::open(&b_path, PASS).unwrap();
    let mut resolver = |path: &str,
                        o: Option<&[u8]>,
                        t: Option<&[u8]>|
     -> classeur_core::Result<classeur_core::sync::Side> {
        let (o, t) =
            (vault_b.open_record_blob(path, o.unwrap())?, vault_b.open_record_blob(path, t.unwrap())?);
        Ok(if t.updated_at > o.updated_at {
            classeur_core::sync::Side::Theirs
        } else {
            classeur_core::sync::Side::Ours
        })
    };
    let r = b.sync(Some(TOKEN), Some(&mut resolver)).unwrap();
    assert!(r.merged && r.pushed, "{r:?}");
    assert_eq!(r.resolved.len(), 1);
    assert_eq!(vault_b.load_month(2025, 5).unwrap().unwrap().notes, "B plus récent");
}

#[test]
#[ignore = "nécessite git et python3"]
fn wrong_or_missing_token_is_an_auth_error() {
    let root = tempdir().unwrap();
    git2::Repository::init_bare(root.path().join("data.git")).unwrap();
    let server = Server::start(root.path());
    let url = server.url("data.git");

    for token in [None, Some("mauvais-jeton")] {
        let dir = tempdir().unwrap();
        let err = DataRepo::clone_from(&url, &dir.path().join("d"), token).unwrap_err();
        assert!(matches!(err, Error::AuthFailed), "{token:?} -> {err:?}");
    }

    // Jeton valable au clone puis révoqué : la synchronisation le dit clairement.
    let dir = tempdir().unwrap();
    let path = dir.path().join("d");
    let repo = DataRepo::clone_from(&url, &path, Some(TOKEN)).unwrap();
    Vault::create(&path, PASS, KdfParams::FAST_FOR_TESTS).unwrap();
    repo.write_scaffold().unwrap();
    let err = repo.sync(Some("jeton-revoque"), None).unwrap_err();
    assert!(matches!(err, Error::AuthFailed), "{err:?}");
    // Le coffre local n'est pas perdu pour autant.
    assert!(repo.has_unpushed().unwrap());
    assert!(repo.sync(Some(TOKEN), None).unwrap().pushed);
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
