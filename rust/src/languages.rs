//! Optional printed-OCR resources pinned to the official Tesseract data repository.
use crate::{
    storage::{self, Activity, Control},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguagePack {
    pub code: String,
    pub label: String,
    pub script: String,
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
    pub revision: String,
    pub url: String,
    pub license: String,
}
pub fn catalog() -> Vec<LanguagePack> {
    serde_json::from_str::<Vec<LanguagePack>>(include_str!("../languages.json"))
        .expect("compiled language catalog")
        .into_iter()
        .filter(|p| p.code != "license")
        .collect()
}
fn assets() -> Result<Vec<LanguagePack>> {
    let packs: Vec<LanguagePack> = serde_json::from_str(include_str!("../languages.json"))?;
    let mut codes = BTreeSet::new();
    for p in &packs {
        anyhow::ensure!(
            codes.insert(&p.code)
                && ["eng", "sin", "tam", "ara", "heb", "osd", "license"].contains(&p.code.as_str())
                && p.revision == "87416418657359cb625c412a48b6e1d6d41c29bd"
                && p.bytes > 0
                && p.bytes <= 20_000_000
                && p.sha256.len() == 64
                && p.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                && p.license == "Apache-2.0"
                && p.file
                    == if p.code == "license" {
                        "LICENSE-tessdata-fast.txt".into()
                    } else {
                        format!("{}.traineddata", p.code)
                    }
                && p.url
                    == format!(
                        "https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/{}/{}",
                        p.revision,
                        if p.code == "license" {
                            "LICENSE".to_string()
                        } else {
                            p.file.clone()
                        }
                    ),
            Fault::new("NG740", "Language resource pins failed validation")
        );
    }
    anyhow::ensure!(
        codes.len() == 7,
        Fault::new("NG740", "Compiled language catalog is incomplete")
    );
    Ok(packs)
}
pub fn selection(codes: &[String]) -> Result<String> {
    let mut seen = BTreeSet::new();
    anyhow::ensure!(
        !codes.is_empty()
            && codes.len() <= 5
            && codes.iter().all(
                |s| ["eng", "sin", "tam", "ara", "heb"].contains(&s.as_str()) && seen.insert(s)
            ),
        Fault::new(
            "NG740",
            "Choose unique recognition languages: eng, sin, tam, ara or heb"
        )
    );
    Ok(codes.join("+"))
}
fn plain_path(path: &Path) -> Result<()> {
    for part in std::path::absolute(path)?.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(m) => {
                let linked = m.file_type().is_symlink();
                #[cfg(windows)]
                let linked = {
                    use std::os::windows::fs::MetadataExt;
                    linked || m.file_attributes() & 0x400 != 0
                };
                anyhow::ensure!(
                    !linked,
                    Fault::new(
                        "NG740",
                        "OCR resource folders cannot contain symlinks or junctions"
                    )
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn healthy(path: &Path, p: &LanguagePack, c: &Control) -> Result<bool> {
    plain_path(path)?;
    c.check()?;
    match fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
        Ok(m) => Ok(m.is_file()
            && m.len() == p.bytes
            && storage::hash_file(path, c)?.eq_ignore_ascii_case(&p.sha256)),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackState {
    pub pack: LanguagePack,
    pub verified: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LanguagePlan {
    pub files: Vec<PackState>,
    pub remaining_bytes: u64,
    pub total_bytes: u64,
    pub recognition_languages: String,
    pub notes: Vec<String>,
}
pub fn plan(dir: &Path, codes: &[String], c: &Control) -> Result<LanguagePlan> {
    let recognition_languages = selection(codes)?;
    plain_path(dir)?;
    let mut required = codes.iter().map(String::as_str).collect::<BTreeSet<_>>();
    // A custom tessdata folder needs its own baseline English and orientation files.
    required.extend(["eng", "osd", "license"]);
    let mut files = vec![];
    let mut remaining_bytes = 0;
    let mut total_bytes = 0;
    for pack in assets()?
        .into_iter()
        .filter(|p| required.contains(p.code.as_str()))
    {
        let verified = healthy(&dir.join(&pack.file), &pack, c)?;
        total_bytes += pack.bytes;
        if !verified {
            remaining_bytes += pack.bytes;
        }
        files.push(PackState { pack, verified });
    }
    Ok(LanguagePlan{files,remaining_bytes,total_bytes,recognition_languages,notes:vec![
        "These official fast integer resources support printed LSTM OCR in Tesseract 4/5. They do not establish handwriting accuracy.".into(),
        "Combined language order is manual and applies to a whole OCR pass; automatic paragraph/region language routing is not implemented.".into(),
        "SHA256 and byte length are verified before activation. English, orientation data and the Apache-2.0 license are included in a custom resource folder.".into()]})
}
pub fn verify_selection(dir: &Path, languages: &str, c: &Control) -> Result<()> {
    let codes = languages.split('+').map(str::to_string).collect::<Vec<_>>();
    let report = plan(dir, &codes, c)?;
    anyhow::ensure!(
        report.files.iter().all(|f| f.verified),
        Fault::new(
            "NG740",
            "Required OCR resources are missing or fail SHA256 verification; install or repair the selected packs"
        )
    );
    Ok(())
}
fn download(dir: &Path, pack: &LanguagePack, c: &Control, log: &Activity) -> Result<()> {
    c.check()?;
    plain_path(dir)?;
    let destination = dir.join(&pack.file);
    if healthy(&destination, pack, c)? {
        return Ok(());
    }
    let current = match destination.metadata() {
        Ok(metadata) => {
            anyhow::ensure!(
                metadata.is_file() && metadata.len() <= 20_000_000,
                Fault::new(
                    "NG740",
                    "Existing OCR resource is not a regular bounded file; preserve it and choose another resource folder"
                )
            );
            (
                Some(metadata.len()),
                Some(storage::hash_file(&destination, c)?),
            )
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, None),
        Err(e) => return Err(e.into()),
    };
    let expected = crate::diagnostics::RepairFile {
        key: format!("ocr/{}", pack.code),
        expected_bytes: pack.bytes,
        expected_sha256: pack.sha256.clone(),
        current_bytes: current.0,
        current_sha256: current.1,
        health: if current.0.is_some() {
            crate::diagnostics::AssetHealth::Corrupt
        } else {
            crate::diagnostics::AssetHealth::Missing
        },
    };
    let staging = tempfile::Builder::new()
        .prefix(".notegen-ocr-")
        .tempdir_in(dir)?;
    let staged = tempfile::TempPath::try_from_path(staging.path().join("verified-asset"))?;
    crate::models::download(
        &pack.url,
        &staged,
        &crate::models::Asset {
            file: pack.file.clone(),
            bytes: pack.bytes,
            sha256: pack.sha256.clone(),
        },
        c,
        log,
    )?;
    c.check()?;
    plain_path(&destination)?;
    if let Some(backup) = crate::diagnostics::publish_repair(staged, &destination, &expected, c)? {
        log.emit(
            "repair",
            format!(
                "OCR {} repaired; previous bytes retained at {}",
                pack.code,
                backup.display()
            ),
            0,
            1,
            1,
            "",
        );
    }
    Ok(())
}
pub fn install(dir: &Path, codes: &[String], c: &Control, log: &Activity) -> Result<LanguagePlan> {
    let preview = plan(dir, codes, c)?;
    c.check()?;
    fs::create_dir_all(dir)?;
    plain_path(dir)?;
    for file in preview.files.iter().filter(|f| !f.verified) {
        download(dir, &file.pack, c, log)?;
    }
    let result = plan(dir, codes, c)?;
    anyhow::ensure!(
        result.files.iter().all(|f| f.verified),
        Fault::new(
            "NG740",
            "OCR resource verification failed after installation"
        )
    );
    Ok(result)
}
pub fn recommended_directory(models_dir: &Path) -> PathBuf {
    models_dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .join("ocr-resources/tessdata")
}
