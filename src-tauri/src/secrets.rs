//! Jeton d'accès GitHub : stocké uniquement dans le trousseau du système
//! (Trousseau d'accès macOS, Gestionnaire d'identifiants Windows, Secret Service sous Linux).
//! Jamais écrit dans un fichier, jamais dans l'adresse du dépôt.

use zeroize::Zeroizing;

/// Trousseau en mémoire, réservé aux tests : le trousseau réel n'existe pas sur une machine sans
/// session graphique. Le test `keychain_roundtrip_with_real_store` (ignoré par défaut) exerce le vrai.
#[cfg(test)]
pub mod testing {
    use std::sync::Mutex;
    /// `None` = vrai trousseau ; `Some(valeur)` = mode mémoire.
    pub static MEMORY: Mutex<Option<Option<String>>> = Mutex::new(None);
    pub fn enable_memory_store() {
        let mut m = MEMORY.lock().unwrap();
        if m.is_none() {
            *m = Some(None);
        }
    }
}

const SERVICE: &str = "io.github.serwen195.classeur-urssaf";
const ACCOUNT: &str = "jeton-depot-donnees";

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(describe)
}

fn describe(e: keyring::Error) -> String {
    format!(
        "Trousseau du système indisponible ({e}). Sous Linux, un service de trousseau (GNOME Keyring, KWallet) doit être installé et déverrouillé."
    )
}

pub fn store_token(token: &str) -> Result<(), String> {
    #[cfg(test)]
    if let Some(slot) = testing::MEMORY.lock().unwrap().as_mut() {
        *slot = Some(token.to_string());
        return Ok(());
    }
    let entry = entry()?;
    entry.set_password(token).map_err(describe)?;
    // Relecture : certains trousseaux acceptent l'écriture sans la conserver réellement.
    match entry.get_password() {
        Ok(back) if back == token => Ok(()),
        Ok(_) | Err(keyring::Error::NoEntry) => {
            Err("Le trousseau du système n'a pas conservé le jeton.".into())
        }
        Err(e) => Err(describe(e)),
    }
}

pub fn load_token() -> Result<Option<Zeroizing<String>>, String> {
    #[cfg(test)]
    if let Some(slot) = testing::MEMORY.lock().unwrap().as_ref() {
        return Ok(slot.clone().map(Zeroizing::new));
    }
    match entry()?.get_password() {
        Ok(t) => Ok(Some(Zeroizing::new(t))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(describe(e)),
    }
}

pub fn delete_token() -> Result<(), String> {
    #[cfg(test)]
    if let Some(slot) = testing::MEMORY.lock().unwrap().as_mut() {
        *slot = None;
        return Ok(());
    }
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(describe(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// À lancer à la main (`cargo test -p classeur-urssaf -- --ignored keychain`) sur une machine
    /// dotée d'un vrai trousseau déverrouillé : macOS, Windows, ou Linux avec un Secret Service.
    #[test]
    #[ignore = "nécessite un vrai trousseau système"]
    fn keychain_roundtrip_with_real_store() {
        delete_token().unwrap();
        assert!(load_token().unwrap().is_none());
        store_token("ghp_jeton-de-test-éè").unwrap();
        assert_eq!(load_token().unwrap().unwrap().as_str(), "ghp_jeton-de-test-éè");
        store_token("remplacé").unwrap();
        assert_eq!(load_token().unwrap().unwrap().as_str(), "remplacé");
        delete_token().unwrap();
        assert!(load_token().unwrap().is_none());
        delete_token().unwrap(); // idempotent
    }
}
