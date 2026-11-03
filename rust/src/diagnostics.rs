//! Inspectable support exports and repair of publisher-pinned local assets.
use crate::{models, setup, storage::*, types::*};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom, Write},
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
    runtime: Option<RuntimeMember>,
}
#[derive(Clone)]
struct RuntimeMember {
    archive: PathBuf,
    pin: setup::Download,
    index: usize,
    name: String,
}
struct RepairSnapshot {
    assets: BTreeMap<String, RepairAsset>,
    preview: RepairPreview,
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
fn runtime_key(name: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    name.nfkc()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect()
}
fn runtime_component(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    !name.is_empty()
        && name.len() <= 240
        && !name.ends_with(['.', ' '])
        && !name
            .chars()
            .any(|ch| ch.is_control() || "\\:<>\"|?*".contains(ch))
        && !["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        && !matches!(
            stem.strip_prefix("COM")
                .or_else(|| stem.strip_prefix("LPT")),
            Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        )
}
/// Reject large central directories/ZIP64 before the ZIP reader can allocate
/// metadata from an attacker-controlled entry count. Published x64 pins fit ZIP32.
fn runtime_zip_count(file: &mut fs::File) -> Result<usize> {
    let length = file.metadata()?.len();
    let tail_len = length.min(65_577) as usize;
    anyhow::ensure!(tail_len >= 22, support_fault("Truncated runtime ZIP"));
    file.seek(SeekFrom::End(-(tail_len as i64)))?;
    let mut tail = vec![0u8; tail_len];
    file.read_exact(&mut tail)?;
    let offset = (0..=tail.len() - 22)
        .rev()
        .find(|start| {
            &tail[*start..*start + 4] == b"PK\x05\x06"
                && *start
                    + 22
                    + usize::from(u16::from_le_bytes([tail[*start + 20], tail[*start + 21]]))
                    == tail.len()
        })
        .ok_or_else(|| support_fault("Runtime ZIP has no bounded end record"))?;
    let u16_at =
        |index: usize| u16::from_le_bytes([tail[offset + index], tail[offset + index + 1]]);
    let u32_at = |index: usize| {
        u32::from_le_bytes(
            tail[offset + index..offset + index + 4]
                .try_into()
                .expect("bounded ZIP footer"),
        )
    };
    let count = usize::from(u16_at(10));
    let directory_bytes = u64::from(u32_at(12));
    let directory_offset = u64::from(u32_at(16));
    anyhow::ensure!(
        u16_at(4) == 0
            && u16_at(6) == 0
            && usize::from(u16_at(8)) == count
            && (1..=1000).contains(&count)
            && directory_bytes <= 4_000_000
            && directory_offset
                .checked_add(directory_bytes)
                .is_some_and(|end| end <= length - tail_len as u64 + offset as u64)
            && !(offset >= 20 && &tail[offset - 20..offset - 16] == b"PK\x06\x07"),
        support_fault("Runtime ZIP metadata exceeds supported single-volume ZIP32 bounds")
    );
    file.seek(SeekFrom::Start(0))?;
    Ok(count)
}
fn verified_runtime_archive(
    path: &Path,
    pin: &setup::Download,
    c: &Control,
) -> Result<Option<fs::File>> {
    c.check()?;
    plain_path(path)?;
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Ok(None),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() != pin.bytes {
        return Ok(None);
    }
    file.try_lock_shared().map_err(|_| {
        support_fault(
            "Pinned runtime archive is busy; wait for its writer before inspecting binary repairs",
        )
    })?;
    let actual = match repair_hash(&mut file, pin.bytes, c) {
        Ok(hash) => hash,
        Err(error)
            if error
                .downcast_ref::<Fault>()
                .is_some_and(|fault| fault.code == "NG900") =>
        {
            return Err(error);
        }
        Err(_) => return Ok(None),
    };
    if !actual.eq_ignore_ascii_case(&pin.sha256) {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(Some(file))
}
fn managed_runtime_target(
    settings: &Settings,
    backend: &str,
    build: &str,
) -> Result<Option<PathBuf>> {
    let server = std::path::absolute(&settings.server)?;
    plain_path(&server)?;
    if !server
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("llama-server.exe"))
    {
        return Ok(None);
    }
    let Some(parent) = server.parent() else {
        return Ok(None);
    };
    let root = std::path::absolute(setup::runtime_root(settings))?.join("ai-backends");
    plain_path(&root)?;
    if !root.is_dir() || !parent.is_dir() {
        return Ok(None);
    }
    let parent = parent.canonicalize()?;
    let root = root.canonicalize()?;
    let prefix = format!("{backend}-{build}-");
    if parent.parent() != Some(root.as_path())
        || !parent
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.starts_with(&prefix) && value.len() > prefix.len())
    {
        return Ok(None);
    }
    Ok(Some(parent))
}
fn member_name(entry: &zip::read::ZipFile<'_, fs::File>) -> Result<Option<String>> {
    use std::path::Component;
    let path = entry
        .enclosed_name()
        .ok_or_else(|| support_fault("Runtime archive contains an escaping path"))?;
    anyhow::ensure!(
        !entry.is_symlink()
            && entry
                .unix_mode()
                .is_none_or(|mode| matches!(mode & 0o170000, 0 | 0o100000 | 0o040000)),
        support_fault("Runtime archive contains a link or special file")
    );
    anyhow::ensure!(path.as_os_str().len()<=4096&&path.components().all(|part|matches!(part,Component::Normal(value)if value.to_str().is_some_and(runtime_component))),support_fault("Runtime archive contains an unportable path"));
    if entry.is_dir() {
        return Ok(None);
    }
    anyhow::ensure!(
        entry
            .unix_mode()
            .is_none_or(|mode| mode & 0o170000 != 0o040000),
        support_fault("Runtime archive directory metadata is inconsistent")
    );
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| support_fault("Runtime archive filename is not Unicode"))?;
    let lower = name.to_ascii_lowercase();
    // setup copies these from the application AFTER archive extraction. Their
    // installed authority is the verified application bundle, not this ZIP.
    if [
        "vcruntime140.dll",
        "vcruntime140_1.dll",
        "msvcp140.dll",
        "concrt140.dll",
    ]
    .contains(&lower.as_str())
    {
        return Ok(None);
    }
    if !lower.ends_with(".dll") && lower != "llama-server.exe" {
        return Ok(None);
    }
    anyhow::ensure!(
        name.is_ascii() && runtime_component(name) && (1..=2_000_000_000).contains(&entry.size()),
        support_fault("Runtime binary name or size exceeds its supported bounds")
    );
    Ok(Some(name.into()))
}
fn runtime_binary_assets(
    settings: &Settings,
    backend: &str,
    build: &str,
    pins: &[setup::Download],
    c: &Control,
) -> Result<(BTreeMap<String, RepairAsset>, Vec<String>)> {
    let Some(target) = managed_runtime_target(settings, backend, build)? else {
        return Ok((
            BTreeMap::new(),
            vec![format!(
                "Extracted runtime inspection omitted: selected server is not in a matching managed {backend}-{build}-* folder. Custom/other-version binaries are never mixed with these pins."
            )],
        ));
    };
    let cache = std::path::absolute(setup::runtime_root(settings))?.join("downloads");
    plain_path(&cache)?;
    anyhow::ensure!(
        pins.len() == if backend.starts_with("cuda") { 2 } else { 1 },
        support_fault("Pinned runtime bundle is incomplete")
    );
    let mut sources = vec![];
    for pin in pins {
        let path = cache.join(&pin.file);
        let Some(file) = verified_runtime_archive(&path, pin, c)? else {
            return Ok((BTreeMap::new(),vec!["Extracted runtime hashes are unavailable until every source archive is publisher-verified. Repair the listed runtime archives first, then refresh the preview.".into()]));
        };
        sources.push((pin.clone(), path, file));
    }
    let mut members = BTreeMap::<String, RepairAsset>::new();
    let mut expanded = 0u64;
    for (pin, path, mut file) in sources {
        let count = runtime_zip_count(&mut file)?;
        let mut zip = zip::ZipArchive::new(file)?;
        anyhow::ensure!(
            zip.len() == count,
            support_fault("Runtime ZIP entry count changed")
        );
        for index in 0..zip.len() {
            c.check()?;
            let mut entry = zip.by_index(index)?;
            let Some(name) = member_name(&entry)? else {
                continue;
            };
            expanded = expanded
                .checked_add(entry.size())
                .ok_or_else(|| support_fault("Runtime expanded size overflow"))?;
            anyhow::ensure!(
                expanded <= 4_000_000_000,
                support_fault("Runtime bundle exceeds 4 GB binary extraction budget")
            );
            let bytes = entry.size();
            let raw_name = entry.name().to_owned();
            let sha256 = repair_hash(&mut entry, bytes, c)?;
            let key = runtime_key(&name);
            if let Some(previous) = members.get(&key) {
                anyhow::ensure!(
                    previous.download.file == name
                        && previous.download.bytes == bytes
                        && previous.download.sha256 == sha256,
                    support_fault(
                        "Runtime archive has conflicting flattened filenames or case aliases"
                    )
                );
                continue;
            }
            members.insert(
                key,
                RepairAsset {
                    path: target.join(&name),
                    download: setup::Download {
                        file: name,
                        bytes,
                        sha256,
                        url: pin.url.clone(),
                    },
                    runtime: Some(RuntimeMember {
                        archive: path.clone(),
                        pin: pin.clone(),
                        index,
                        name: raw_name,
                    }),
                },
            );
        }
        let mut file = zip.into_inner();
        file.seek(SeekFrom::Start(0))?;
        anyhow::ensure!(
            repair_hash(&mut file, pin.bytes, c)?.eq_ignore_ascii_case(&pin.sha256),
            support_fault("Runtime archive changed while deriving binary hashes")
        );
    }
    anyhow::ensure!(
        members.contains_key("llama-server.exe"),
        support_fault("Verified runtime bundle contains no llama-server.exe")
    );
    let mut existing = BTreeMap::<String, Vec<PathBuf>>::new();
    let mut count = 0usize;
    for entry in fs::read_dir(&target)? {
        c.check()?;
        count += 1;
        anyhow::ensure!(
            count <= 5000,
            support_fault("Managed runtime folder exceeds its inspection entry budget")
        );
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(support_fault(
                "Managed runtime folder contains non-Unicode names",
            ));
        };
        existing
            .entry(runtime_key(name))
            .or_default()
            .push(entry.path());
    }
    let mut result = BTreeMap::new();
    for (key, mut asset) in members {
        if let Some(paths) = existing.get(&key) {
            anyhow::ensure!(
                paths.len() == 1
                    && paths[0]
                        .file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|name| name.is_ascii()
                            && name.eq_ignore_ascii_case(&asset.download.file)),
                support_fault(
                    "Managed runtime contains ambiguous filename aliases; preserve them and resolve the conflict manually"
                )
            );
            asset.path = paths[0].clone();
        }
        plain_path(&asset.path)?;
        result.insert(
            format!(
                "runtime-file/{backend}/{}",
                asset.download.file.to_ascii_lowercase()
            ),
            asset,
        );
    }
    Ok((result,vec![format!("Extracted {build}/{backend} binary sizes/hashes were derived only from the complete verified publisher archive bundle. Close readers before repair; busy files are not forcibly unlocked. Active/previous pointers, registrations and unknown files stay untouched."),"App-local MSVC dependencies are installer-serviced; they are not inferred from llama.cpp archives.".into()]))
}
fn stage_runtime_member(
    member: &RuntimeMember,
    expected: &setup::Download,
    staged: &Path,
    c: &Control,
) -> Result<()> {
    let mut source = verified_runtime_archive(&member.archive, &member.pin, c)?
        .ok_or_else(|| support_fault("Runtime source archive is no longer publisher-verified"))?;
    let count = runtime_zip_count(&mut source)?;
    let mut zip = zip::ZipArchive::new(source)?;
    anyhow::ensure!(
        zip.len() == count,
        support_fault("Runtime ZIP entry count changed")
    );
    let mut entry = zip.by_index(member.index)?;
    anyhow::ensure!(
        entry.name() == member.name
            && member_name(&entry)?.as_deref() == Some(expected.file.as_str())
            && entry.size() == expected.bytes,
        support_fault("Runtime source member changed after preview")
    );
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(staged)?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        c.check()?;
        let count = entry.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        anyhow::ensure!(
            total <= expected.bytes,
            support_fault("Runtime member grew beyond its verified size")
        );
        hash.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
    }
    anyhow::ensure!(
        total == expected.bytes && hex::encode(hash.finalize()) == expected.sha256,
        support_fault("Staged runtime binary does not match publisher-derived bytes")
    );
    output.flush()?;
    output.sync_all()?;
    c.check()?;
    Ok(())
}
fn repair_assets(
    input: &RepairInput<'_>,
    c: &Control,
) -> Result<(BTreeMap<String, RepairAsset>, Vec<String>)> {
    let pins: Vec<setup::Download> =
        serde_json::from_str(include_str!("../runtime-downloads.json"))?;
    repair_assets_with_pins(input, &pins, "b12032", c)
}
fn repair_assets_with_pins(
    input: &RepairInput<'_>,
    runtime_pins: &[setup::Download],
    build: &str,
    c: &Control,
) -> Result<(BTreeMap<String, RepairAsset>, Vec<String>)> {
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
    let mut notes = vec![];
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
                    runtime: None,
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
        crate::maintenance::validate_runtime_release(build, runtime_pins)?;
        let selected = runtime_pins
            .iter()
            .filter(|d| d.file.contains(marker))
            .cloned()
            .collect::<Vec<_>>();
        anyhow::ensure!(
            !selected.is_empty(),
            support_fault("Pinned runtime archive selection is empty")
        );
        for download in &selected {
            assets.insert(
                format!("runtime-archive/{}", download.file),
                RepairAsset {
                    path: std::path::absolute(setup::runtime_root(input.settings))?
                        .join("downloads")
                        .join(&download.file),
                    download: download.clone(),
                    runtime: None,
                },
            );
        }
        let (binary_assets, binary_notes) =
            runtime_binary_assets(input.settings, backend, build, &selected, c)?;
        assets.extend(binary_assets);
        notes.extend(binary_notes);
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
    Ok((assets, notes))
}
pub fn repair_preview(input: &RepairInput<'_>, c: &Control) -> Result<RepairPreview> {
    Ok(repair_snapshot(input, c)?.preview)
}
fn repair_snapshot(input: &RepairInput<'_>, c: &Control) -> Result<RepairSnapshot> {
    let (assets, notes) = repair_assets(input, c)?;
    repair_snapshot_from_assets(assets, notes, c)
}
fn repair_snapshot_from_assets(
    assets: BTreeMap<String, RepairAsset>,
    extra_notes: Vec<String>,
    c: &Control,
) -> Result<RepairSnapshot> {
    c.check()?;
    let mut files = vec![];
    for (key, asset) in &assets {
        c.check()?;
        let (current_bytes, current_sha256, health) = match fs::metadata(&asset.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (None, None, AssetHealth::Missing)
            }
            Err(_) => (None, None, AssetHealth::Unreadable),
            Ok(meta)
                if !meta.is_file()
                    || (key.starts_with("runtime-") && meta.len() > 2_000_000_000) =>
            {
                (Some(meta.len()), None, AssetHealth::Unreadable)
            }
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
    let mut notes=vec!["Repairs compiled-pinned model assets, runtime download archives and extracted runtime binaries whose exact size/SHA was derived from a fully verified publisher archive. Custom readers and registrations are not rewritten.".into(),
        "Extracted binaries appear only for the matching managed pinned build/backend and verified cached source archives. Missing/corrupt source archives must be repaired first, then preview refreshed. No preview downloads or runtime execution.".into(),
        "A damaged file is retained in a uniquely named adjacent .notegen-repair-recovery-* directory. Healthy selected files need no download.".into()];
    notes.extend(extra_notes);
    let binding = assets
        .iter()
        .map(|(key, a)| (key, a.path.to_string_lossy().to_string()))
        .collect::<Vec<_>>();
    let token = fingerprint(&(1, &files, &notes, binding))?;
    let preview = RepairPreview {
        schema: 1,
        files,
        notes,
        token,
    };
    Ok(RepairSnapshot { assets, preview })
}
fn repair_hash(reader: impl Read, expected_bytes: u64, c: &Control) -> Result<String> {
    let mut reader = reader.take(expected_bytes.saturating_add(1));
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut bytes = 0u64;
    loop {
        c.check()?;
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes = bytes.saturating_add(count as u64);
        anyhow::ensure!(
            bytes <= expected_bytes,
            Fault::new("NG730", "Captured repair target grew after preview")
        );
        hash.update(&buffer[..count]);
    }
    anyhow::ensure!(
        bytes == expected_bytes,
        Fault::new("NG730", "Captured repair target shrank after preview")
    );
    Ok(hex::encode(hash.finalize()))
}
/// Owns the exact directory entry removed from a repair target. The recovery directory is
/// persistent before the rename: cancellation, errors and unwinding must never delete it.
struct CapturedRepair {
    target: PathBuf,
    original: PathBuf,
    directory: PathBuf,
    restore: bool,
}
impl CapturedRepair {
    fn capture(target: &Path) -> Result<Self> {
        let filename = target
            .file_name()
            .ok_or_else(|| Fault::new("NG730", "Repair target has no filename"))?;
        let parent = target
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let directory = tempfile::Builder::new()
            .prefix(".notegen-repair-recovery-")
            .tempdir_in(parent)?
            .keep();
        let original = directory.join(filename);
        if let Err(error) = fs::rename(target, &original) {
            let _ = fs::remove_dir(&directory);
            return Err(error.into());
        }
        Ok(Self {
            target: target.into(),
            original,
            directory,
            restore: true,
        })
    }
    fn verify(&self, expected: &RepairFile, c: &Control) -> Result<()> {
        c.check()?;
        plain_path(&self.original)?;
        let metadata = fs::metadata(&self.original)?;
        anyhow::ensure!(
            metadata.is_file() && Some(metadata.len()) == expected.current_bytes,
            Fault::new("NG730", "Captured repair target changed after preview")
        );
        let hash = repair_hash(fs::File::open(&self.original)?, metadata.len(), c)?;
        anyhow::ensure!(
            expected.current_sha256.as_ref() == Some(&hash),
            Fault::new("NG730", "Captured repair target changed after preview")
        );
        c.check()?;
        Ok(())
    }
    fn restore(&mut self) -> std::io::Result<()> {
        if !self.restore {
            return Ok(());
        }
        let mut original = tempfile::TempPath::try_from_path(&self.original)?;
        // An unsuccessful no-clobber restore must leave the captured original in place.
        original.disable_cleanup(true);
        original
            .persist_noclobber(&self.target)
            .map_err(|error| error.error)?;
        self.restore = false;
        let _ = fs::remove_dir(&self.directory);
        Ok(())
    }
    fn publish(
        mut self,
        staged: tempfile::TempPath,
        expected: &RepairFile,
        c: &Control,
    ) -> Result<PathBuf> {
        let result: Result<PathBuf> = (|| {
            self.verify(expected, c)?;
            c.check()?;
            plain_path(&self.target)?;
            // The captured entry, rather than a pathname checked before a long copy,
            // is the backup. A newly created target always wins over this repair.
            staged
                .persist_noclobber(&self.target)
                .map_err(|error| error.error)?;
            self.restore = false;
            Ok(self.original.clone())
        })();
        match result {
            Ok(backup) => Ok(backup),
            Err(error) => match self.restore() {
                Ok(()) => Err(error),
                Err(restore_error) => {
                    self.restore = false;
                    Err(error.context(format!(
                        "Repair did not replace the current target; captured original retained at {} (restore failed: {restore_error})",
                        self.original.display()
                    )))
                }
            },
        }
    }
}
impl Drop for CapturedRepair {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
pub(crate) fn publish_repair(
    staged: tempfile::TempPath,
    target: &Path,
    expected: &RepairFile,
    c: &Control,
) -> Result<Option<PathBuf>> {
    c.check()?;
    plain_path(target)?;
    if expected.health == AssetHealth::Missing {
        staged
            .persist_noclobber(target)
            .map_err(|error| error.error)?;
        return Ok(None);
    }
    CapturedRepair::capture(target)?
        .publish(staged, expected, c)
        .map(Some)
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
    repair_from_snapshot(
        repair_snapshot(input, c)?,
        expected,
        token,
        selected,
        c,
        log,
    )
}
fn repair_from_snapshot(
    snapshot: RepairSnapshot,
    expected: &RepairPreview,
    token: &str,
    selected: &[String],
    c: &Control,
    log: &Activity,
) -> Result<Vec<String>> {
    c.check()?;
    let current = snapshot.preview;
    let assets = snapshot.assets;
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
        let staged = tempfile::TempPath::try_from_path(staging.path().join("verified-asset"))?;
        if let Some(member) = &asset.runtime {
            stage_runtime_member(member, &asset.download, &staged, c)?;
        } else {
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
        }
        anyhow::ensure!(
            staged.metadata()?.len() == asset.download.bytes
                && hash_file(&staged, c)? == asset.download.sha256,
            Fault::new("NG730", "Staged repair failed publisher verification")
        );
        let backup = publish_repair(staged, &asset.path, info, c)?;
        repaired.push(key.clone());
        let message = if let Some(path) = backup {
            format!(
                "Repaired verified asset {key}; registrations preserved; damaged original retained at {}",
                path.display()
            )
        } else {
            format!("Repaired verified asset {key}; registrations preserved")
        };
        log.emit("repair", message, 0, repaired.len(), selected.len(), "");
    }
    Ok(repaired)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn synthetic_runtime(
        root: &Path,
        entries: &[(&str, &[u8])],
    ) -> (Settings, Vec<setup::Download>) {
        let server = root.join("runtime/ai-backends/cpu-b1-synthetic/llama-server.exe");
        fs::create_dir_all(server.parent().unwrap()).unwrap();
        let archive = root.join("runtime/downloads/llama-b1-bin-win-cpu-x64.zip");
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.set_comment("bounded deterministic synthetic fixture ".repeat(45));
        for (name, bytes) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().sync_all().unwrap();
        let bytes = fs::read(&archive).unwrap();
        let pin=setup::Download{file:"llama-b1-bin-win-cpu-x64.zip".into(),bytes:bytes.len()as u64,sha256:digest(&bytes),url:"https://github.com/ggml-org/llama.cpp/releases/download/b1/llama-b1-bin-win-cpu-x64.zip".into()};
        (
            Settings {
                server,
                models_dir: root.join("models"),
                ..Default::default()
            },
            vec![pin],
        )
    }
    fn synthetic_snapshot(
        settings: &Settings,
        pins: &[setup::Download],
        c: &Control,
    ) -> Result<RepairSnapshot> {
        let input = RepairInput {
            settings,
            model_ids: &[],
            runtime_backend: Some("cpu"),
        };
        let (assets, notes) = repair_assets_with_pins(&input, pins, "b1", c)?;
        repair_snapshot_from_assets(assets, notes, c)
    }
    #[test]
    fn extracted_runtime_repair_uses_publisher_derived_bytes_and_preserves_every_unselected_file() {
        let root = tempfile::tempdir().unwrap();
        let (settings, pins) = synthetic_runtime(
            root.path(),
            &[
                ("bin/llama-server.exe", b"publisher server"),
                ("bin/ggml.dll", b"publisher ggml"),
                (
                    "ignored/vcruntime140.dll",
                    b"archive MSVC must not override app",
                ),
            ],
        );
        let folder = settings.server.parent().unwrap();
        fs::write(&settings.server, b"unselected current server").unwrap();
        fs::write(folder.join("ggml.dll"), b"damaged original").unwrap();
        fs::write(folder.join("vcruntime140.dll"), b"APP SERVICED").unwrap();
        fs::write(folder.join("manual.txt"), b"UNKNOWN RETAINED").unwrap();
        fs::create_dir_all(&settings.models_dir).unwrap();
        let registration = settings.models_dir.join("reader.notegen.json");
        fs::write(&registration, b"REGISTRATION UNCHANGED").unwrap();
        let active = root.path().join("runtime/active-ai-runtime.json");
        let previous = root.path().join("runtime/previous-ai-runtime.json");
        fs::write(&active, b"ACTIVE POINTER").unwrap();
        fs::write(&previous, b"PREVIOUS POINTER").unwrap();
        let snapshot = synthetic_snapshot(&settings, &pins, &Control::default()).unwrap();
        let expected = snapshot.preview.clone();
        let file = expected
            .files
            .iter()
            .find(|file| file.key == "runtime-file/cpu/ggml.dll")
            .unwrap();
        assert_eq!(file.expected_bytes, 14);
        assert_eq!(file.expected_sha256, digest(b"publisher ggml"));
        assert_eq!(file.health, AssetHealth::Corrupt);
        assert!(
            !expected
                .files
                .iter()
                .any(|file| file.key.ends_with("vcruntime140.dll"))
        );
        assert!(
            !serde_json::to_string(&expected)
                .unwrap()
                .contains(&root.path().to_string_lossy().to_string())
        );
        let repaired = repair_from_snapshot(
            snapshot,
            &expected,
            &expected.token,
            &["runtime-file/cpu/ggml.dll".into()],
            &Control::default(),
            &Activity::default(),
        )
        .unwrap();
        assert_eq!(repaired, vec!["runtime-file/cpu/ggml.dll"]);
        assert_eq!(
            fs::read(folder.join("ggml.dll")).unwrap(),
            b"publisher ggml"
        );
        assert_eq!(
            fs::read(&settings.server).unwrap(),
            b"unselected current server"
        );
        assert_eq!(
            fs::read(folder.join("vcruntime140.dll")).unwrap(),
            b"APP SERVICED"
        );
        assert_eq!(
            fs::read(folder.join("manual.txt")).unwrap(),
            b"UNKNOWN RETAINED"
        );
        assert_eq!(fs::read(&registration).unwrap(), b"REGISTRATION UNCHANGED");
        assert_eq!(fs::read(active).unwrap(), b"ACTIVE POINTER");
        assert_eq!(fs::read(previous).unwrap(), b"PREVIOUS POINTER");
        let backups = fs::read_dir(folder)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".notegen-repair-recovery-")
            })
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            fs::read(backups[0].path().join("ggml.dll")).unwrap(),
            b"damaged original"
        );
    }
    #[test]
    fn runtime_inventory_never_trusts_a_local_manifest_or_unverified_archive_and_never_fetches() {
        let root = tempfile::tempdir().unwrap();
        let (settings, pins) = synthetic_runtime(
            root.path(),
            &[
                ("llama-server.exe", b"verified server"),
                ("ggml.dll", b"verified ggml"),
            ],
        );
        let archive = root.path().join("runtime/downloads").join(&pins[0].file);
        fs::write(&archive, b"not the pinned archive").unwrap();
        fs::write(
            settings
                .server
                .parent()
                .unwrap()
                .join("invented-runtime-manifest.json"),
            br#"{"ggml.dll":{"bytes":7,"sha256":"fake"}}"#,
        )
        .unwrap();
        let snapshot = synthetic_snapshot(&settings, &pins, &Control::default()).unwrap();
        assert!(
            snapshot
                .preview
                .files
                .iter()
                .all(|file| file.key.starts_with("runtime-archive/"))
        );
        assert!(
            snapshot
                .preview
                .notes
                .iter()
                .any(|note| note.contains("hashes are unavailable"))
        );
        assert_eq!(fs::read(archive).unwrap(), b"not the pinned archive");
        assert!(!settings.server.is_file());
    }
    #[test]
    fn identical_flat_duplicates_deduplicate_but_conflicts_aliases_and_traversal_abort() {
        let root = tempfile::tempdir().unwrap();
        let (settings, pins) = synthetic_runtime(
            root.path(),
            &[
                ("bin/llama-server.exe", b"server"),
                ("first/ggml.dll", b"same"),
                ("second/ggml.dll", b"same"),
            ],
        );
        let snapshot = synthetic_snapshot(&settings, &pins, &Control::default()).unwrap();
        assert_eq!(
            snapshot
                .preview
                .files
                .iter()
                .filter(|file| file.key == "runtime-file/cpu/ggml.dll")
                .count(),
            1
        );
        for entries in [
            vec![
                ("llama-server.exe", b"server".as_slice()),
                ("a/ggml.dll", b"one".as_slice()),
                ("b/ggml.dll", b"two".as_slice()),
            ],
            vec![
                ("llama-server.exe", b"server".as_slice()),
                ("ggml.dll", b"same".as_slice()),
                ("GGML.DLL", b"same".as_slice()),
            ],
            vec![
                ("llama-server.exe", b"server".as_slice()),
                ("../escape.dll", b"evil".as_slice()),
            ],
            vec![
                ("llama-server.exe", b"server".as_slice()),
                ("nested/CON.dll", b"evil".as_slice()),
            ],
        ] {
            let root = tempfile::tempdir().unwrap();
            let (settings, pins) = synthetic_runtime(root.path(), &entries);
            assert!(synthetic_snapshot(&settings, &pins, &Control::default()).is_err());
            assert!(!root.path().join("escape.dll").exists());
            assert!(!settings.server.is_file());
        }
    }
    #[test]
    fn runtime_source_tamper_and_stale_target_cannot_publish_missing_or_corrupt_binaries() {
        let root = tempfile::tempdir().unwrap();
        let (settings, pins) = synthetic_runtime(
            root.path(),
            &[
                ("llama-server.exe", b"verified server"),
                ("ggml.dll", b"verified ggml"),
            ],
        );
        let folder = settings.server.parent().unwrap();
        fs::write(folder.join("ggml.dll"), b"damaged").unwrap();
        let snapshot = synthetic_snapshot(&settings, &pins, &Control::default()).unwrap();
        let expected = snapshot.preview.clone();
        // The fresh preview still matches, but a concurrently replaced archive is
        // reauthenticated through its held file before any target is captured.
        fs::write(
            root.path().join("runtime/downloads").join(&pins[0].file),
            b"replaced",
        )
        .unwrap();
        assert!(
            repair_from_snapshot(
                snapshot,
                &expected,
                &expected.token,
                &["runtime-file/cpu/ggml.dll".into()],
                &Control::default(),
                &Activity::default()
            )
            .is_err()
        );
        assert_eq!(fs::read(folder.join("ggml.dll")).unwrap(), b"damaged");
        let (settings, pins) = synthetic_runtime(
            root.path(),
            &[
                ("llama-server.exe", b"verified server"),
                ("ggml.dll", b"verified ggml"),
            ],
        );
        let snapshot = synthetic_snapshot(&settings, &pins, &Control::default()).unwrap();
        let expected = snapshot.preview.clone();
        fs::write(folder.join("ggml.dll"), b"concurrent replacement").unwrap();
        assert!(
            repair_from_snapshot(
                snapshot,
                &expected,
                &expected.token,
                &["runtime-file/cpu/ggml.dll".into()],
                &Control::default(),
                &Activity::default()
            )
            .is_err()
        );
        assert_eq!(
            fs::read(folder.join("ggml.dll")).unwrap(),
            b"concurrent replacement"
        );
        let snapshot = synthetic_snapshot(&settings, &pins, &Control::default()).unwrap();
        let expected = snapshot.preview.clone();
        let cancelled = Control::default();
        cancelled.cancelled.store(true, Ordering::Relaxed);
        assert!(
            repair_from_snapshot(
                snapshot,
                &expected,
                &expected.token,
                &["runtime-file/cpu/llama-server.exe".into()],
                &cancelled,
                &Activity::default()
            )
            .is_err()
        );
        assert!(!settings.server.exists());
    }
    #[test]
    fn runtime_zip_metadata_budgets_reject_oversized_directory_count_before_parsing() {
        let root = tempfile::tempdir().unwrap();
        let (settings, _) = synthetic_runtime(root.path(), &[("llama-server.exe", b"server")]);
        let archive = root
            .path()
            .join("runtime/downloads/llama-b1-bin-win-cpu-x64.zip");
        let mut bytes = fs::read(&archive).unwrap();
        let end = bytes
            .windows(4)
            .rposition(|part| part == b"PK\x05\x06")
            .unwrap();
        bytes[end + 8..end + 10].copy_from_slice(&1001u16.to_le_bytes());
        bytes[end + 10..end + 12].copy_from_slice(&1001u16.to_le_bytes());
        fs::write(&archive, &bytes).unwrap();
        assert!(runtime_zip_count(&mut fs::File::open(archive).unwrap()).is_err());
        assert!(!settings.server.exists());
    }
    #[test]
    fn runtime_symlink_and_unicode_target_aliases_never_become_repair_authority() {
        let root = tempfile::tempdir().unwrap();
        let (settings, mut pins) = synthetic_runtime(
            root.path(),
            &[("llama-server.exe", b"server"), ("ggml.dll", b"verified")],
        );
        let archive = root.path().join("runtime/downloads").join(&pins[0].file);
        let mut zip = zip::ZipWriter::new_append(
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&archive)
                .unwrap(),
        )
        .unwrap();
        zip.set_comment("bounded synthetic symlink fixture ".repeat(45));
        zip.add_symlink(
            "link.dll",
            "outside",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.finish().unwrap();
        let bytes = fs::read(&archive).unwrap();
        pins[0].bytes = bytes.len() as u64;
        pins[0].sha256 = digest(&bytes);
        assert!(synthetic_snapshot(&settings, &pins, &Control::default()).is_err());
        assert!(!settings.server.exists());
        let (settings, pins) = synthetic_runtime(
            root.path(),
            &[("llama-server.exe", b"server"), ("ggml.dll", b"verified")],
        );
        let alias = settings.server.parent().unwrap().join("ｇｇｍｌ.dll");
        fs::write(&alias, b"manual Unicode alias").unwrap();
        assert!(synthetic_snapshot(&settings, &pins, &Control::default()).is_err());
        assert_eq!(fs::read(alias).unwrap(), b"manual Unicode alias");
    }
    #[test]
    fn selected_missing_runtime_binary_is_published_without_executing_it_or_changing_other_paths() {
        let root = tempfile::tempdir().unwrap();
        let (settings, pins) = synthetic_runtime(
            root.path(),
            &[("llama-server.exe", b"synthetic verified server bytes")],
        );
        let snapshot = synthetic_snapshot(&settings, &pins, &Control::default()).unwrap();
        let expected = snapshot.preview.clone();
        assert_eq!(
            expected
                .files
                .iter()
                .find(|file| file.key == "runtime-file/cpu/llama-server.exe")
                .unwrap()
                .health,
            AssetHealth::Missing
        );
        repair_from_snapshot(
            snapshot,
            &expected,
            &expected.token,
            &["runtime-file/cpu/llama-server.exe".into()],
            &Control::default(),
            &Activity::default(),
        )
        .unwrap();
        assert_eq!(
            fs::read(&settings.server).unwrap(),
            b"synthetic verified server bytes"
        );
        assert!(!root.path().join("runtime/active-ai-runtime.json").exists());
        assert!(
            !root
                .path()
                .join("runtime/previous-ai-runtime.json")
                .exists()
        );
    }

    fn repair_file(original: &[u8]) -> RepairFile {
        RepairFile {
            key: "test/asset".into(),
            expected_bytes: 8,
            expected_sha256: digest(b"verified"),
            current_bytes: Some(original.len() as u64),
            current_sha256: Some(digest(original)),
            health: AssetHealth::Corrupt,
        }
    }
    fn staged_repair(root: &Path) -> tempfile::TempPath {
        let mut file = tempfile::NamedTempFile::new_in(root).unwrap();
        file.write_all(b"verified").unwrap();
        file.as_file().sync_all().unwrap();
        file.into_temp_path()
    }
    #[test]
    fn repair_publish_retains_exact_original_and_commits_verified_file() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("weights.gguf");
        fs::write(&target, b"damaged").unwrap();
        let backup = publish_repair(
            staged_repair(root.path()),
            &target,
            &repair_file(b"damaged"),
            &Control::default(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"verified");
        assert_eq!(fs::read(&backup).unwrap(), b"damaged");
        assert_eq!(backup.parent().unwrap().parent(), Some(root.path()));
    }
    #[test]
    fn repair_rejects_replacement_before_capture_and_restores_its_exact_bytes() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("weights.gguf");
        fs::write(&target, b"damaged").unwrap();
        let expected = repair_file(b"damaged");
        let mut concurrent = tempfile::NamedTempFile::new_in(root.path()).unwrap();
        concurrent.write_all(b"changed").unwrap();
        concurrent.persist(&target).unwrap();
        assert!(
            publish_repair(
                staged_repair(root.path()),
                &target,
                &expected,
                &Control::default()
            )
            .is_err()
        );
        assert_eq!(fs::read(&target).unwrap(), b"changed");
    }
    #[test]
    fn repair_never_overwrites_a_target_created_after_capture() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("weights.gguf");
        fs::write(&target, b"damaged").unwrap();
        let captured = CapturedRepair::capture(&target).unwrap();
        let backup = captured.original.clone();
        fs::write(&target, b"concurrent replacement").unwrap();
        let error = captured
            .publish(
                staged_repair(root.path()),
                &repair_file(b"damaged"),
                &Control::default(),
            )
            .unwrap_err();
        assert!(error.to_string().contains(&backup.display().to_string()));
        assert_eq!(fs::read(&target).unwrap(), b"concurrent replacement");
        assert_eq!(fs::read(&backup).unwrap(), b"damaged");
    }
    #[test]
    fn repair_cancellation_after_capture_restores_without_consulting_cancelled_control() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("weights.gguf");
        fs::write(&target, b"damaged").unwrap();
        let captured = CapturedRepair::capture(&target).unwrap();
        let directory = captured.directory.clone();
        let c = Control::default();
        c.cancelled.store(true, Ordering::Relaxed);
        let error = captured
            .publish(staged_repair(root.path()), &repair_file(b"damaged"), &c)
            .unwrap_err();
        assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG900");
        assert_eq!(fs::read(&target).unwrap(), b"damaged");
        assert!(!directory.exists());
    }
    #[test]
    fn repair_cancelled_after_capture_keeps_concurrent_target_and_recovery_original() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("weights.gguf");
        fs::write(&target, b"damaged").unwrap();
        let captured = CapturedRepair::capture(&target).unwrap();
        let backup = captured.original.clone();
        fs::write(&target, b"concurrent replacement").unwrap();
        let c = Control::default();
        c.cancelled.store(true, Ordering::Relaxed);
        let error = captured
            .publish(staged_repair(root.path()), &repair_file(b"damaged"), &c)
            .unwrap_err();
        assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG900");
        assert!(error.to_string().contains(&backup.display().to_string()));
        assert_eq!(fs::read(&target).unwrap(), b"concurrent replacement");
        assert_eq!(fs::read(&backup).unwrap(), b"damaged");
    }
    #[test]
    fn repair_unwind_keeps_original_even_when_the_target_is_recreated() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("weights.gguf");
        fs::write(&target, b"damaged").unwrap();
        let captured = CapturedRepair::capture(&target).unwrap();
        let backup = captured.original.clone();
        let unwind = std::panic::catch_unwind(move || {
            let captured_guard = captured;
            fs::write(&captured_guard.target, b"concurrent replacement").unwrap();
            panic!("injected failure after capture");
        });
        assert!(unwind.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"concurrent replacement");
        assert_eq!(fs::read(&backup).unwrap(), b"damaged");
    }
    #[test]
    fn missing_repair_never_overwrites_an_appearing_target() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("weights.gguf");
        let expected = RepairFile {
            current_bytes: None,
            current_sha256: None,
            health: AssetHealth::Missing,
            ..repair_file(b"")
        };
        fs::write(&target, b"appeared").unwrap();
        assert!(
            publish_repair(
                staged_repair(root.path()),
                &target,
                &expected,
                &Control::default()
            )
            .is_err()
        );
        assert_eq!(fs::read(&target).unwrap(), b"appeared");
    }
    #[test]
    fn repair_hash_bounds_growing_readers_and_preserves_cancellation() {
        let c = Control::default();
        let error = repair_hash(std::io::repeat(b'x'), 17, &c).unwrap_err();
        assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG730");
        assert_eq!(
            repair_hash(std::io::Cursor::new(b"known"), 5, &c).unwrap(),
            digest(b"known")
        );
        c.cancelled.store(true, Ordering::Relaxed);
        let error = repair_hash(std::io::repeat(b'x'), u64::MAX, &c).unwrap_err();
        assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG900");
    }
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
