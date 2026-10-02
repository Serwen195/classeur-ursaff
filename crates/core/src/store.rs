//! Enregistrements mensuels et pièces jointes, chiffrés fichier par fichier.
//!
//! Disposition du dossier de données (rien d'autre n'est jamais écrit) :
//!
//! ```text
//! vault.json                  sel Argon2id + clé de données enveloppée (aucun secret en clair)
//! records/<32 hex>.enc        un fichier par mois ; le nom est un HKDF opaque de « année-mois »
//! files/<32 hex>.enc          une pièce jointe chiffrée ; le nom est un identifiant aléatoire
//! ```
//!
//! Les montants, dates, statuts, notes, noms et types des pièces jointes vivent *dans* le blob
//! chiffré du mois. Seules fuient la taille des fichiers, leur nombre et les dates de commit.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::crypto::random_bytes;
use crate::error::{Error, Result};
use crate::fsutil::atomic_write;
use crate::vault::{Vault, VAULT_FILE};

pub const RECORDS_DIR: &str = "records";
pub const FILES_DIR: &str = "files";
pub const MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024;
pub const MIN_YEAR: i32 = 2000;
pub const MAX_YEAR: i32 = 2100;
const MAX_NOTES_CHARS: usize = 10_000;
const MAX_AMOUNT_CENTS: i64 = 100_000_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    ToDeclare,
    Declared,
    Paid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    /// 32 caractères hexadécimaux aléatoires ; sert de nom de fichier dans `files/`.
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub added_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthRecord {
    pub year: i32,
    pub month: u8,
    /// Montant en centimes d'euro (entier : pas d'erreur d'arrondi dans les totaux).
    pub amount_cents: Option<i64>,
    /// Date de déclaration au format `AAAA-MM-JJ`.
    pub declared_on: Option<String>,
    pub status: Status,
    pub notes: String,
    pub attachments: Vec<Attachment>,
    /// Millisecondes depuis l'epoch Unix ; sert à départager deux modifications concurrentes.
    pub updated_at: i64,
}

/// Ce que l'utilisateur peut modifier d'un mois. Les pièces jointes passent par leurs propres
/// opérations : l'interface ne peut donc pas écraser la liste par erreur.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthInput {
    pub year: i32,
    pub month: u8,
    pub amount_cents: Option<i64>,
    pub declared_on: Option<String>,
    pub status: Status,
    pub notes: String,
}

#[derive(Debug, Default, Serialize)]
pub struct LoadOutcome {
    pub records: Vec<MonthRecord>,
    /// Fichiers `records/*.enc` illisibles (altérés, ou chiffrés avec une autre clé).
    pub skipped: Vec<String>,
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Seuls ces chemins (relatifs à la racine du dépôt) ont le droit d'être commités.
pub fn is_committable_path(path: &str) -> bool {
    fn opaque(dir: &str, path: &str) -> bool {
        path.strip_prefix(dir)
            .and_then(|p| p.strip_prefix('/'))
            .and_then(|p| p.strip_suffix(".enc"))
            .is_some_and(is_opaque_id)
    }
    matches!(path, VAULT_FILE | "README.md" | ".gitattributes" | ".gitignore")
        || opaque(RECORDS_DIR, path)
        || opaque(FILES_DIR, path)
}

fn is_opaque_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn validate_year_month(year: i32, month: u8) -> Result<()> {
    if !(MIN_YEAR..=MAX_YEAR).contains(&year) {
        return Err(Error::InvalidInput(format!("Année hors limites ({MIN_YEAR}–{MAX_YEAR}).")));
    }
    if !(1..=12).contains(&month) {
        return Err(Error::InvalidInput("Le mois doit être compris entre 1 et 12.".into()));
    }
    Ok(())
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn validate_date(s: &str) -> Result<()> {
    let bad = || Error::InvalidInput("Date invalide (format attendu : AAAA-MM-JJ).".into());
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return Err(bad());
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return Err(bad());
    }
    let y: i32 = s[0..4].parse().map_err(|_| bad())?;
    let m: u32 = s[5..7].parse().map_err(|_| bad())?;
    let d: u32 = s[8..10].parse().map_err(|_| bad())?;
    let dim = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => return Err(bad()),
    };
    if d == 0 || d > dim {
        return Err(bad());
    }
    Ok(())
}

