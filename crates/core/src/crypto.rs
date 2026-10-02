//! Primitives cryptographiques.
//!
//! * Phrase secrète → clé d'enveloppe (KEK) : Argon2id, sel aléatoire.
//! * Données : AES-256-GCM, nonce aléatoire de 96 bits par chiffrement, données associées (AAD)
//!   liant chaque blob à son emplacement (un fichier copié sous un autre nom ne se déchiffre pas).
//! * Sous-clés (chiffrement / noms de fichiers) dérivées de la clé de données par HKDF-SHA256.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use crate::error::{Error, Result};

pub const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
/// En-tête de tous les blobs chiffrés : identifie le format et sa version.
pub const MAGIC: &[u8; 4] = b"CUR1";

pub type SecretKey = Zeroizing<[u8; KEY_LEN]>;

/// Paramètres Argon2id, stockés en clair dans `vault.json` (ce n'est pas un secret).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl KdfParams {
    /// Valeurs de la RFC 9106 (2ᵉ option recommandée) : 64 Mio, 3 passes, 4 voies.
    pub const DEFAULT: KdfParams = KdfParams { memory_kib: 64 * 1024, iterations: 3, parallelism: 4 };

    /// Paramètres volontairement faibles, réservés aux tests.
    pub const FAST_FOR_TESTS: KdfParams = KdfParams { memory_kib: 64, iterations: 1, parallelism: 1 };

    /// Borne haute : un dépôt altéré ne doit pas pouvoir nous faire allouer des gigaoctets.
    pub fn validate(&self) -> Result<()> {
        let ok = (8..=1024 * 1024).contains(&self.memory_kib)
            && (1..=16).contains(&self.iterations)
            && (1..=16).contains(&self.parallelism);
        if ok {
            Ok(())
        } else {
            Err(Error::Corrupt("paramètres de dérivation de clé hors limites".into()))
        }
    }
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("le générateur aléatoire du système est indisponible");
    buf
}

pub fn random_key() -> SecretKey {
    Zeroizing::new(random_bytes::<KEY_LEN>())
}

/// Normalise la phrase secrète en NFC : « é » saisi sous macOS (e + accent combinant) et sous
/// Windows (caractère précomposé) doit donner la même clé. Aucun autre traitement (pas de trim).
fn normalize(passphrase: &str) -> Zeroizing<String> {
    Zeroizing::new(passphrase.nfc().collect::<String>())
}

/// Dérive la clé d'enveloppe depuis la phrase secrète.
pub fn derive_kek(passphrase: &str, salt: &[u8], params: &KdfParams) -> Result<SecretKey> {
    params.validate()?;
    let argon_params =
        argon2::Params::new(params.memory_kib, params.iterations, params.parallelism, Some(KEY_LEN))
            .map_err(|e| Error::Corrupt(format!("paramètres Argon2 invalides ({e})")))?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, argon_params);
    let pass = normalize(passphrase);
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(pass.as_bytes(), salt, out.as_mut_slice())
        .map_err(|e| Error::Corrupt(format!("échec d'Argon2 ({e})")))?;
    Ok(out)
}

/// Chiffreur AES-256-GCM lié à une clé de 32 octets.
pub struct Cipher {
    aead: Aes256Gcm,
}

impl Cipher {
    pub fn new(key: &[u8; KEY_LEN]) -> Self {
        let aead = Aes256Gcm::new_from_slice(key).expect("clé de 32 octets");
        Cipher { aead }
    }

    /// Retourne `MAGIC ‖ nonce ‖ texte chiffré ‖ tag`.
    /// `context` identifie l'emplacement logique du blob (ex. `record:ab12…`).
    pub fn seal(&self, context: &str, plaintext: &[u8]) -> Vec<u8> {
        let nonce_bytes = random_bytes::<NONCE_LEN>();
        let nonce = Nonce::<aes_gcm::aes::cipher::consts::U12>::from(nonce_bytes);
        let aad = aad(context);
        let ct = self
            .aead
            .encrypt(&nonce, Payload { msg: plaintext, aad: &aad })
            .expect("AES-GCM : chiffrement impossible (message trop long)");
        let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ct.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        out
    }

    pub fn open(&self, context: &str, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        if blob.len() < MAGIC.len() + NONCE_LEN + TAG_LEN || &blob[..MAGIC.len()] != MAGIC {
            return Err(Error::Corrupt("format de fichier chiffré inconnu".into()));
        }
        let (nonce_bytes, ct) = blob[MAGIC.len()..].split_at(NONCE_LEN);
        let nonce = Nonce::<aes_gcm::aes::cipher::consts::U12>::try_from(nonce_bytes)
            .map_err(|_| Error::Corrupt("nonce invalide".into()))?;
        let aad = aad(context);
        self.aead.decrypt(&nonce, Payload { msg: ct, aad: &aad }).map(Zeroizing::new).map_err(|_| {
            Error::Corrupt("authentification échouée (fichier modifié ou clé différente)".into())
        })
    }
}

fn aad(context: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(MAGIC.len() + context.len());
    v.extend_from_slice(MAGIC);
    v.extend_from_slice(context.as_bytes());
    v
}

/// Jeu de clés dérivé de la clé de données : une pour chiffrer, une pour nommer les fichiers.
pub struct Keys {
    cipher: Cipher,
    name_key: SecretKey,
}

