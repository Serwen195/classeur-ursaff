//! Le coffre : `vault.json` (en clair, sans secret) contient le sel Argon2id, ses paramètres et la
//! clé de données aléatoire *enveloppée* (chiffrée) par la clé dérivée de la phrase secrète.
//!
//! Pourquoi une clé de données aléatoire plutôt que la clé dérivée directement ? Changer la phrase
//! secrète ne demande alors de réécrire que `vault.json`, pas tous les fichiers du classeur.

use std::fs;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::crypto::{self, Cipher, KdfParams, Keys, SecretKey};
use crate::error::{Error, Result};

pub const VAULT_FILE: &str = "vault.json";
pub const MIN_PASSPHRASE_CHARS: usize = 12;
const FORMAT: &str = "classeur-urssaf-vault";
const VERSION: u32 = 1;
const WRAP_CONTEXT: &str = "vault-key";
const SALT_LEN: usize = 16;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KdfSection {
    algorithm: String,
    #[serde(flatten)]
    params: KdfParams,
    salt: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VaultFile {
    format: String,
    version: u32,
    kdf: KdfSection,
    cipher: String,
    wrapped_key: String,
}

/// Coffre déverrouillé : détient les clés en mémoire (effacées au `Drop`).
pub struct Vault {
    pub(crate) root: PathBuf,
    data_key: SecretKey,
    pub(crate) keys: Keys,
}

impl Vault {
    pub fn exists(root: &Path) -> bool {
        root.join(VAULT_FILE).is_file()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Crée un nouveau coffre dans `root` (qui doit exister) et écrit `vault.json`.
    pub fn create(root: &Path, passphrase: &str, params: KdfParams) -> Result<Vault> {
        check_passphrase(passphrase)?;
        if Vault::exists(root) {
            return Err(Error::VaultExists);
        }
        let data_key = crypto::random_key();
        let file = wrap(&data_key, passphrase, params)?;
        write_vault_file(root, &file)?;
        Ok(Vault::from_parts(root, data_key))
    }

    /// Déverrouille le coffre. `WrongPassphrase` si la phrase est fausse.
    pub fn open(root: &Path, passphrase: &str) -> Result<Vault> {
        let file = read_vault_file(root)?;
        let data_key = unwrap(&file, passphrase)?;
        Ok(Vault::from_parts(root, data_key))
    }

    /// Change la phrase secrète : vérifie l'ancienne, ré-enveloppe la clé de données avec un
    /// nouveau sel. Seul `vault.json` est modifié ; les données ne sont pas rechiffrées.
    pub fn change_passphrase(&self, old: &str, new: &str, params: KdfParams) -> Result<()> {
        check_passphrase(new)?;
        let current = read_vault_file(&self.root)?;
        let check = unwrap(&current, old)?;
        if *check != *self.data_key {
            return Err(Error::Corrupt("la clé du coffre a changé depuis le déverrouillage".into()));
        }
        let file = wrap(&self.data_key, new, params)?;
        write_vault_file(&self.root, &file)
    }

    fn from_parts(root: &Path, data_key: SecretKey) -> Vault {
        let keys = Keys::from_data_key(&data_key);
        Vault { root: root.to_path_buf(), data_key, keys }
    }
}

fn check_passphrase(passphrase: &str) -> Result<()> {
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(Error::InvalidInput(format!(
            "La phrase secrète doit contenir au moins {MIN_PASSPHRASE_CHARS} caractères."
        )));
    }
    Ok(())
}

fn wrap(data_key: &SecretKey, passphrase: &str, params: KdfParams) -> Result<VaultFile> {
    let salt = crypto::random_bytes::<SALT_LEN>();
    let kek = crypto::derive_kek(passphrase, &salt, &params)?;
    let wrapped = Cipher::new(&kek).seal(WRAP_CONTEXT, data_key.as_slice());
    Ok(VaultFile {
        format: FORMAT.into(),
        version: VERSION,
        kdf: KdfSection { algorithm: "argon2id".into(), params, salt: B64.encode(salt) },
        cipher: "aes-256-gcm".into(),
        wrapped_key: B64.encode(wrapped),
    })
}

fn unwrap(file: &VaultFile, passphrase: &str) -> Result<SecretKey> {
    let salt =
        B64.decode(&file.kdf.salt).map_err(|_| Error::Corrupt("sel illisible dans vault.json".into()))?;
    let wrapped = B64
        .decode(&file.wrapped_key)
        .map_err(|_| Error::Corrupt("clé enveloppée illisible dans vault.json".into()))?;
    let kek = crypto::derive_kek(passphrase, &salt, &file.kdf.params)?;
    // Le blob est authentifié : un échec ici signifie, avec une quasi-certitude, une mauvaise phrase.
    let plain = Cipher::new(&kek).open(WRAP_CONTEXT, &wrapped).map_err(|_| Error::WrongPassphrase)?;
    let bytes: [u8; crypto::KEY_LEN] = plain
        .as_slice()
        .try_into()
        .map_err(|_| Error::Corrupt("clé de données de longueur inattendue".into()))?;
    Ok(Zeroizing::new(bytes))
}

fn read_vault_file(root: &Path) -> Result<VaultFile> {
    let path = root.join(VAULT_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Error::NoVault),
        Err(e) => return Err(e.into()),
    };
    let file: VaultFile =
        serde_json::from_str(&text).map_err(|e| Error::Corrupt(format!("vault.json illisible ({e})")))?;
    if file.format != FORMAT {
        return Err(Error::Corrupt("vault.json n'est pas un coffre Classeur URSSAF".into()));
    }
    if file.version != VERSION {
        return Err(Error::Corrupt(format!(
            "version de coffre {} non prise en charge : mettez l'application à jour",
            file.version
        )));
    }
    if file.kdf.algorithm != "argon2id" || file.cipher != "aes-256-gcm" {
        return Err(Error::Corrupt("algorithmes inconnus dans vault.json".into()));
    }
    Ok(file)
}