fn validate_input(i: &MonthInput) -> Result<()> {
    validate_year_month(i.year, i.month)?;
    if let Some(a) = i.amount_cents {
        if !(0..=MAX_AMOUNT_CENTS).contains(&a) {
            return Err(Error::InvalidInput("Montant invalide.".into()));
        }
    }
    if let Some(d) = &i.declared_on {
        validate_date(d)?;
    }
    if i.notes.chars().count() > MAX_NOTES_CHARS {
        return Err(Error::InvalidInput(format!("Les notes sont limitées à {MAX_NOTES_CHARS} caractères.")));
    }
    Ok(())
}

/// Type MIME d'après l'extension. Seuls les PDF et les images sont acceptés : c'est ce que
/// demande l'usage (accusés, justificatifs) et cela garantit que « Ouvrir » ne lance jamais
/// un exécutable.
pub fn mime_for_extension(ext: &str) -> Option<&'static str> {
    Some(match ext.to_ascii_lowercase().as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "heic" => "image/heic",
        "heif" => "image/heif",
        _ => return None,
    })
}

/// Extension à utiliser pour un fichier temporaire, déduite du MIME *validé* (pas du nom stocké).
pub fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "application/pdf" => "pdf",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        "image/heic" => "heic",
        "image/heif" => "heif",
        _ => "bin",
    }
}

/// Garde uniquement le nom du fichier, sans dossier ni caractère de contrôle.
pub fn sanitize_file_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    let cleaned: String = base.chars().filter(|c| !c.is_control()).collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    let cleaned: String = cleaned.chars().take(200).collect();
    if cleaned.is_empty() {
        "piece-jointe".into()
    } else {
        cleaned
    }
}

impl Vault {
    fn record_label(year: i32, month: u8) -> String {
        format!("record:{year:04}-{month:02}")
    }

    fn record_name(&self, year: i32, month: u8) -> String {
        self.keys.opaque_name(&Self::record_label(year, month))
    }

    fn record_file(&self, year: i32, month: u8) -> std::path::PathBuf {
        self.root.join(RECORDS_DIR).join(format!("{}.enc", self.record_name(year, month)))
    }

    fn attachment_file(&self, id: &str) -> Result<std::path::PathBuf> {
        if !is_opaque_id(id) {
            return Err(Error::InvalidInput("Identifiant de pièce jointe invalide.".into()));
        }
        Ok(self.root.join(FILES_DIR).join(format!("{id}.enc")))
    }

    fn write_record(&self, rec: &MonthRecord) -> Result<()> {
        let name = self.record_name(rec.year, rec.month);
        let json = Zeroizing::new(
            serde_json::to_vec(rec).map_err(|e| Error::Corrupt(format!("sérialisation ({e})")))?,
        );
        let blob = self.keys.seal(&format!("record-file:{name}"), &json);
        atomic_write(&self.root.join(RECORDS_DIR).join(format!("{name}.enc")), &blob)
    }

    /// Déchiffre le contenu d'un fichier `records/<nom>.enc` (utilisé aussi pour arbitrer les
    /// conflits de synchronisation). Vérifie que le nom correspond bien au mois contenu.
    pub fn open_record_blob(&self, rel_path: &str, blob: &[u8]) -> Result<MonthRecord> {
        let name = rel_path
            .strip_prefix("records/")
            .and_then(|p| p.strip_suffix(".enc"))
            .filter(|n| is_opaque_id(n))
            .ok_or_else(|| Error::Corrupt(format!("chemin d'enregistrement inattendu : {rel_path}")))?;
        let plain = self.keys.open(&format!("record-file:{name}"), blob)?;
        let rec: MonthRecord = serde_json::from_slice(&plain)
            .map_err(|e| Error::Corrupt(format!("enregistrement illisible ({e})")))?;
        validate_year_month(rec.year, rec.month)?;
        if self.record_name(rec.year, rec.month) != name {
            return Err(Error::Corrupt("enregistrement rangé sous un mauvais nom".into()));
        }
        Ok(rec)
    }

