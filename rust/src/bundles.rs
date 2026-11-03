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
const MAX_INVENTORY_BYTES: u64 = 1_000_000;
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
) -> Result<(Inventory, BTreeMap<String, PathBuf>, Vec<u8>)> {
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
    let metadata = serde_json::to_vec_pretty(&inventory)?;
    anyhow::ensure!(
        metadata.len() as u64 <= MAX_INVENTORY_BYTES,
        Fault::new(
            "NG650",
            "Offline inventory exceeds 1 MB; select fewer readers or shorten registration metadata"
        )
    );
    Ok((inventory, paths, metadata))
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
    let (inventory, paths, metadata) = build_inventory(settings, ids, runtime, c)?;
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
        zip.write_all(&metadata)?;
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
        entry.size() <= MAX_INVENTORY_BYTES,
        Fault::new("NG650", "Offline inventory too large")
    );
    entry
        .take(MAX_INVENTORY_BYTES + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_INVENTORY_BYTES,
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
/// Low-level extraction into a separate registry/runtime for staging and fixtures.
/// Production callers must use import_validated before selecting these settings.
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

/// Import into a new isolated destination, retain failed candidates, and expose
/// selectable settings only after fresh native print/table canaries pass. No
/// registration or settings in the original installation is modified.
pub fn import_validated(
    settings: &Settings,
    input: &Path,
    destination: &Path,
    c: &Control,
    log: &crate::storage::Activity,
) -> Result<Settings> {
    let mut next = import(settings, input, destination, c)?;
    next.external_server = false;
    let payload = std::path::absolute(destination)?.join("payload");
    let status = payload.join("canary-status.json");
    storage::protected_json(
        &status,
        &serde_json::json!({
            "schema":1, "state":"pending",
            "note":"Isolated candidates only. Original installation/settings remain selected. A fresh native print/table canary is required before this registry can be selected."
        }),
    )?;
    let result = (|| -> Result<Settings> {
        let models = crate::runtime::inventory(&next.models_dir)?;
        anyhow::ensure!(
            models
                .iter()
                .filter(|model| model.endpoint.is_none())
                .count()
                <= 64,
            Fault::new(
                "NG660",
                "Offline activation supports at most 64 local readers"
            )
        );
        // Withdraw all private descriptor pointers before any native validation.
        // Files stay inspectable, and a failed candidate is never installed into
        // the source registry or made selectable through returned settings.
        let pending = payload.join("pending-registry");
        check_path(&pending)?;
        anyhow::ensure!(
            !pending.exists(),
            Fault::new("NG401", "Pending registry collision")
        );
        fs::rename(&next.models_dir, &pending)?;
        let runtime = payload.join("ai-runtime");
        let runtime_pointer = runtime.join("active-ai-runtime.json");
        let runtime_content = if runtime_pointer.is_file() {
            check_path(&runtime_pointer)?;
            let snapshot = crate::activation::snapshot(&runtime_pointer, c)?
                .ok_or_else(|| Fault::new("NG660", "Offline runtime pointer disappeared"))?;
            let mut pointer = tempfile::TempPath::try_from_path(&runtime_pointer)?;
            pointer.disable_cleanup(true);
            pointer
                .persist_noclobber(runtime.join("pending-ai-runtime.json"))
                .map_err(|error| error.error)?;
            Some(snapshot.content()?)
        } else {
            None
        };
        let mut candidates = vec![];
        let mut external = vec![];
        for model in models {
            c.check()?;
            if model.endpoint.is_none() {
                candidates.push(crate::models::prepare_custom(&next, model, c)?);
            } else {
                external.push(crate::activation::Change {
                    scope: crate::activation::Scope::Models,
                    file: format!("{}.notegen.json", model.id),
                    expected: None,
                    content: serde_json::to_vec_pretty(&model)?,
                });
            }
        }
        anyhow::ensure!(
            !candidates.is_empty(),
            Fault::new(
                "NG660",
                "Offline candidates remain pending until a local reader canary is available"
            )
        );
        let probes = candidates
            .iter()
            .map(|candidate| candidate.model().clone())
            .collect::<Vec<_>>();
        let proofs = setup::validate_pairs(&probes, &next, &payload.join("canaries"), c, log)?;
        for (model, proof) in probes.iter().zip(&proofs) {
            proof.verify_current(model, &next.server, c)?;
        }
        let mut changes = external;
        for candidate in &candidates {
            changes.extend(candidate.changes(c)?);
        }
        if let Some(content) = runtime_content {
            changes.push(crate::activation::Change {
                scope: crate::activation::Scope::Runtime,
                file: "active-ai-runtime.json".into(),
                expected: None,
                content,
            });
        }
        crate::activation::commit(
            &next.models_dir,
            &setup::runtime_root(&next),
            None,
            changes,
            c,
        )?;
        storage::atomic_json(
            &status,
            &serde_json::json!({
                "schema":1, "state":"validated",
                "note":"Known print/table compatibility canaries passed. Select the separately saved settings explicitly. Original installation remains untouched; real documents still require review."
            }),
        )?;
        Ok(next.clone())
    })();
    result.map_err(|error| error.context(format!("Offline import remains isolated/pending at {}; the original installation and settings were preserved", destination.display())))
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
    pub provenance: CapabilityProvenance,
    pub publisher_claims: Vec<String>,
    pub publisher_declared_languages: Vec<String>,
    pub language_evidence_note: String,
    pub license_declarations: Vec<CapabilityLicense>,
    pub sources: Vec<CapabilitySource>,
    pub assets: Vec<CapabilityAsset>,
    pub task_protocols: Vec<TaskProtocol>,
    pub memory_estimate: CapabilityMemory,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySourceKind {
    PublisherModelCard,
    ConversionModelCard,
    PublisherLicenseFile,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySource {
    pub kind: CapabilitySourceKind,
    pub url: String,
    pub revision: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityProvenance {
    pub upstream_publisher_account: String,
    pub upstream_repository: String,
    /// Documentation captured independently, not the upstream revision used by the converter.
    pub upstream_documentation_revision: String,
    pub upstream_weights_revision: Option<String>,
    pub conversion_repository_owner: String,
    pub conversion_repository: String,
    pub conversion_revision: String,
    pub metadata_verified_on: String,
    pub note: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseScope {
    PublisherModel,
    ConversionModel,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityLicense {
    pub scope: LicenseScope,
    pub declared_identifier: Option<String>,
    pub evidence_url: String,
    pub separate_license_file_available: bool,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityAsset {
    pub role: String,
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
    pub immutable_url: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskProtocol {
    pub adapter: String,
    pub task: String,
    pub request: String,
    pub response: String,
    pub evidence_note: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryEstimateBasis {
    PlanningCatalogEstimate,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityMemory {
    pub resident_mib: u64,
    pub basis: MemoryEstimateBasis,
    pub measured_peak_mib: Option<u64>,
    pub note: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityFacts {
    schema: u32,
    verified_on: String,
    models: Vec<ReaderFacts>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReaderFacts {
    id: String,
    publisher_account: String,
    upstream_repository: String,
    documentation_revision: String,
    upstream_license: String,
    publisher_license_file_available: bool,
    conversion_repository: String,
    conversion_revision: String,
    conversion_license: Option<String>,
    publisher_claims: Vec<String>,
    declared_languages: Vec<String>,
    language_note: String,
    sources: Vec<CapabilitySource>,
}
fn capability_hex(value: &str, n: usize) -> bool {
    value.len() == n
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn capability_repository(value: &str) -> bool {
    let parts = value.split('/').collect::<Vec<_>>();
    parts.len() == 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 150
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}
fn capability_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
}
fn capability_protocols(prompt: &str) -> Result<Vec<TaskProtocol>> {
    let adapter = prompt.to_owned();
    let evidence_note="Application request/output contract only; not a measured accuracy or language-support claim.".to_owned();
    let tasks: Vec<(&str, &str, &str)> = match prompt {
        "lighton" => vec![(
            "grounded document transcription",
            "grounding",
            "Publisher grounding blocks parsed into source-grounded text, tables and other emitted block kinds.",
        )],
        "glm" => vec![
            (
                "text transcription",
                "Text Recognition:",
                "Plain/Markdown recognition parsed into NoteGen blocks.",
            ),
            (
                "table recognition",
                "Table Recognition:",
                "Explicit table-region pass parsed into NoteGen blocks.",
            ),
        ],
        "structured" => vec![(
            "structured document transcription",
            "Scanned-page JSON schema instruction",
            "Schema-bounded text, heading, list, table, figure, equation, code, caption and reference blocks. Figure interpretations require review; equations/code remain recognized text.",
        )],
        _ => {
            return Err(Fault::new(
                "NG650",
                "Unknown reader adapter; capability protocols must be reviewed explicitly",
            )
            .into());
        }
    };
    Ok(tasks
        .into_iter()
        .map(|(task, request, response)| TaskProtocol {
            adapter: adapter.clone(),
            task: task.into(),
            request: request.into(),
            response: response.into(),
            evidence_note: evidence_note.clone(),
        })
        .collect())
}
fn capability_cards_from(facts: CapabilityFacts) -> Result<Vec<CapabilityCard>> {
    let catalog = crate::planning::catalog();
    anyhow::ensure!(
        facts.schema == 1
            && facts.models.len() == catalog.len()
            && chrono::NaiveDate::parse_from_str(&facts.verified_on, "%Y-%m-%d").is_ok(),
        Fault::new(
            "NG650",
            "Capability facts schema/date/catalog coverage is invalid"
        )
    );
    let mut by_id = BTreeMap::new();
    for fact in facts.models {
        anyhow::ensure!(
            by_id.insert(fact.id.clone(), fact).is_none(),
            Fault::new("NG650", "Duplicate packaged capability facts")
        );
    }
    let mut cards = vec![];
    for model in catalog {
        let fact = by_id.remove(&model.id).ok_or_else(|| {
            Fault::new(
                "NG650",
                "Catalog reader is missing reviewed capability facts",
            )
        })?;
        let (m, pin) = setup::pinned(&model.id)?;
        anyhow::ensure!(
            capability_repository(&fact.upstream_repository)
                && capability_repository(&fact.conversion_repository)
                && fact.upstream_repository.split('/').next()
                    == Some(fact.publisher_account.as_str())
                && capability_hex(&fact.documentation_revision, 40)
                && fact.conversion_repository == pin.repository
                && fact.conversion_revision == pin.revision
                && capability_identifier(&fact.upstream_license)
                && fact
                    .conversion_license
                    .as_deref()
                    .is_none_or(capability_identifier)
                && !fact.publisher_claims.is_empty()
                && fact.publisher_claims.len() <= 8
                && fact
                    .publisher_claims
                    .iter()
                    .all(|s| !s.trim().is_empty() && s.len() <= 2000)
                && fact.declared_languages.len() <= 256
                && fact
                    .declared_languages
                    .iter()
                    .all(|s| capability_identifier(s))
                && fact.language_note.len() <= 4000,
            Fault::new(
                "NG650",
                "Capability provenance does not match the compiled download pins"
            )
        );
        let mut source_kinds = BTreeSet::new();
        for source in &fact.sources {
            let (repo, revision, file) = match source.kind {
                CapabilitySourceKind::PublisherModelCard => (
                    &fact.upstream_repository,
                    &fact.documentation_revision,
                    "README.md",
                ),
                CapabilitySourceKind::ConversionModelCard => (
                    &fact.conversion_repository,
                    &fact.conversion_revision,
                    "README.md",
                ),
                CapabilitySourceKind::PublisherLicenseFile => (
                    &fact.upstream_repository,
                    &fact.documentation_revision,
                    "LICENSE",
                ),
            };
            anyhow::ensure!(
                source_kinds.insert(format!("{:?}", source.kind))
                    && source.revision == *revision
                    && source.url == format!("https://huggingface.co/{repo}/raw/{revision}/{file}")
                    && (1..=1_000_000).contains(&source.bytes)
                    && capability_hex(&source.sha256, 64),
                Fault::new("NG650", "Invalid or duplicate capability source evidence")
            );
        }
        let publisher_card = fact
            .sources
            .iter()
            .find(|s| s.kind == CapabilitySourceKind::PublisherModelCard)
            .ok_or_else(|| Fault::new("NG650", "Publisher model-card source is missing"))?;
        let conversion_card = fact
            .sources
            .iter()
            .find(|s| s.kind == CapabilitySourceKind::ConversionModelCard)
            .ok_or_else(|| Fault::new("NG650", "Pinned conversion-card source is missing"))?;
        let license_file = fact
            .sources
            .iter()
            .find(|s| s.kind == CapabilitySourceKind::PublisherLicenseFile);
        anyhow::ensure!(
            license_file.is_some() == fact.publisher_license_file_available,
            Fault::new("NG650", "Capability license evidence scope is inconsistent")
        );
        let license_declarations=vec![CapabilityLicense{scope:LicenseScope::PublisherModel,declared_identifier:Some(fact.upstream_license.clone()),evidence_url:license_file.unwrap_or(publisher_card).url.clone(),separate_license_file_available:fact.publisher_license_file_available,note:if fact.publisher_license_file_available{"Publisher model-card declaration and captured LICENSE file."}else{"Publisher model-card declaration; no separate LICENSE file exists in the captured model repository revision."}.into()},CapabilityLicense{scope:LicenseScope::ConversionModel,declared_identifier:fact.conversion_license.clone(),evidence_url:conversion_card.url.clone(),separate_license_file_available:false,note:if fact.conversion_license.is_some(){"GGUF repository card declaration; no separate LICENSE file exists at the pinned conversion revision."}else{"The pinned conversion card does not declare a separate license. This absence is not replaced with an SDK/source-code license."}.into()}];
        let task_protocols = capability_protocols(&m.prompt)?;
        let tasks = task_protocols.iter().map(|p| p.task.clone()).collect();
        let download_bytes = pin
            .weights
            .bytes
            .checked_add(pin.projector.bytes)
            .ok_or_else(|| Fault::new("NG650", "Capability asset byte count overflow"))?;
        let assets = vec![("weights", &pin.weights), ("projector", &pin.projector)]
            .into_iter()
            .map(|(role, asset)| CapabilityAsset {
                role: role.into(),
                file: asset.file.clone(),
                bytes: asset.bytes,
                sha256: asset.sha256.clone(),
                immutable_url: format!(
                    "https://huggingface.co/{}/resolve/{}/{}",
                    pin.repository, pin.revision, asset.file
                ),
            })
            .collect();
        let mut limitations=vec!["No versioned per-language quality measurement is packaged for this exact quantized reader; tested_languages is intentionally empty.".into(),"Publisher benchmark/feature statements concern its own model/pipeline and are not NoteGen accuracy, speed or handwriting guarantees.".into(),"Resident memory is a planning estimate, not a measured peak; context, image size, backend, KV cache and native buffers change actual use.".into(),"The captured publisher documentation revision is not proof of the upstream weights revision used by the GGUF converter; that base commit is unknown.".into()];
        limitations.push(fact.language_note.clone());
        if m.prompt == "glm" {
            limitations.push("The publisher also documents Formula Recognition and schema extraction, but this NoteGen adapter emits only Text Recognition and Table Recognition prompts. The publisher's optional layout-detector SDK is a separate pipeline.".into());
        }
        if m.prompt == "lighton" {
            limitations.push("Targeted tasks still use the publisher's fixed grounding protocol; arbitrary custom instructions are not claimed as supported.".into());
        }
        let provenance=CapabilityProvenance{upstream_publisher_account:fact.publisher_account,upstream_repository:fact.upstream_repository,upstream_documentation_revision:fact.documentation_revision,upstream_weights_revision:None,conversion_repository_owner:pin.repository.split('/').next().unwrap_or_default().into(),conversion_repository:pin.repository.clone(),conversion_revision:pin.revision.clone(),metadata_verified_on:facts.verified_on.clone(),note:"GGUF repository owner and upstream model publisher are separate identities. Repository ownership does not independently prove who authored each quantization. Sources are packaged snapshots; opening cards performs no network requests.".into()};
        let license = format!(
            "Publisher model declaration: {}. Pinned GGUF conversion declaration: {}.",
            fact.upstream_license,
            fact.conversion_license
                .as_deref()
                .unwrap_or("not separately recorded")
        );
        cards.push(CapabilityCard{id:m.id,name:m.name,family:m.family,source:m.source,download_repository:pin.repository,pinned_revision:pin.revision,tasks,estimated_resident_mib:m.resident_mib,download_bytes,license,tested_languages:vec![],limitations,provenance,publisher_claims:fact.publisher_claims,publisher_declared_languages:fact.declared_languages,language_evidence_note:fact.language_note,license_declarations,sources:fact.sources,assets,task_protocols,memory_estimate:CapabilityMemory{resident_mib:m.resident_mib,basis:MemoryEstimateBasis::PlanningCatalogEstimate,measured_peak_mib:None,note:"Estimated resident allowance copied from the compiled planning catalog, in MiB (1,048,576 bytes). No measured peak is asserted by these static cards.".into()}});
    }
    anyhow::ensure!(
        by_id.is_empty(),
        Fault::new(
            "NG650",
            "Capability facts include an unknown catalog reader"
        )
    );
    Ok(cards)
}
/// Read packaged, reviewed metadata only. Never performs network requests or loads reader files.
pub fn capability_cards() -> Result<Vec<CapabilityCard>> {
    let bytes = include_str!("../capabilities.json");
    anyhow::ensure!(
        bytes.len() <= 65_536,
        Fault::new(
            "NG650",
            "Packaged capability metadata exceeds its byte budget"
        )
    );
    capability_cards_from(serde_json::from_str(bytes)?)
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    #[test]
    fn stale_pins_missing_readers_and_relabelled_sources_cannot_produce_trusted_cards() {
        let original: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities.json")).unwrap();
        for mutate in 0..5 {
            let mut facts = original.clone();
            match mutate {
                0 => facts["models"][0]["conversion_revision"] = serde_json::json!("0".repeat(40)),
                1 => {
                    facts["models"].as_array_mut().unwrap().pop().unwrap();
                }
                2 => {
                    facts["models"][0]["sources"][0]["url"] =
                        serde_json::json!("https://huggingface.co/other/model/raw/main/README.md")
                }
                3 => {
                    let source = facts["models"][0]["sources"][0].clone();
                    facts["models"][0]["sources"]
                        .as_array_mut()
                        .unwrap()
                        .push(source);
                }
                _ => {
                    facts["models"][0]["publisher_license_file_available"] = serde_json::json!(true)
                }
            }
            assert!(
                capability_cards_from(serde_json::from_value(facts).unwrap()).is_err(),
                "variant {mutate}"
            );
        }
        assert!(capability_protocols("unreviewed_adapter").is_err());
    }
}
