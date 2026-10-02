use std::fs;
use std::io::Write;
use std::path::Path;

use crate::error::Result;

/// Écriture atomique : fichier temporaire dans le même dossier, `fsync`, puis renommage.
/// Un plantage en cours d'écriture ne laisse jamais un fichier tronqué à la place de l'ancien.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().expect("chemin sans dossier parent");
    fs::create_dir_all(dir)?;
    let name = path.file_name().expect("chemin sans nom").to_string_lossy();
    let tmp = dir.join(format!(".{name}.tmp"));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}