fn write_vault_file(root: &Path, file: &VaultFile) -> Result<()> {
    let mut text = serde_json::to_string_pretty(file)
        .map_err(|e| Error::Corrupt(format!("sérialisation impossible ({e})")))?;
    text.push('\n');
    crate::fsutil::atomic_write(&root.join(VAULT_FILE), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const PASS: &str = "correct horse battery staple";
    const FAST: KdfParams = KdfParams::FAST_FOR_TESTS;

    #[test]
    fn create_then_open() {
        let dir = tempdir().unwrap();
        let v1 = Vault::create(dir.path(), PASS, FAST).unwrap();
        let v2 = Vault::open(dir.path(), PASS).unwrap();
        assert_eq!(v1.keys.opaque_name("x"), v2.keys.opaque_name("x"));
        assert!(Vault::exists(dir.path()));
    }

    #[test]
    fn wrong_passphrase_is_reported_as_such() {
        let dir = tempdir().unwrap();
        Vault::create(dir.path(), PASS, FAST).unwrap();
        assert!(matches!(Vault::open(dir.path(), "another passphrase!"), Err(Error::WrongPassphrase)));
    }

    #[test]
    fn short_passphrase_refused() {
        let dir = tempdir().unwrap();
        assert!(matches!(Vault::create(dir.path(), "trop court", FAST), Err(Error::InvalidInput(_))));
        assert!(!Vault::exists(dir.path()));
    }

    #[test]
    fn cannot_overwrite_existing_vault() {
        let dir = tempdir().unwrap();
        Vault::create(dir.path(), PASS, FAST).unwrap();
        assert!(matches!(Vault::create(dir.path(), PASS, FAST), Err(Error::VaultExists)));
    }

    #[test]
    fn missing_vault() {
        let dir = tempdir().unwrap();
        assert!(matches!(Vault::open(dir.path(), PASS), Err(Error::NoVault)));
    }

    #[test]
    fn vault_json_contains_no_secret_and_is_stable_text() {
        let dir = tempdir().unwrap();
        let v = Vault::create(dir.path(), PASS, FAST).unwrap();
        let text = fs::read_to_string(dir.path().join(VAULT_FILE)).unwrap();
        assert!(text.ends_with('\n') && !text.contains('\r'));
        assert!(!text.contains(PASS));
        assert!(!text.contains(&hex::encode(*v.data_key)));
        assert!(!text.contains(&B64.encode(*v.data_key)));
        assert!(text.contains("argon2id") && text.contains("\"salt\""));
    }

    #[test]
    fn salt_is_random_per_vault() {
        let (a, b) = (tempdir().unwrap(), tempdir().unwrap());
        Vault::create(a.path(), PASS, FAST).unwrap();
        Vault::create(b.path(), PASS, FAST).unwrap();
        let fa = read_vault_file(a.path()).unwrap();
        let fb = read_vault_file(b.path()).unwrap();
        assert_ne!(fa.kdf.salt, fb.kdf.salt);
        assert_ne!(fa.wrapped_key, fb.wrapped_key);
    }

    #[test]
    fn change_passphrase_keeps_data_key() {
        let dir = tempdir().unwrap();
        let v = Vault::create(dir.path(), PASS, FAST).unwrap();
        let name = v.keys.opaque_name("probe");
        v.change_passphrase(PASS, "une toute nouvelle phrase", FAST).unwrap();
        assert!(matches!(Vault::open(dir.path(), PASS), Err(Error::WrongPassphrase)));
        let v2 = Vault::open(dir.path(), "une toute nouvelle phrase").unwrap();
        assert_eq!(v2.keys.opaque_name("probe"), name);
    }

    #[test]
    fn change_passphrase_requires_old_one() {
        let dir = tempdir().unwrap();
        let v = Vault::create(dir.path(), PASS, FAST).unwrap();
        assert!(matches!(
            v.change_passphrase("mauvaise ancienne phrase", "une toute nouvelle phrase", FAST),
            Err(Error::WrongPassphrase)
        ));
        // Le coffre reste ouvrable avec l'ancienne phrase.
        Vault::open(dir.path(), PASS).unwrap();
    }

    #[test]
    fn tampered_wrapped_key_is_rejected() {
        let dir = tempdir().unwrap();
        Vault::create(dir.path(), PASS, FAST).unwrap();
        let path = dir.path().join(VAULT_FILE);
        let mut file = read_vault_file(dir.path()).unwrap();
        let mut raw = B64.decode(&file.wrapped_key).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        file.wrapped_key = B64.encode(raw);
        fs::write(&path, serde_json::to_string(&file).unwrap()).unwrap();
        assert!(Vault::open(dir.path(), PASS).is_err());
    }

    #[test]
    fn hostile_kdf_params_do_not_allocate() {
        let dir = tempdir().unwrap();
        Vault::create(dir.path(), PASS, FAST).unwrap();
        let mut file = read_vault_file(dir.path()).unwrap();
        file.kdf.params.memory_kib = u32::MAX;
        fs::write(dir.path().join(VAULT_FILE), serde_json::to_string(&file).unwrap()).unwrap();
        assert!(matches!(Vault::open(dir.path(), PASS), Err(Error::Corrupt(_))));
    }
}
