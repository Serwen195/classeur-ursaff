//! Réglages non secrets (adresse du dépôt, délai de verrouillage). Le jeton n'est PAS ici.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const DEFAULT_AUTO_LOCK_MINUTES: u32 = 10;
pub const MAX_AUTO_LOCK_MINUTES: u32 = 480;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub remote_url: Option<String>,
    /// 0 = ne jamais verrouiller automatiquement.
    pub auto_lock_minutes: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config { remote_url: None, auto_lock_minutes: DEFAULT_AUTO_LOCK_MINUTES }
    }
}

impl Config {
    pub fn load(path: &Path) -> Config {
        let mut cfg: Config =
            fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        cfg.auto_lock_minutes = cfg.auto_lock_minutes.min(MAX_AUTO_LOCK_MINUTES);
        cfg
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        classeur_core::fsutil::atomic_write(path, text.as_bytes()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_corrupt_file_gives_defaults() {
        let dir = std::env::temp_dir().join(format!("cu-cfg-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(Config::load(&dir.join("absent.json")), Config::default());
        fs::write(dir.join("bad.json"), "{ pas du json").unwrap();
        assert_eq!(Config::load(&dir.join("bad.json")), Config::default());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn roundtrip_and_clamp_and_never_stores_a_token() {
        let dir = std::env::temp_dir().join(format!("cu-cfg2-{}", std::process::id()));
        let path = dir.join("config.json");
        let cfg = Config { remote_url: Some("https://github.com/a/b".into()), auto_lock_minutes: 99_999 };
        cfg.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.to_lowercase().contains("token") && !text.to_lowercase().contains("jeton"));
        let back = Config::load(&path);
        assert_eq!(back.remote_url.as_deref(), Some("https://github.com/a/b"));
        assert_eq!(back.auto_lock_minutes, MAX_AUTO_LOCK_MINUTES);
        fs::remove_dir_all(&dir).unwrap();
    }
}