    pub fn load_all(&self) -> Result<LoadOutcome> {
        let dir = self.root.join(RECORDS_DIR);
        let mut out = LoadOutcome::default();
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if !file_name.ends_with(".enc") {
                continue; // fichiers temporaires `.xxx.tmp` d'une écriture interrompue
            }
            let rel = format!("{RECORDS_DIR}/{file_name}");
            match fs::read(entry.path())
                .map_err(Error::from)
                .and_then(|blob| self.open_record_blob(&rel, &blob))
            {
                Ok(rec) => out.records.push(rec),
                Err(_) => out.skipped.push(file_name),
            }
        }
        out.records.sort_by_key(|r| (r.year, r.month));
        out.skipped.sort();
        Ok(out)
    }

    pub fn load_month(&self, year: i32, month: u8) -> Result<Option<MonthRecord>> {
        validate_year_month(year, month)?;
        let path = self.record_file(year, month);
        match fs::read(&path) {
            Ok(blob) => {
                let rel = format!("{RECORDS_DIR}/{}", path.file_name().unwrap().to_string_lossy());
                self.open_record_blob(&rel, &blob).map(Some)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn load_or_new(&self, year: i32, month: u8) -> Result<MonthRecord> {
        Ok(self.load_month(year, month)?.unwrap_or(MonthRecord {
            year,
            month,
            amount_cents: None,
            declared_on: None,
            status: Status::ToDeclare,
            notes: String::new(),
            attachments: Vec::new(),
            updated_at: 0,
        }))
    }

    /// Crée ou met à jour un mois. Les pièces jointes existantes sont conservées.
    pub fn save_month(&self, input: &MonthInput) -> Result<MonthRecord> {
        validate_input(input)?;
        let mut rec = self.load_or_new(input.year, input.month)?;
        rec.amount_cents = input.amount_cents;
        rec.declared_on = input.declared_on.clone();
        rec.status = input.status;
        rec.notes = input.notes.clone();
        rec.updated_at = now_ms();
        self.write_record(&rec)?;
        Ok(rec)
    }

    /// Supprime un mois et toutes ses pièces jointes.
    pub fn delete_month(&self, year: i32, month: u8) -> Result<()> {
        validate_year_month(year, month)?;
        let Some(rec) = self.load_month(year, month)? else { return Ok(()) };
        // Le mois d'abord : si on est interrompu ensuite, il reste des fichiers orphelins
        // (inoffensifs) plutôt qu'un mois qui pointe vers des pièces disparues.
        remove_if_exists(&self.record_file(year, month))?;
        for a in &rec.attachments {
            remove_if_exists(&self.attachment_file(&a.id)?)?;
        }
        Ok(())
    }

    pub fn add_attachment(&self, year: i32, month: u8, file_name: &str, bytes: &[u8]) -> Result<MonthRecord> {
        validate_year_month(year, month)?;
        if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
            return Err(Error::InvalidInput(format!(
                "Fichier trop volumineux (maximum {} Mo).",
                MAX_ATTACHMENT_BYTES / 1024 / 1024
            )));
        }
        let name = sanitize_file_name(file_name);
        let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        let mime = mime_for_extension(ext).ok_or_else(|| {
            Error::InvalidInput(
                "Seuls les PDF et les images (PNG, JPEG, GIF, WebP, BMP, TIFF, HEIC) sont acceptés.".into(),
            )
        })?;
        let mut rec = self.load_or_new(year, month)?;
        let id = hex::encode(random_bytes::<16>());
        let blob = self.keys.seal(&format!("file:{id}"), bytes);
        atomic_write(&self.attachment_file(&id)?, &blob)?;
        rec.attachments.push(Attachment {
            id: id.clone(),
            name,
            mime: mime.into(),
            size: bytes.len() as u64,
            added_at: now_ms(),
        });
        rec.updated_at = now_ms();
        if let Err(e) = self.write_record(&rec) {
            let _ = fs::remove_file(self.attachment_file(&id)?);
            return Err(e);
        }
        Ok(rec)
    }

    pub fn add_attachment_from_path(&self, year: i32, month: u8, path: &Path) -> Result<MonthRecord> {
        let meta = fs::metadata(path)?;
        if !meta.is_file() {
            return Err(Error::InvalidInput("Ce n'est pas un fichier.".into()));
        }
        if meta.len() > MAX_ATTACHMENT_BYTES {
            return Err(Error::InvalidInput(format!(
                "Fichier trop volumineux (maximum {} Mo).",
                MAX_ATTACHMENT_BYTES / 1024 / 1024
            )));
        }
        let bytes = Zeroizing::new(fs::read(path)?);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.add_attachment(year, month, &name, &bytes)
    }

    pub fn remove_attachment(&self, year: i32, month: u8, id: &str) -> Result<MonthRecord> {
        let mut rec =
            self.load_month(year, month)?.ok_or_else(|| Error::InvalidInput("Mois introuvable.".into()))?;
        let before = rec.attachments.len();
        rec.attachments.retain(|a| a.id != id);
        if rec.attachments.len() == before {
            return Err(Error::InvalidInput("Pièce jointe introuvable dans ce mois.".into()));
        }
        rec.updated_at = now_ms();
        self.write_record(&rec)?;
        remove_if_exists(&self.attachment_file(id)?)?;
        Ok(rec)
    }

    /// Déchiffre une pièce jointe ; la pièce doit appartenir au mois indiqué.
    pub fn read_attachment(
        &self,
        year: i32,
        month: u8,
        id: &str,
    ) -> Result<(Attachment, Zeroizing<Vec<u8>>)> {
        let rec =
            self.load_month(year, month)?.ok_or_else(|| Error::InvalidInput("Mois introuvable.".into()))?;
        let att = rec
            .attachments
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| Error::InvalidInput("Pièce jointe introuvable dans ce mois.".into()))?;
        let blob = fs::read(self.attachment_file(id)?).map_err(|e| {
            Error::Corrupt(format!("pièce jointe absente du dépôt ({e}) — synchronisez puis réessayez"))
        })?;
        let plain = self.keys.open(&format!("file:{id}"), &blob)?;
        Ok((att, plain))
    }
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::KdfParams;
    use tempfile::{tempdir, TempDir};

    fn vault() -> (TempDir, Vault) {
        let dir = tempdir().unwrap();
        let v = Vault::create(dir.path(), "correct horse battery staple", KdfParams::FAST_FOR_TESTS).unwrap();
        (dir, v)
    }

    fn input(year: i32, month: u8) -> MonthInput {
        MonthInput {
            year,
            month,
            amount_cents: Some(123_456),
            declared_on: Some(format!("{year}-{month:02}-05")),
            status: Status::Declared,
            notes: "note secrète ÉÉ".into(),
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        let (_d, v) = vault();
        let saved = v.save_month(&input(2025, 3)).unwrap();
        assert_eq!(saved.amount_cents, Some(123_456));
        assert!(saved.updated_at > 0);
        assert_eq!(v.load_month(2025, 3).unwrap().unwrap(), saved);
        assert!(v.load_month(2025, 4).unwrap().is_none());
        let all = v.load_all().unwrap();
        assert_eq!(all.records, vec![saved]);
        assert!(all.skipped.is_empty());
    }

    #[test]
    fn records_are_sorted_and_persist_across_reopen() {
        let (d, v) = vault();
        for (y, m) in [(2025, 12), (2024, 1), (2025, 2)] {
            v.save_month(&input(y, m)).unwrap();
        }
        drop(v);
        let v = Vault::open(d.path(), "correct horse battery staple").unwrap();
        let ids: Vec<_> = v.load_all().unwrap().records.iter().map(|r| (r.year, r.month)).collect();
        assert_eq!(ids, vec![(2024, 1), (2025, 2), (2025, 12)]);
    }

    #[test]
    fn nothing_readable_on_disk() {
        let (d, v) = vault();
        let mut i = input(2025, 3);
        i.notes = "SUPERSECRETNOTE".into();
        v.save_month(&i).unwrap();
        v.add_attachment(2025, 3, "relevé-confidentiel.pdf", b"%PDF-1.4 CONTENUPDFSECRET").unwrap();
        let mut seen = 0;
        for entry in walk(d.path()) {
            let bytes = fs::read(&entry).unwrap();
            let name = entry.strip_prefix(d.path()).unwrap().to_string_lossy().into_owned();
            for needle in [
                "SUPERSECRETNOTE",
                "CONTENUPDFSECRET",
                "confidentiel",
                "123456",
                "2025",
                "declared",
                "Declared",
            ] {
                assert!(
                    !String::from_utf8_lossy(&bytes).contains(needle),
                    "« {needle} » visible dans {name}"
                );
            }
            // Les noms sont de l'hexadécimal : on ne cherche que des lettres hors a-f.
            for needle in ["pdf", "confidentiel", "declared", "SECRET"] {
                assert!(!name.contains(needle), "« {needle} » visible dans le nom {name}");
            }
            if name != VAULT_FILE {
                assert!(is_committable_path(&name.replace('\\', "/")), "{name} hors liste blanche");
                seen += 1;
            }
        }
        assert_eq!(seen, 2);
    }

    fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                out.extend(walk(&p))
            } else {
                out.push(p)
            }
        }
        out
    }

    #[test]
    fn validation() {
        let (_d, v) = vault();
        let mut i = input(2025, 3);
        i.month = 13;
        assert!(v.save_month(&i).is_err());
        i = input(1999, 3);
        assert!(v.save_month(&i).is_err());
        i = input(2025, 3);
        i.amount_cents = Some(-1);
        assert!(v.save_month(&i).is_err());
        for bad in [
            "2025-02-30",
            "2025-13-01",
            "2025-1-01",
            "25-01-01",
            "2025/01/01",
            "2025-01-0a",
            "2025-00-10",
            "2023-02-29",
        ] {
            i = input(2025, 3);
            i.declared_on = Some(bad.into());
            assert!(v.save_month(&i).is_err(), "{bad} accepté");
        }
        for good in ["2024-02-29", "2025-01-31", "2000-02-29"] {
            i = input(2025, 3);
            i.declared_on = Some(good.into());
            v.save_month(&i).unwrap();
        }
        i = input(2025, 3);
        i.notes = "x".repeat(MAX_NOTES_CHARS + 1);
        assert!(v.save_month(&i).is_err());
    }

    #[test]
    fn editing_a_month_keeps_its_attachments() {
        let (_d, v) = vault();
        v.save_month(&input(2025, 3)).unwrap();
        v.add_attachment(2025, 3, "ar.pdf", b"%PDF-1").unwrap();
        let mut i = input(2025, 3);
        i.amount_cents = Some(999);
        let rec = v.save_month(&i).unwrap();
        assert_eq!(rec.attachments.len(), 1);
        assert_eq!(rec.amount_cents, Some(999));
    }

    #[test]
    fn attachments_roundtrip_and_removal() {
        let (d, v) = vault();
        let rec =
            v.add_attachment(2025, 3, "C:\\Users\\moi\\Accusé réception.PDF", b"%PDF-1.7 hello").unwrap();
        assert_eq!(rec.status, Status::ToDeclare, "créer une pièce crée un mois vide par défaut");
        let a = rec.attachments[0].clone();
        assert_eq!(a.name, "Accusé réception.PDF");
        assert_eq!(a.mime, "application/pdf");
        assert_eq!(a.size, 14);
        let (meta, bytes) = v.read_attachment(2025, 3, &a.id).unwrap();
        assert_eq!(meta, a);
        assert_eq!(bytes.as_slice(), b"%PDF-1.7 hello");
        assert!(d.path().join("files").join(format!("{}.enc", a.id)).is_file());

        let rec = v.remove_attachment(2025, 3, &a.id).unwrap();
        assert!(rec.attachments.is_empty());
        assert!(!d.path().join("files").join(format!("{}.enc", a.id)).exists());
        assert!(v.read_attachment(2025, 3, &a.id).is_err());
    }

    #[test]
    fn attachment_rules() {
        let (_d, v) = vault();
        assert!(v.add_attachment(2025, 3, "virus.exe", b"MZ").is_err());
        assert!(v.add_attachment(2025, 3, "noext", b"x").is_err());
        let big = vec![0u8; MAX_ATTACHMENT_BYTES as usize + 1];
        assert!(v.add_attachment(2025, 3, "big.pdf", &big).is_err());
        // Un id forgé ne doit jamais sortir du dossier files/.
        assert!(v.read_attachment(2025, 3, "../vault").is_err());
        v.save_month(&input(2025, 3)).unwrap();
        assert!(v.remove_attachment(2025, 3, "../../etc/passwd").is_err());
        assert!(v.load_all().unwrap().records.len() == 1, "un refus ne crée pas de mois");
    }

    #[test]
    fn a_pdf_attached_to_another_month_is_not_readable_via_wrong_month() {
        let (_d, v) = vault();
        let rec = v.add_attachment(2025, 3, "a.pdf", b"x").unwrap();
        v.save_month(&input(2025, 4)).unwrap();
        assert!(v.read_attachment(2025, 4, &rec.attachments[0].id).is_err());
    }

    #[test]
    fn delete_month_removes_everything() {
        let (d, v) = vault();
        v.save_month(&input(2025, 3)).unwrap();
        let rec = v.add_attachment(2025, 3, "a.png", b"png").unwrap();
        v.delete_month(2025, 3).unwrap();
        assert!(v.load_month(2025, 3).unwrap().is_none());
        assert!(!d.path().join("files").join(format!("{}.enc", rec.attachments[0].id)).exists());
        v.delete_month(2025, 3).unwrap(); // idempotent
    }

    #[test]
    fn corrupted_record_is_reported_not_hidden_and_does_not_block_others() {
        let (d, v) = vault();
        v.save_month(&input(2025, 3)).unwrap();
        v.save_month(&input(2025, 4)).unwrap();
        let victim = d.path().join("records").join(format!("{}.enc", v.record_name(2025, 3)));
        let mut bytes = fs::read(&victim).unwrap();
        bytes[20] ^= 0xff;
        fs::write(&victim, bytes).unwrap();
        let out = v.load_all().unwrap();
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.skipped.len(), 1);
    }

    #[test]
    fn swapping_two_record_files_is_detected() {
        let (d, v) = vault();
        v.save_month(&input(2025, 3)).unwrap();
        v.save_month(&input(2025, 4)).unwrap();
        let a = d.path().join("records").join(format!("{}.enc", v.record_name(2025, 3)));
        let b = d.path().join("records").join(format!("{}.enc", v.record_name(2025, 4)));
        let (ba, bb) = (fs::read(&a).unwrap(), fs::read(&b).unwrap());
        fs::write(&a, bb).unwrap();
        fs::write(&b, ba).unwrap();
        assert_eq!(v.load_all().unwrap().skipped.len(), 2);
    }

    #[test]
    fn leftover_temp_files_are_ignored() {
        let (d, v) = vault();
        v.save_month(&input(2025, 3)).unwrap();
        fs::write(d.path().join("records").join(".abc.enc.tmp"), b"junk").unwrap();
        let out = v.load_all().unwrap();
        assert_eq!(out.records.len(), 1);
        assert!(out.skipped.is_empty());
    }

    #[test]
    fn committable_allowlist() {
        let ok = format!("records/{}.enc", "a".repeat(32));
        assert!(is_committable_path(&ok));
        assert!(is_committable_path(&format!("files/{}.enc", "0f".repeat(16))));
        for p in ["vault.json", "README.md", ".gitattributes", ".gitignore"] {
            assert!(is_committable_path(p));
        }
        for bad in [
            "notes.txt",
            "records/2025-03.json",
            "records/short.enc",
            &format!("records/{}.enc", "A".repeat(32)),
            &format!("records/{}.enc.tmp", "a".repeat(32)),
            &format!("other/{}.enc", "a".repeat(32)),
            &format!("records/sub/{}.enc", "a".repeat(32)),
            &format!("../records/{}.enc", "a".repeat(32)),
            "releve.pdf",
            ".env",
        ] {
            assert!(!is_committable_path(bad), "{bad} aurait dû être refusé");
        }
    }

    #[test]
    fn file_name_sanitising() {
        assert_eq!(sanitize_file_name("../../etc/passwd.pdf"), "passwd.pdf");
        assert_eq!(sanitize_file_name("a\\b\\c.png"), "c.png");
        assert_eq!(sanitize_file_name("  \u{7}.. "), "piece-jointe");
        assert_eq!(sanitize_file_name(&"é".repeat(500)).chars().count(), 200);
    }
}
