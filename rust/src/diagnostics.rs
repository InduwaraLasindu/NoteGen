//! Inspectable support exports and repair of publisher-pinned local assets.
use crate::{models, setup, storage::*, types::*};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
};

const MAX_BUNDLE: usize = 32_000_000;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SupportOptions {
    pub include_activity: bool,
    pub include_document_content: bool,
}
pub struct SupportInput<'a> {
    pub settings: &'a Settings,
    pub activity: &'a [PathBuf],
    pub documents: &'a [PathBuf],
    pub options: &'a SupportOptions,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
    pub document_content: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportPreview {
    pub schema: u32,
    pub files: Vec<SupportFile>,
    pub options: SupportOptions,
    pub exclusions: Vec<String>,
    pub token: String,
}
fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn support_fault(detail: &str) -> anyhow::Error {
    Fault::new("NG730", detail).into()
}
fn read_bounded(path: &Path, limit: u64, c: &Control) -> Result<Vec<u8>> {
    c.check()?;
    let mut file = fs::File::open(path)
        .map_err(|_| support_fault("Selected support file cannot be opened"))?;
    anyhow::ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= limit,
        Fault::new(
            "NG730",
            "Selected support file is not a regular file or exceeds its size limit"
        )
    );
    let mut bytes = vec![];
    Read::by_ref(&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| support_fault("Selected support file could not be read completely"))?;
    c.check()?;
    anyhow::ensure!(
        bytes.len() as u64 <= limit,
        Fault::new("NG730", "Support file grew beyond its size limit")
    );
    Ok(bytes)
}
fn safe_activity(bytes: &[u8]) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| support_fault("Activity file must contain UTF-8 JSON lines"))?;
    let mut entries = vec![];
    let mut malformed = 0usize;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        anyhow::ensure!(
            entries.len() + malformed < 10000,
            Fault::new("NG730", "Activity preview exceeds 10000 entries per file")
        );
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            malformed += 1;
            continue;
        };
        let stage = v["stage"].as_str().unwrap_or("");
        let stage = if [
            "progress", "render", "ocr", "read", "crop", "load", "export", "models", "runtime",
            "download", "verify", "prepare", "complete", "cached", "warning", "error", "activate",
        ]
        .contains(&stage)
        {
            stage
        } else {
            "other"
        };
        let code = v["code"].as_str().unwrap_or("");
        let code = if code.len() == 5
            && code.starts_with("NG")
            && code.as_bytes()[2..].iter().all(u8::is_ascii_digit)
        {
            code
        } else {
            ""
        };
        let timestamp = v["timestamp"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.to_utc().to_rfc3339());
        entries.push(json!({"timestamp":timestamp,"stage":stage,"code":code,
            "page":v["page"].as_u64().unwrap_or(0).min(3000),"done":v["done"].as_u64().unwrap_or(0),
            "total":v["total"].as_u64().unwrap_or(0),"warning":v["warning"].as_bool().unwrap_or(false)}));
    }
    Ok(serde_json::to_vec_pretty(
        &json!({"entries":entries,"malformed_entries_omitted":malformed,
        "omitted":"Raw messages, recognition drafts, paths, model names and previews are excluded."}),
    )?)
}
fn insert_support_file(
    files: &mut BTreeMap<String, Vec<u8>>,
    name: String,
    bytes: Vec<u8>,
) -> Result<()> {
    anyhow::ensure!(
        files
            .values()
            .try_fold(bytes.len(), |n, b| n.checked_add(b.len()))
            .is_some_and(|n| n <= MAX_BUNDLE),
        Fault::new(
            "NG730",
            "Support bundle exceeds 32 MB; select fewer or smaller attachments"
        )
    );
    files.insert(name, bytes);
    Ok(())
}
fn support_files(input: &SupportInput<'_>, c: &Control) -> Result<BTreeMap<String, Vec<u8>>> {
    c.check()?;
    input.settings.validate()?;
    anyhow::ensure!(
        input.activity.len() <= 16 && input.documents.len() <= 8,
        Fault::new(
            "NG730",
            "Choose at most 16 activity files and eight explicit document attachments"
        )
    );
    let s = input.settings;
    let summary = json!({"schema":1,"application":"NoteGen","version":env!("CARGO_PKG_VERSION"),
        "platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,
        "settings":{"preset":s.preset,"backend":s.backend,"cpu_threads":s.cpu_threads,
            "dpi":s.dpi,"image_edge":s.image_edge,"context":s.context,"max_tokens":s.max_tokens,
            "ram_fraction":s.ram_fraction,"vram_fraction":s.vram_fraction,"reserve_ram_mib":s.reserve_ram_mib,
            "reserve_vram_mib":s.reserve_vram_mib,"automatic_models":s.automatic_models,
            "check_updates":s.experience.check_updates},
        "privacy":"Paths, endpoints, environment variables, identifiers, subject text, writer data and recognized content are excluded from this summary."});
    let mut files = BTreeMap::from([(
        "settings-summary.json".into(),
        serde_json::to_vec_pretty(&summary)?,
    )]);
    if input.options.include_activity {
        for (i, path) in input.activity.iter().enumerate() {
            let bytes = read_bounded(path, 2_000_000, c)?;
            insert_support_file(
                &mut files,
                format!("activity-{:04}.json", i + 1),
                safe_activity(&bytes)?,
            )?;
        }
    }
    if input.options.include_document_content {
        for (i, path) in input.documents.iter().enumerate() {
            insert_support_file(
                &mut files,
                format!("document-{:04}.bin", i + 1),
                read_bounded(path, 16_000_000, c)?,
            )?;
        }
    }
    anyhow::ensure!(
        files
            .values()
            .try_fold(0usize, |n, b| n.checked_add(b.len()))
            .is_some_and(|n| n <= MAX_BUNDLE),
        Fault::new(
            "NG730",
            "Support bundle exceeds 32 MB; select fewer or smaller attachments"
        )
    );
    Ok(files)
}
fn support_preview_from(
    input: &SupportInput<'_>,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<SupportPreview> {
    let mut preview=SupportPreview {schema:1, options:input.options.clone(), token:String::new(),
        files:files.iter().map(|(name,bytes)|SupportFile{name:name.clone(),bytes:bytes.len() as u64,
            sha256:digest(bytes),document_content:name.starts_with("document-")}).collect(),
        exclusions:vec!["Environment variables and credential stores are never read.".into(),
            "Raw log messages and source paths are never included.".into(),
            "Document bytes are included only when separately enabled and explicitly selected for export.".into()]};
    preview.token = fingerprint(&preview)?;
    Ok(preview)
}
pub fn support_preview(input: &SupportInput<'_>, c: &Control) -> Result<SupportPreview> {
    support_preview_from(input, &support_files(input, c)?)
}
/// Inspect the exact sanitized bytes listed in a file-by-file support preview.
pub fn inspect_support_file(input: &SupportInput<'_>, name: &str, c: &Control) -> Result<Vec<u8>> {
    support_files(input, c)?
        .remove(name)
        .ok_or_else(|| support_fault("File is not in the current support preview"))
}
fn write_support_zip<W: Write + Seek>(
    writer: W,
    files: &BTreeMap<String, Vec<u8>>,
    selected: &[String],
    c: &Control,
) -> Result<W> {
    let mut zip = zip::ZipWriter::new(writer);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut manifest = vec![];
    for name in selected {
        c.check()?;
        let bytes = &files[name];
        zip.start_file(name, options)?;
        zip.write_all(bytes)?;
        manifest.push(SupportFile {
            name: name.clone(),
            bytes: bytes.len() as u64,
            sha256: digest(bytes),
            document_content: name.starts_with("document-"),
        });
    }
    c.check()?;
    zip.start_file("support-manifest.json", options)?;
    zip.write_all(&serde_json::to_vec_pretty(
        &json!({"schema":1,"files":manifest}),
    )?)?;
    Ok(zip.finish()?)
}
pub fn export_support(
    input: &SupportInput<'_>,
    expected: &SupportPreview,
    token: &str,
    selected: &[String],
    output: &Path,
    c: &Control,
) -> Result<()> {
    c.check()?;
    let files = support_files(input, c)?;
    let actual = support_preview_from(input, &files)?;
    anyhow::ensure!(
        token == expected.token && *expected == actual,
        Fault::new(
            "NG730",
            "Support contents or preview changed; inspect a new preview before export"
        )
    );
    let unique = selected.iter().collect::<BTreeSet<_>>();
    anyhow::ensure!(
        !selected.is_empty()
            && unique.len() == selected.len()
            && selected.iter().all(|n| files.contains_key(n)),
        Fault::new(
            "NG730",
            "Select unique files from the inspected support preview"
        )
    );
    anyhow::ensure!(
        !output.exists(),
        Fault::new("NG401", "Support output already exists")
    );
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    write_support_zip(temporary.as_file_mut(), &files, selected, c)?;
    temporary.as_file().sync_all()?;
    c.check()?;
    temporary.persist_noclobber(output).map_err(|_| {
        Fault::new(
            "NG401",
            "Support output exists or its folder is unavailable",
        )
    })?;
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct Troubleshooting {
    pub code: String,
    pub title: String,
    pub checks: Vec<String>,
    pub actions: Vec<String>,
}
pub fn troubleshoot(code: &str) -> Troubleshooting {
    let (title, checks, actions): (&str, &[&str], &[&str]) = match code {
        "NG100" => (
            "Source cannot be read",
            &[
                "Confirm the file still exists and opens locally.",
                "Check the selected page range and supported input format.",
            ],
            &[
                "Copy removable-storage input to a local folder and try again.",
                "Convert unsupported TIFF sample formats to an RGB PNG.",
            ],
        ),
        "NG102" => (
            "Settings need attention",
            &["Review the named setting and supported range."],
            &["Restore its default or load a validated template."],
        ),
        "NG110" => (
            "Printed OCR is unavailable",
            &[
                "Check that Tesseract and the selected language packs are installed.",
                "Check whether the native executable is permitted to run.",
            ],
            &[
                "Repair the app installation or choose the correct Tesseract executable.",
                "If security policy blocks execution, ask the device administrator; do not bypass it.",
            ],
        ),
        "NG200" | "NG202" | "NG300" => (
            "The reader cannot start with these resources",
            &[
                "Check free RAM/VRAM and the selected reader size.",
                "Check the installed model/projector pair and runtime backend.",
            ],
            &[
                "Close memory-heavy applications or select a smaller reader.",
                "Preview verified repair for missing or corrupt pinned assets.",
                "Choose a compatible CPU, CUDA or Vulkan runtime.",
            ],
        ),
        "NG201" => (
            "The reader returned an incomplete answer",
            &["Inspect the source and preserved alternative readings."],
            &[
                "Re-read the uncertain region or transcribe the visible content manually.",
                "Do not approve an uncertain number solely because readers agree.",
            ],
        ),
        "NG203" => (
            "Recognition endpoint is not allowed",
            &["Check that the endpoint uses a literal loopback address."],
            &["Use the local runtime endpoint shown by NoteGen."],
        ),
        "NG400" | "NG401" => (
            "The output cannot be saved",
            &[
                "Check free disk space and folder access.",
                "Check whether Word has the destination open or a file already exists.",
            ],
            &[
                "Choose a new filename in a writable local folder.",
                "Keep the project checkpoints and retry export.",
            ],
        ),
        "NG500" => (
            "Project state changed or recovery is required",
            &[
                "Inspect the recovery dashboard and revision timeline.",
                "Check that the original inputs and pinned model revisions remain available.",
            ],
            &[
                "Reload current project state before making another edit.",
                "Preserve backups and resume matching checkpoints.",
            ],
        ),
        "NG610" | "NG620" | "NG630" => (
            "A handwriting example or correction needs review",
            &[
                "Check the writer profile and the selected source region.",
                "Confirm the visible text before adding an example.",
            ],
            &[
                "Quarantine questionable examples and keep confirmed corrections.",
                "Collect missing glyphs before exporting a font.",
            ],
        ),
        "NG640" => (
            "Optional public lookup is unavailable",
            &["Check the explicitly typed query and lookup setting."],
            &["Continue offline or retry the optional lookup later."],
        ),
        "NG650" => (
            "A verified asset operation failed",
            &[
                "Check disk space, connection and the exact publisher revision.",
                "Inspect the repair preview for missing or corrupt pinned files.",
            ],
            &[
                "Retry the verified download; partial files may resume.",
                "Retain the active registration and rollback version until verification succeeds.",
            ],
        ),
        "NG710" => (
            "Evaluation inputs need attention",
            &[
                "Check corpus permissions, unique sample IDs and writer-separated splits.",
                "Check prediction coverage and finite uncertainty values.",
            ],
            &["Correct the manifest or divide an oversized sample before comparing."],
        ),
        "NG720" => (
            "The resource preview is stale or invalid",
            &["Check reserved cores, available memory and whether settings changed."],
            &["Create a fresh preview and leave at least one worker and enough RAM."],
        ),
        "NG730" => (
            "Support or repair preview needs attention",
            &["Check selected file availability, size limits and preview fingerprints."],
            &[
                "Inspect a new preview after any file or selection change.",
                "Choose fewer attachments or a new output filename.",
            ],
        ),
        "NG900" => (
            "Operation was cancelled",
            &["Inspect completed stages and preserved checkpoints."],
            &[
                "Resume with matching inputs/settings or create a fresh preview for an interrupted export.",
            ],
        ),
        _ => (
            "An operation needs attention",
            &["Note the stage and error code without sharing source content."],
            &["Inspect local recovery information or export a sanitized support bundle."],
        ),
    };
    Troubleshooting {
        code: if code.len() == 5
            && code.starts_with("NG")
            && code.as_bytes()[2..].iter().all(u8::is_ascii_digit)
        {
            code.into()
        } else {
            "unknown".into()
        },
        title: title.into(),
        checks: checks.iter().map(|s| s.to_string()).collect(),
        actions: actions.iter().map(|s| s.to_string()).collect(),
    }
}

pub struct RepairInput<'a> {
    pub settings: &'a Settings,
    pub model_ids: &'a [String],
    pub runtime_backend: Option<&'a str>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetHealth {
    Healthy,
    Missing,
    Corrupt,
    Unreadable,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairFile {
    pub key: String,
    pub expected_bytes: u64,
    pub expected_sha256: String,
    pub current_bytes: Option<u64>,
    pub current_sha256: Option<String>,
    pub health: AssetHealth,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairPreview {
    pub schema: u32,
    pub files: Vec<RepairFile>,
    pub notes: Vec<String>,
    pub token: String,
}
struct RepairAsset {
    path: PathBuf,
    download: setup::Download,
}
fn plain_path(path: &Path) -> Result<()> {
    let absolute = std::path::absolute(path)?;
    for part in absolute.ancestors() {
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
                    Fault::new("NG730", "Repair paths cannot contain symlinks or junctions")
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(support_fault("Repair path metadata is unavailable")),
        }
    }
    Ok(())
}
fn repair_assets(input: &RepairInput<'_>) -> Result<BTreeMap<String, RepairAsset>> {
    input.settings.validate()?;
    let ids = input.model_ids.iter().collect::<BTreeSet<_>>();
    anyhow::ensure!(
        ids.len() == input.model_ids.len() && ids.len() <= 100,
        Fault::new(
            "NG730",
            "Choose at most 100 unique compiled-pinned reader IDs"
        )
    );
    let pins: Vec<Value> = serde_json::from_str(include_str!("../downloads.json"))?;
    let mut assets = BTreeMap::new();
    for id in ids {
        let (_, update) = setup::pinned(id)?;
        let pin = pins
            .iter()
            .find(|p| p["id"].as_str() == Some(id.as_str()))
            .ok_or_else(|| support_fault("Reader pins are unavailable"))?;
        for (role, file) in [("weights", "weights.gguf"), ("projector", "projector.gguf")] {
            let download: setup::Download = serde_json::from_value(pin[role].clone())?;
            assets.insert(
                format!("model/{id}/{role}"),
                RepairAsset {
                    path: std::path::absolute(&input.settings.models_dir)?
                        .join("versions")
                        .join(id)
                        .join(&update.revision)
                        .join(file),
                    download,
                },
            );
        }
    }
    if let Some(backend) = input.runtime_backend {
        let marker = match backend {
            "cpu" => "win-cpu-x64",
            "vulkan" => "win-vulkan-x64",
            "cuda12" => "win-cuda-12.4-x64",
            "cuda13" => "win-cuda-13.4-x64",
            _ => {
                return Err(support_fault(
                    "Choose cpu, vulkan, cuda12 or cuda13 runtime archives",
                ));
            }
        };
        let downloads: Vec<setup::Download> =
            serde_json::from_str(include_str!("../runtime-downloads.json"))?;
        for download in downloads.into_iter().filter(|d| d.file.contains(marker)) {
            assets.insert(
                format!("runtime-archive/{}", download.file),
                RepairAsset {
                    path: std::path::absolute(&setup::runtime_root(input.settings))?
                        .join("downloads")
                        .join(&download.file),
                    download,
                },
            );
        }
    }
    anyhow::ensure!(
        !assets.is_empty(),
        Fault::new(
            "NG730",
            "Choose readers or a runtime archive backend to inspect"
        )
    );
    for asset in assets.values() {
        plain_path(&asset.path)?;
    }
    Ok(assets)
}
pub fn repair_preview(input: &RepairInput<'_>, c: &Control) -> Result<RepairPreview> {
    c.check()?;
    let assets = repair_assets(input)?;
    let mut files = vec![];
    for (key, asset) in &assets {
        c.check()?;
        let (current_bytes, current_sha256, health) = match fs::metadata(&asset.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (None, None, AssetHealth::Missing)
            }
            Err(_) => (None, None, AssetHealth::Unreadable),
            Ok(meta) if !meta.is_file() => (Some(meta.len()), None, AssetHealth::Unreadable),
            Ok(meta) => match hash_file(&asset.path, c) {
                Ok(hash) => {
                    let healthy = meta.len() == asset.download.bytes
                        && hash.eq_ignore_ascii_case(&asset.download.sha256);
                    (
                        Some(meta.len()),
                        Some(hash),
                        if healthy {
                            AssetHealth::Healthy
                        } else {
                            AssetHealth::Corrupt
                        },
                    )
                }
                Err(e) if e.downcast_ref::<Fault>().is_some_and(|f| f.code == "NG900") => {
                    return Err(e);
                }
                Err(_) => (Some(meta.len()), None, AssetHealth::Unreadable),
            },
        };
        files.push(RepairFile {
            key: key.clone(),
            expected_bytes: asset.download.bytes,
            expected_sha256: asset.download.sha256.clone(),
            current_bytes,
            current_sha256,
            health,
        });
    }
    let notes=vec!["Repairs only compiled-pinned model weight/projector files and verified runtime download archives. Custom readers and registrations are not rewritten.".into(),
        "Runtime archive repair does not activate or verify extracted runtime binaries; use verified runtime installation afterward.".into(),
        "A damaged file is retained beside a repaired file for recovery. Healthy selected files need no download.".into()];
    let binding = assets
        .iter()
        .map(|(key, a)| (key, a.path.to_string_lossy().to_string()))
        .collect::<Vec<_>>();
    let token = fingerprint(&(1, &files, &notes, binding))?;
    Ok(RepairPreview {
        schema: 1,
        files,
        notes,
        token,
    })
}
/// Downloads to a fresh staging directory and preserves registrations and damaged originals.
pub fn repair_selected(
    input: &RepairInput<'_>,
    expected: &RepairPreview,
    token: &str,
    selected: &[String],
    c: &Control,
    log: &Activity,
) -> Result<Vec<String>> {
    c.check()?;
    let current = repair_preview(input, c)?;
    anyhow::ensure!(
        token == expected.token && *expected == current,
        Fault::new(
            "NG730",
            "Repair files or preview changed; inspect a fresh preview"
        )
    );
    let unique = selected.iter().collect::<BTreeSet<_>>();
    anyhow::ensure!(
        !selected.is_empty()
            && unique.len() == selected.len()
            && selected
                .iter()
                .all(|k| current.files.iter().any(|f| &f.key == k)),
        Fault::new(
            "NG730",
            "Select unique assets from the inspected repair preview"
        )
    );
    let assets = repair_assets(input)?;
    let mut repaired = vec![];
    for key in selected {
        c.check()?;
        let info = current.files.iter().find(|f| &f.key == key).unwrap();
        if info.health == AssetHealth::Healthy {
            continue;
        }
        anyhow::ensure!(
            info.health != AssetHealth::Unreadable,
            Fault::new(
                "NG730",
                "An unreadable repair target needs manual storage or permission repair"
            )
        );
        let asset = &assets[key];
        plain_path(&asset.path)?;
        let parent = asset.path.parent().unwrap();
        fs::create_dir_all(parent)?;
        let staging = tempfile::Builder::new()
            .prefix(".notegen-repair-")
            .tempdir_in(parent)?;
        let staged = tempfile::Builder::new()
            .prefix("verified-")
            .tempfile_in(staging.path())?
            .into_temp_path();
        models::download(
            &asset.download.url,
            &staged,
            &models::Asset {
                file: asset.download.file.clone(),
                bytes: asset.download.bytes,
                sha256: asset.download.sha256.clone(),
            },
            c,
            log,
        )?;
        anyhow::ensure!(
            staged.metadata()?.len() == asset.download.bytes
                && hash_file(&staged, c)? == asset.download.sha256,
            Fault::new("NG730", "Downloaded repair failed publisher verification")
        );
        c.check()?;
        plain_path(&asset.path)?;
        // Recheck the target after a potentially long download before moving it.
        if let Some(hash) = &info.current_sha256 {
            anyhow::ensure!(
                asset.path.is_file() && hash_file(&asset.path, c)? == *hash,
                Fault::new("NG730", "Repair target changed during download")
            );
        } else {
            anyhow::ensure!(
                !asset.path.exists(),
                Fault::new("NG730", "Repair target appeared during download")
            );
        }
        if asset.path.exists() {
            let b = parent.join(format!(
                "{}.damaged-{}",
                asset.path.file_name().unwrap().to_string_lossy(),
                &info.current_sha256.as_ref().unwrap()[..12]
            ));
            anyhow::ensure!(
                !b.exists(),
                Fault::new(
                    "NG730",
                    "A damaged-file backup already exists; inspect it before another repair"
                )
            );
            let mut original = fs::File::open(&asset.path)?;
            let mut backup = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&b)?;
            std::io::copy(&mut original, &mut backup)?;
            backup.sync_all()?;
            anyhow::ensure!(
                hash_file(&b, c)? == *info.current_sha256.as_ref().unwrap(),
                Fault::new("NG730", "Damaged-file backup changed while copying")
            );
        }
        // Replacement is on the same filesystem; the original remains in place if rename fails.
        if info.health == AssetHealth::Missing {
            staged.persist_noclobber(&asset.path).map_err(|e| e.error)?;
        } else {
            staged.persist(&asset.path).map_err(|e| e.error)?;
        }
        repaired.push(key.clone());
        log.emit(
            "repair",
            format!("Repaired verified asset {key}; registrations preserved"),
            0,
            repaired.len(),
            selected.len(),
            "",
        );
    }
    Ok(repaired)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FullDisk(std::io::Cursor<Vec<u8>>);
    impl Write for FullDisk {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected disk-full write"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Seek for FullDisk {
        fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
            self.0.seek(p)
        }
    }
    #[test]
    fn support_writer_propagates_injected_disk_full_without_success() {
        let files = BTreeMap::from([("settings-summary.json".into(), b"{}".to_vec())]);
        assert!(
            write_support_zip(
                FullDisk(std::io::Cursor::new(vec![])),
                &files,
                &["settings-summary.json".into()],
                &Control::default()
            )
            .is_err()
        );
    }
}
