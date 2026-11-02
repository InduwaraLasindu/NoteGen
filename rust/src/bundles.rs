//! Offline, content-verified AI transfer. Import never changes the active settings.
use crate::{
    models,
    planning::Model,
    setup,
    storage::{self, Control},
    types::*,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const MAX_BYTES: u64 = 200_000_000_000;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReaderEntry {
    pub model: Model,
    pub weights: String,
    pub projector: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeEntry {
    pub backend: String,
    pub build: String,
    pub archives: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Inventory {
    pub schema: u32,
    pub readers: Vec<ReaderEntry>,
    pub runtime: Option<RuntimeEntry>,
    pub files: Vec<FileEntry>,
    pub total_bytes: u64,
    pub contents_note: String,
}
fn check_path(path: &Path) -> Result<()> {
    let absolute = std::path::absolute(path)?;
    for part in absolute.ancestors() {
        let metadata = match fs::symlink_metadata(part) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        let linked = metadata.file_type().is_symlink();
        #[cfg(windows)]
        let linked = {
            use std::os::windows::fs::MetadataExt;
            linked || metadata.file_attributes() & 0x400 != 0
        };
        anyhow::ensure!(
            !linked,
            Fault::new(
                "NG650",
                "Offline bundle paths cannot contain symlinks or junctions"
            )
        );
    }
    Ok(())
}
fn runtime_pins(backend: &str) -> Result<Vec<setup::Download>> {
    let marker = match backend {
        "cpu" => "win-cpu-x64",
        "vulkan" => "win-vulkan-x64",
        "cuda12" => "win-cuda-12.4-x64",
        "cuda13" => "win-cuda-13.4-x64",
        _ => {
            return Err(Fault::new(
                "NG650",
                "Choose cpu, vulkan, cuda12 or cuda13 for the offline Windows runtime",
            )
            .into());
        }
    };
    let downloads: Vec<setup::Download> =
        serde_json::from_str(include_str!("../runtime-downloads.json"))?;
    let selected = downloads
        .into_iter()
        .filter(|d| d.file.contains(marker))
        .collect::<Vec<_>>();
    anyhow::ensure!(
        !selected.is_empty(),
        Fault::new("NG650", "Pinned runtime archives unavailable")
    );
    Ok(selected)
}
fn verify(path: &Path, bytes: u64, sha: &str, c: &Control) -> Result<()> {
    check_path(path)?;
    anyhow::ensure!(
        path.is_file() && path.metadata()?.len() == bytes && storage::hash_file(path, c)? == sha,
        Fault::new(
            "NG650",
            format!(
                "Offline asset failed size/SHA256 verification: {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )
        )
    );
    Ok(())
}
fn build_inventory(
    settings: &Settings,
    ids: &[String],
    runtime: Option<&str>,
    c: &Control,
) -> Result<(Inventory, BTreeMap<String, PathBuf>)> {
    c.check()?;
    let ids = ids.iter().collect::<BTreeSet<_>>();
    anyhow::ensure!(
        ids.len() <= 100 && (!ids.is_empty() || runtime.is_some()),
        Fault::new(
            "NG650",
            "Select at least one reader or runtime and no more than 100 readers"
        )
    );
    check_path(&settings.models_dir)?;
    let (installed, _) = crate::runtime::inventory_available(&settings.models_dir)?;
    let mut inventory=Inventory {schema:1,readers:vec![],runtime:None,files:vec![],total_bytes:0,contents_note:"Contains selected reader metadata, GGUF weights/projectors and optional publisher-pinned Windows runtime archives. No document, writer, credential, endpoint or application-settings files are included. Model hashes identify the locally registered revision, not independent publisher authenticity. Runtime archives must match this NoteGen build's compiled pins. Runtime activation requires installed NoteGen native dependencies; app-local Microsoft support DLLs are reused only from that target installation.".into()};
    let mut paths = BTreeMap::new();
    for id in ids {
        c.check()?;
        let mut model = installed
            .iter()
            .find(|m| &m.id == id)
            .cloned()
            .ok_or_else(|| Fault::new("NG650", format!("Selected reader {id} is not installed")))?;
        anyhow::ensure!(
            model.endpoint.is_none(),
            Fault::new(
                "NG650",
                "Server endpoint readers cannot be transferred offline"
            )
        );
        let mut pair = vec![];
        for path in [&model.weights, &model.projector] {
            check_path(path)?;
            crate::runtime::verify_gguf(path)?;
            let bytes = path.metadata()?.len();
            let hash = storage::hash_file(path, c)?;
            anyhow::ensure!(
                (4..=100_000_000_000).contains(&bytes),
                Fault::new("NG650", "Reader file exceeds offline bundle limits")
            );
            let name = format!("objects/{hash}.gguf");
            if !paths.contains_key(&name) {
                inventory.files.push(FileEntry {
                    path: name.clone(),
                    bytes,
                    sha256: hash.clone(),
                });
                paths.insert(name.clone(), path.clone());
            }
            pair.push((name, hash));
        }
        anyhow::ensure!(
            model.sha256 == storage::fingerprint(&(&pair[0].1, &pair[1].1))?,
            Fault::new("NG650", "Reader files changed since registration")
        );
        model.weights = PathBuf::new();
        model.projector = PathBuf::new();
        model.endpoint = None;
        model.alias.clear();
        inventory.readers.push(ReaderEntry {
            model,
            weights: pair[0].0.clone(),
            projector: pair[1].0.clone(),
        });
    }
    if let Some(backend) = runtime {
        let pins = runtime_pins(backend)?;
        let mut archives = vec![];
        for pin in pins {
            let path = setup::runtime_root(settings)
                .join("downloads")
                .join(&pin.file);
            verify(&path, pin.bytes, &pin.sha256, c)?;
            let name = format!("runtime/{}", pin.file);
            inventory.files.push(FileEntry {
                path: name.clone(),
                bytes: pin.bytes,
                sha256: pin.sha256,
            });
            paths.insert(name.clone(), path);
            archives.push(name);
        }
        inventory.runtime = Some(RuntimeEntry {
            backend: backend.into(),
            build: "b12032".into(),
            archives,
        });
    }
    inventory.files.sort_by(|a, b| a.path.cmp(&b.path));
    inventory.total_bytes = inventory.files.iter().map(|f| f.bytes).sum();
    validate(&inventory)?;
    Ok((inventory, paths))
}
pub fn preview(
    settings: &Settings,
    ids: &[String],
    runtime: Option<&str>,
    c: &Control,
) -> Result<Inventory> {
    Ok(build_inventory(settings, ids, runtime, c)?.0)
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.contains(['\\', ':'])
        && name.split('/').all(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}
fn validate(inventory: &Inventory) -> Result<()> {
    anyhow::ensure!(
        inventory.schema == 1
            && inventory.readers.len() <= 100
            && !inventory.files.is_empty()
            && inventory.files.len() <= 206,
        Fault::new("NG650", "Invalid offline bundle inventory")
    );
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for file in &inventory.files {
        anyhow::ensure!(
            valid_name(&file.path)
                && names.insert(file.path.to_ascii_lowercase())
                && file.bytes >= 4
                && file.bytes <= 100_000_000_000
                && file.sha256.len() == 64
                && file.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            Fault::new(
                "NG650",
                "Invalid, duplicate or oversized offline bundle file"
            )
        );
        total = total
            .checked_add(file.bytes)
            .ok_or_else(|| Fault::new("NG650", "Offline bundle size overflow"))?;
    }
    anyhow::ensure!(
        total == inventory.total_bytes && total <= MAX_BYTES,
        Fault::new(
            "NG650",
            "Offline bundle exceeds limits or its byte total is inconsistent"
        )
    );
    let mut referenced = BTreeSet::new();
    let mut reader_ids = BTreeSet::new();
    for reader in &inventory.readers {
        models::validate_model(&reader.model)?;
        anyhow::ensure!(
            reader_ids.insert(reader.model.id.to_ascii_lowercase())
                && reader.model.endpoint.is_none()
                && reader.model.alias.is_empty()
                && reader.model.weights.as_os_str().is_empty()
                && reader.model.projector.as_os_str().is_empty(),
            Fault::new(
                "NG650",
                "Offline reader contains private paths, endpoint data or duplicate IDs"
            )
        );
        let mut hashes = vec![];
        for name in [&reader.weights, &reader.projector] {
            let asset = inventory
                .files
                .iter()
                .find(|f| &f.path == name)
                .ok_or_else(|| Fault::new("NG650", "Reader asset missing from inventory"))?;
            anyhow::ensure!(
                asset.path == format!("objects/{}.gguf", asset.sha256),
                Fault::new("NG650", "Reader object name must match its content hash")
            );
            hashes.push(&asset.sha256);
            referenced.insert(name.clone());
        }
        anyhow::ensure!(
            reader.model.sha256 == storage::fingerprint(&(hashes[0], hashes[1]))?,
            Fault::new("NG650", "Reader pair fingerprint mismatch")
        );
    }
    if let Some(runtime) = &inventory.runtime {
        anyhow::ensure!(
            runtime.build == "b12032",
            Fault::new(
                "NG650",
                "Runtime build is not pinned by this NoteGen version"
            )
        );
        let pins = runtime_pins(&runtime.backend)?;
        let expected = pins
            .iter()
            .map(|p| format!("runtime/{}", p.file))
            .collect::<BTreeSet<_>>();
        anyhow::ensure!(
            runtime.archives.iter().collect::<BTreeSet<_>>().len() == runtime.archives.len()
                && runtime.archives.iter().cloned().collect::<BTreeSet<_>>() == expected,
            Fault::new("NG650", "Runtime archive set does not match compiled pins")
        );
        for pin in pins {
            let name = format!("runtime/{}", pin.file);
            let asset = inventory
                .files
                .iter()
                .find(|f| f.path == name)
                .ok_or_else(|| Fault::new("NG650", "Runtime asset missing"))?;
            anyhow::ensure!(
                asset.bytes == pin.bytes && asset.sha256 == pin.sha256,
                Fault::new("NG650", "Runtime metadata is not publisher-pinned")
            );
            referenced.insert(name);
        }
    }
    anyhow::ensure!(
        referenced.len() == inventory.files.len(),
        Fault::new("NG650", "Offline bundle includes unreferenced files")
    );
    Ok(())
}
pub fn export(
    settings: &Settings,
    ids: &[String],
    runtime: Option<&str>,
    output: &Path,
    c: &Control,
) -> Result<()> {
    c.check()?;
    check_path(output)?;
    anyhow::ensure!(
        !output.exists(),
        Fault::new("NG401", "Offline bundle already exists")
    );
    anyhow::ensure!(
        output
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip")),
        Fault::new("NG650", "Choose a .zip offline bundle")
    );
    let (inventory, paths) = build_inventory(settings, ids, runtime, c)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut zip = zip::ZipWriter::new(&mut temp);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("inventory.json", options)?;
        zip.write_all(&serde_json::to_vec_pretty(&inventory)?)?;
        for asset in &inventory.files {
            c.check()?;
            let path = &paths[&asset.path];
            verify(path, asset.bytes, &asset.sha256, c)?;
            zip.start_file(&asset.path, options)?;
            let mut file = fs::File::open(path)?;
            let mut buffer = vec![0u8; 1024 * 1024];
            let mut hash = Sha256::new();
            let mut total = 0;
            loop {
                c.check()?;
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hash.update(&buffer[..n]);
                zip.write_all(&buffer[..n])?;
                total += n as u64;
            }
            anyhow::ensure!(
                total == asset.bytes && hex::encode(hash.finalize()) == asset.sha256,
                Fault::new("NG650", "Offline asset changed during export")
            );
        }
        zip.finish()?;
    }
    c.check()?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(output)
        .map_err(|_| Fault::new("NG401", "Offline bundle collision or unavailable folder"))?;
    Ok(())
}
fn read_inventory(zip: &mut zip::ZipArchive<fs::File>, c: &Control) -> Result<Inventory> {
    c.check()?;
    anyhow::ensure!(
        zip.len() <= 207,
        Fault::new("NG650", "Offline bundle has too many ZIP entries")
    );
    let mut names = BTreeSet::new();
    for i in 0..zip.len() {
        let entry = zip.by_index(i)?;
        anyhow::ensure!(
            valid_name(entry.name())
                && entry.compression() == zip::CompressionMethod::Stored
                && entry.compressed_size() == entry.size()
                && !entry.is_symlink()
                && !entry.is_dir()
                && entry.enclosed_name().is_some()
                && names.insert(entry.name().to_ascii_lowercase()),
            Fault::new(
                "NG650",
                "Unsafe, compressed, duplicate or symlink offline bundle entry; this format requires stored ZIP entries"
            )
        );
    }
    let mut bytes = vec![];
    let entry = zip.by_name("inventory.json")?;
    anyhow::ensure!(
        entry.size() <= 1_000_000,
        Fault::new("NG650", "Offline inventory too large")
    );
    entry.take(1_000_001).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 1_000_000,
        Fault::new("NG650", "Offline inventory too large")
    );
    let inventory: Inventory = serde_json::from_slice(&bytes)?;
    validate(&inventory)?;
    anyhow::ensure!(
        zip.len() == inventory.files.len() + 1,
        Fault::new("NG650", "Offline archive file list differs from inventory")
    );
    for file in &inventory.files {
        let entry = zip.by_name(&file.path)?;
        anyhow::ensure!(
            entry.size() == file.bytes,
            Fault::new("NG650", "Offline ZIP declared size differs from inventory")
        );
    }
    Ok(inventory)
}
fn copy_verified(
    reader: &mut impl Read,
    writer: &mut impl Write,
    asset: &FileEntry,
    c: &Control,
) -> Result<()> {
    let mut hash = Sha256::new();
    let mut copied = 0u64;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        c.check()?;
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        copied = copied
            .checked_add(n as u64)
            .ok_or_else(|| Fault::new("NG650", "Offline asset size overflow"))?;
        anyhow::ensure!(
            copied <= asset.bytes,
            Fault::new("NG650", "Offline asset expanded beyond its declared limit")
        );
        hash.update(&buffer[..n]);
        writer.write_all(&buffer[..n])?;
    }
    anyhow::ensure!(
        copied == asset.bytes && hex::encode(hash.finalize()) == asset.sha256,
        Fault::new("NG650", "Offline asset checksum mismatch")
    );
    Ok(())
}
/// Inspects the exact file list and verifies all file contents, without extracting or executing.
pub fn inspect(input: &Path, c: &Control) -> Result<Inventory> {
    check_path(input)?;
    let mut zip = zip::ZipArchive::new(fs::File::open(input)?)?;
    let inventory = read_inventory(&mut zip, c)?;
    for asset in &inventory.files {
        copy_verified(
            &mut zip.by_name(&asset.path)?,
            &mut std::io::sink(),
            asset,
            c,
        )?;
    }
    Ok(inventory)
}
fn extract_runtime(archives: &[PathBuf], destination: &Path, c: &Control) -> Result<()> {
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for archive in archives {
        let mut zip = zip::ZipArchive::new(fs::File::open(archive)?)?;
        anyhow::ensure!(
            zip.len() <= 1000,
            Fault::new("NG650", "Runtime ZIP has too many files")
        );
        for i in 0..zip.len() {
            c.check()?;
            let mut entry = zip.by_index(i)?;
            anyhow::ensure!(
                !entry.is_symlink() && entry.enclosed_name().is_some(),
                Fault::new("NG650", "Unsafe runtime archive entry")
            );
            if entry.is_dir() {
                continue;
            }
            let name = entry
                .enclosed_name()
                .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
                .ok_or_else(|| Fault::new("NG650", "Invalid runtime name"))?;
            let lower = name.to_ascii_lowercase();
            if !(lower.ends_with(".dll")
                || lower == "llama-server.exe"
                || ["license", "notice", "copying"]
                    .iter()
                    .any(|s| lower.contains(s)))
            {
                continue;
            }
            anyhow::ensure!(
                valid_name(&name) && names.insert(lower),
                Fault::new("NG650", "Duplicate or invalid flattened runtime filename")
            );
            total = total
                .checked_add(entry.size())
                .ok_or_else(|| Fault::new("NG650", "Runtime expansion overflow"))?;
            anyhow::ensure!(
                total <= 4_000_000_000 && entry.size() <= 2_000_000_000,
                Fault::new("NG650", "Runtime expansion exceeds extraction limits")
            );
            let expected = entry.size();
            let mut out = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination.join(name))?;
            let mut count = 0u64;
            let mut buffer = [0u8; 65536];
            loop {
                c.check()?;
                let n = entry.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                count += n as u64;
                anyhow::ensure!(
                    count <= expected,
                    Fault::new("NG650", "Runtime file exceeded declared size")
                );
                out.write_all(&buffer[..n])?;
            }
            anyhow::ensure!(
                count == expected,
                Fault::new("NG650", "Runtime file exceeded declared size")
            );
            out.sync_all()?;
        }
    }
    Ok(())
}
/// Prepares a separate registry/runtime and returns its settings. The caller can
/// atomically save these settings after success. Existing active data is untouched.
pub fn import(
    settings: &Settings,
    input: &Path,
    destination: &Path,
    c: &Control,
) -> Result<Settings> {
    c.check()?;
    check_path(input)?;
    check_path(destination)?;
    anyhow::ensure!(
        !destination.exists(),
        Fault::new("NG401", "Offline destination already exists")
    );
    let mut zip = zip::ZipArchive::new(fs::File::open(input)?)?;
    let inventory = read_inventory(&mut zip, c)?;
    anyhow::ensure!(
        cfg!(windows) || inventory.runtime.is_none(),
        Fault::new(
            "NG650",
            "Bundled runtimes support Windows x64; import a models-only bundle on this OS"
        )
    );
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    check_path(parent)?;
    let stage = tempfile::tempdir_in(parent)?;
    let final_payload = std::path::absolute(destination)?.join("payload");
    let model_dir = stage.path().join("models");
    fs::create_dir(&model_dir)?;
    let (existing, errors) = crate::runtime::inventory_available(&settings.models_dir)?;
    anyhow::ensure!(
        errors.is_empty(),
        Fault::new(
            "NG650",
            "Repair existing reader registrations before importing; none were changed"
        )
    );
    anyhow::ensure!(
        inventory.readers.iter().all(|reader| {
            existing.iter().all(|old| {
                old.id == reader.model.id || !old.id.eq_ignore_ascii_case(&reader.model.id)
            })
        }),
        Fault::new(
            "NG650",
            "Imported reader ID conflicts with an existing ID by letter case; no registrations were changed"
        )
    );
    for mut model in existing {
        if model.endpoint.is_none() {
            model.weights = model.weights.canonicalize()?;
            model.projector = model.projector.canonicalize()?;
        }
        storage::atomic_json(
            &model_dir.join(format!("{}.notegen.json", model.id)),
            &model,
        )?;
    }
    for asset in &inventory.files {
        c.check()?;
        let path = stage.path().join(&asset.path);
        fs::create_dir_all(path.parent().unwrap())?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        copy_verified(&mut zip.by_name(&asset.path)?, &mut file, asset, c)?;
        file.sync_all()?;
    }
    for reader in &inventory.readers {
        c.check()?;
        let mut model = reader.model.clone();
        crate::runtime::verify_gguf(&stage.path().join(&reader.weights))?;
        crate::runtime::verify_gguf(&stage.path().join(&reader.projector))?;
        model.weights = final_payload.join(&reader.weights);
        model.projector = final_payload.join(&reader.projector);
        let active = model_dir.join(format!("{}.notegen.json", model.id));
        if active.is_file() {
            fs::copy(
                &active,
                model_dir.join(format!("{}.previous.json", model.id)),
            )?;
        }
        storage::atomic_json(&active, &model)?;
    }
    let mut next = settings.clone();
    next.models_dir = final_payload.join("models");
    if let Some(runtime) = &inventory.runtime {
        let runtime_dir = stage.path().join("ai-runtime");
        fs::create_dir(&runtime_dir)?;
        let archives = runtime
            .archives
            .iter()
            .map(|s| stage.path().join(s))
            .collect::<Vec<_>>();
        extract_runtime(&archives, &runtime_dir, c)?;
        for name in [
            "vcruntime140.dll",
            "vcruntime140_1.dll",
            "msvcp140.dll",
            "concrt140.dll",
        ] {
            c.check()?;
            let source = setup::runtime_root(settings).join(name);
            let target = runtime_dir.join(name);
            if source.is_file() && !target.exists() {
                check_path(&source)?;
                anyhow::ensure!(
                    source.metadata()?.len() <= 20_000_000,
                    Fault::new(
                        "NG650",
                        "Installed runtime support DLL exceeds its size limit"
                    )
                );
                fs::copy(source, target)?;
            }
        }
        let server = runtime_dir.join("llama-server.exe");
        anyhow::ensure!(
            server.is_file(),
            Fault::new("NG650", "Offline runtime has no llama-server")
        );
        // The server comes from compiled-pinned archives. Support DLLs come only
        // from the target PC's existing installation, matching normal setup.
        let (ok, out, err) = crate::scan::run(
            &server,
            &["--list-devices".into()],
            Duration::from_secs(30),
            c,
        )?;
        anyhow::ensure!(
            ok,
            Fault::new("NG650", format!("Offline runtime startup failed: {err}"))
        );
        let backend = match runtime.backend.as_str() {
            "cpu" => Backend::Cpu,
            "vulkan" => Backend::Vulkan,
            _ => Backend::Cuda,
        };
        let devices = crate::hardware::parse_devices(&format!("{out}\n{err}"));
        anyhow::ensure!(
            backend == Backend::Cpu
                || devices.iter().any(|d| d.backend
                    == if backend == Backend::Cuda {
                        "cuda"
                    } else {
                        "vulkan"
                    }),
            Fault::new("NG650", "Offline runtime cannot access its requested GPU")
        );
        next.server = final_payload.join("ai-runtime/llama-server.exe");
        next.backend = backend;
        let record = setup::RuntimeRecord {
            server: next.server.clone(),
            backend,
            build: runtime.build.clone(),
        };
        storage::atomic_json(&runtime_dir.join("active-ai-runtime.json"), &record)?;
    }
    storage::atomic_json(&stage.path().join("inventory.json"), &inventory)?;
    c.check()?;
    // create_dir claims the destination atomically, avoiding rename-overwrite races.
    fs::create_dir(destination).map_err(|_| {
        Fault::new(
            "NG401",
            "Offline destination collision or unavailable folder",
        )
    })?;
    if let Err(error) = fs::rename(stage.path(), destination.join("payload")) {
        let _ = fs::remove_dir(destination);
        return Err(error.into());
    }
    Ok(next)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preflight {
    pub model_ids: Vec<String>,
    pub selected_bytes: u64,
    pub verified_reusable_bytes: u64,
    pub remaining_download_bytes: u64,
    pub retained_rollback_bytes: u64,
    pub additional_disk_bytes: u64,
    pub runtime_expansion_is_upper_bound: bool,
    pub available_disk_bytes: Option<u64>,
    pub enough_space: Option<bool>,
    pub note: String,
}
pub fn preflight(
    settings: &Settings,
    ids: &[String],
    runtime: Option<&str>,
    c: &Control,
) -> Result<Preflight> {
    c.check()?;
    let mut result=Preflight{model_ids:ids.to_vec(),selected_bytes:0,verified_reusable_bytes:0,remaining_download_bytes:0,retained_rollback_bytes:0,additional_disk_bytes:0,runtime_expansion_is_upper_bound:runtime.is_some(),available_disk_bytes:None,enough_space:None,note:"Only complete files with matching SHA256 count as reusable. Partial downloads are excluded until verified. Existing versions remain on disk for rollback. Runtime expansion reserves its enforced 4 GB upper bound, not a measured extracted size. Model activation needs no extra copy of reusable assets.".into()};
    let (installed, _) = crate::runtime::inventory_available(&settings.models_dir)?;
    let unique = ids.iter().collect::<BTreeSet<_>>();
    for id in unique {
        let (_, pin) = setup::pinned(id)?;
        for (asset, filename) in [
            (&pin.weights, "weights.gguf"),
            (&pin.projector, "projector.gguf"),
        ] {
            result.selected_bytes += asset.bytes;
            let path = settings
                .models_dir
                .join("versions")
                .join(id)
                .join(&pin.revision)
                .join(filename);
            let current = installed.iter().find(|m| &m.id == id);
            let candidate = current.map(|m| {
                if filename == "weights.gguf" {
                    &m.weights
                } else {
                    &m.projector
                }
            });
            if verify(&path, asset.bytes, &asset.sha256, c).is_ok()
                || candidate.is_some_and(|p| verify(p, asset.bytes, &asset.sha256, c).is_ok())
            {
                result.verified_reusable_bytes += asset.bytes;
            }
        }
        if let Some(old) = installed.iter().find(|m| &m.id == id) {
            result.retained_rollback_bytes +=
                old.weights.metadata()?.len() + old.projector.metadata()?.len();
        }
    }
    if let Some(backend) = runtime {
        for pin in runtime_pins(backend)? {
            result.selected_bytes += pin.bytes;
            let path = setup::runtime_root(settings)
                .join("downloads")
                .join(&pin.file);
            if verify(&path, pin.bytes, &pin.sha256, c).is_ok() {
                result.verified_reusable_bytes += pin.bytes;
            }
        }
    }
    c.check()?;
    result.remaining_download_bytes = result.selected_bytes - result.verified_reusable_bytes;
    result.additional_disk_bytes = result.remaining_download_bytes
        + 128 * 1024 * 1024
        + if runtime.is_some() { 4_000_000_000 } else { 0 };
    let absolute = std::path::absolute(&settings.models_dir)?;
    let disks = sysinfo::Disks::new_with_refreshed_list();
    if let Some(disk) = disks
        .iter()
        .filter(|d| absolute.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().components().count())
    {
        result.available_disk_bytes = Some(disk.available_space());
        result.enough_space = Some(disk.available_space() >= result.additional_disk_bytes);
    }
    Ok(result)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityCard {
    pub id: String,
    pub name: String,
    pub family: String,
    pub source: String,
    pub download_repository: String,
    pub pinned_revision: String,
    pub tasks: Vec<String>,
    pub estimated_resident_mib: u64,
    pub download_bytes: u64,
    pub license: String,
    pub tested_languages: Vec<String>,
    pub limitations: Vec<String>,
}
pub fn capability_cards() -> Result<Vec<CapabilityCard>> {
    crate::planning::catalog().into_iter().map(|model|{let (m,pin)=setup::pinned(&model.id)?;Ok(CapabilityCard{id:m.id,name:m.name,family:m.family,source:m.source,download_repository:pin.repository,pinned_revision:pin.revision,tasks:if m.role=="vision"{vec!["structured transcription".into(),"table reconstruction".into(),"figure interpretation requiring review".into()]}else{vec!["text transcription".into(),"table recognition".into()]},estimated_resident_mib:m.resident_mib,download_bytes:pin.weights.bytes+pin.projector.bytes,license:"Not recorded in the local catalog. Inspect the original model and conversion repository licenses before redistribution.".into(),tested_languages:vec![],limitations:vec!["No per-language quality benchmark is bundled; an empty tested-language list is intentional.".into(),"Memory requirements are estimates. Agreement and confidence are not calibrated accuracy.".into(),"These task protocols do not establish measured handwriting or diagram accuracy.".into()]})}).collect()
}