impl Keys {
    pub fn from_data_key(data_key: &[u8; KEY_LEN]) -> Self {
        let hk = Hkdf::<Sha256>::from_prk(data_key).expect("PRK de 32 octets");
        let mut enc = Zeroizing::new([0u8; KEY_LEN]);
        let mut name = Zeroizing::new([0u8; KEY_LEN]);
        hk.expand(b"classeur-urssaf/v1/enc", enc.as_mut_slice()).expect("longueur HKDF valide");
        hk.expand(b"classeur-urssaf/v1/name", name.as_mut_slice()).expect("longueur HKDF valide");
        Keys { cipher: Cipher::new(&enc), name_key: name }
    }

    pub fn seal(&self, context: &str, plaintext: &[u8]) -> Vec<u8> {
        self.cipher.seal(context, plaintext)
    }

    pub fn open(&self, context: &str, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        self.cipher.open(context, blob)
    }

    /// Nom de fichier opaque et déterministe (128 bits, hexadécimal) pour un libellé donné.
    /// Sans la clé, impossible de savoir quel mois correspond à quel fichier.
    pub fn opaque_name(&self, label: &str) -> String {
        let hk = Hkdf::<Sha256>::from_prk(self.name_key.as_slice()).expect("PRK de 32 octets");
        let mut okm = [0u8; 16];
        hk.expand(label.as_bytes(), &mut okm).expect("longueur HKDF valide");
        hex::encode(okm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Keys {
        Keys::from_data_key(&random_key())
    }

    #[test]
    fn roundtrip() {
        let k = keys();
        let blob = k.seal("record:x", b"bonjour 1234,56 EUR");
        assert_eq!(&blob[..4], MAGIC);
        assert_eq!(k.open("record:x", &blob).unwrap().as_slice(), b"bonjour 1234,56 EUR");
    }

    #[test]
    fn nonces_are_unique_so_same_plaintext_gives_different_blobs() {
        let k = keys();
        assert_ne!(k.seal("c", b"same"), k.seal("c", b"same"));
    }

    #[test]
    fn wrong_context_is_rejected() {
        let k = keys();
        let blob = k.seal("record:a", b"secret");
        assert!(matches!(k.open("record:b", &blob), Err(Error::Corrupt(_))));
    }

    #[test]
    fn wrong_key_is_rejected() {
        let blob = keys().seal("c", b"secret");
        assert!(keys().open("c", &blob).is_err());
    }

    #[test]
    fn tampering_is_detected_everywhere() {
        let k = keys();
        let blob = k.seal("c", b"secret value");
        for i in 0..blob.len() {
            let mut bad = blob.clone();
            bad[i] ^= 0x01;
            assert!(k.open("c", &bad).is_err(), "octet {i} modifié non détecté");
        }
        assert!(k.open("c", &blob[..blob.len() - 1]).is_err());
        assert!(k.open("c", b"").is_err());
    }

    #[test]
    fn opaque_names_are_deterministic_distinct_and_keyed() {
        let data_key = random_key();
        let a = Keys::from_data_key(&data_key);
        let b = Keys::from_data_key(&data_key);
        assert_eq!(a.opaque_name("record:2025-03"), b.opaque_name("record:2025-03"));
        assert_ne!(a.opaque_name("record:2025-03"), a.opaque_name("record:2025-04"));
        assert_ne!(a.opaque_name("record:2025-03"), keys().opaque_name("record:2025-03"));
        assert_eq!(a.opaque_name("x").len(), 32);
    }

    #[test]
    fn kek_depends_on_passphrase_and_salt() {
        let p = KdfParams::FAST_FOR_TESTS;
        let k1 = derive_kek("alpha bravo charlie", b"0123456789abcdef", &p).unwrap();
        let k2 = derive_kek("alpha bravo charlie", b"0123456789abcdef", &p).unwrap();
        let k3 = derive_kek("alpha bravo charlie!", b"0123456789abcdef", &p).unwrap();
        let k4 = derive_kek("alpha bravo charlie", b"fedcba9876543210", &p).unwrap();
        assert_eq!(*k1, *k2);
        assert_ne!(*k1, *k3);
        assert_ne!(*k1, *k4);
    }

    #[test]
    fn passphrase_is_unicode_normalised() {
        let p = KdfParams::FAST_FOR_TESTS;
        let precomposed = "caf\u{e9} \u{e9}t\u{e9} 2025";
        let decomposed = "cafe\u{301} e\u{301}te\u{301} 2025";
        assert_ne!(precomposed, decomposed);
        let a = derive_kek(precomposed, b"0123456789abcdef", &p).unwrap();
        let b = derive_kek(decomposed, b"0123456789abcdef", &p).unwrap();
        assert_eq!(*a, *b);
    }

    #[test]
    fn absurd_kdf_params_are_refused() {
        let bad = KdfParams { memory_kib: u32::MAX, iterations: 3, parallelism: 4 };
        assert!(derive_kek("x", b"0123456789abcdef", &bad).is_err());
    }

    /// Vecteur connu : garantit que la configuration d'Argon2id ne dérive pas silencieusement
    /// (changement de version, de variante ou d'ordre des paramètres).
    #[test]
    fn argon2id_known_answer() {
        let p = KdfParams { memory_kib: 32, iterations: 3, parallelism: 4 };
        let k = derive_kek("password", b"somesalt", &p).unwrap();
        assert_eq!(
            hex::encode(*k),
            "bb0cc80a3e671149526915418c6eefe761bb19d5d2d567a017703e0cea6ab05c",
            "vecteur Argon2id (référence : argon2-cffi)"
        );
    }
}
