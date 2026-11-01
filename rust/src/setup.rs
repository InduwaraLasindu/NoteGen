//! In-app local AI installation. Only pinned files are executed; active versions survive failure.
use crate::{
    hardware::{self, Hardware},
    models::{self, Asset, Update},
    planning::{self, Model},
    progress::{Phase, Work},
    storage::{self, Activity, Control},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[derive(Clone, Debug, Deserialize)]
struct Download {
    file: String,
    bytes: u64,
    sha256: String,
    url: String,
}
impl Download {
    fn asset(&self) -> Asset {
        Asset {
            file: self.file.clone(),
            bytes: self.bytes,
            sha256: self.sha256.clone(),
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
struct Pin {
    id: String,
    revision: String,
    weights: Download,
    projector: Download,
}
fn pins() -> Vec<Pin> {
    serde_json::from_str(include_str!("../downloads.json")).expect("compiled model downloads")
}
pub fn pinned(id: &str) -> Result<(Model, Update)> {
    let mut m = planning::catalog()
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| Fault::new("NG650", "Unknown catalog reader"))?;
    models::attach_catalog_metadata(&mut m);
    let p = pins()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| Fault::new("NG650", "No verified download for this reader"))?;
    let u = Update {
        id: m.id.clone(),
        repository: m.hub.as_ref().unwrap().repository.clone(),
        previous_revision: m.revision.clone(),
        revision: p.revision,
        weights: p.weights.asset(),
        projector: p.projector.asset(),
    };
    Ok((m, u))
}
#[derive(Clone, Debug)]
pub struct Recommendation {
    pub models: Vec<Model>,
    pub bytes: u64,
}
pub fn recommend(hw: &Hardware, s: &Settings) -> Result<Recommendation> {
    let mut s = s.clone();
    if s.preset == Preset::Fast {
        s.preset = Preset::Accuracy;
        s.handwriting = false;
    }
    let plan = planning::build(hw, &s, &planning::catalog())?;
    anyhow::ensure!(
        !plan.models.is_empty(),
        Fault::new(
            "NG300",
            "No bundled AI reader fits the current memory limits"
        )
    );
    let mut bytes = 0;
    for m in &plan.models {
        let (_, p) = pinned(&m.id)?;
        bytes += p.weights.bytes + p.projector.bytes;
    }
    Ok(Recommendation {
        models: plan.models,
        bytes,
    })
}
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub models_only: bool,
    pub runtime_only: bool,
    pub model_ids: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeRecord {
    pub server: PathBuf,
    pub backend: Backend,
    pub build: String,
}
pub fn saved_runtime(root: &Path) -> Option<RuntimeRecord> {
    let record: RuntimeRecord =
        storage::read_json(&root.join("active-ai-runtime.json"), 100_000).ok()?;
    record.server.is_file().then_some(record)
}

pub fn install(s: &Settings, options: &Options, c: &Control, log: &Activity) -> Result<Settings> {
    anyhow::ensure!(
        !(options.models_only && options.runtime_only),
        Fault::new("NG650", "Choose runtime-only or models-only")
    );
    c.check()?;
    let mut next = s.clone();
    log.work(
        Work::new(
            Phase::Runtime,
            0,
            usize::from(!options.models_only),
            15000,
            !options.models_only,
        ),
        "Checking the local AI runtime",
    );
    if !options.models_only {
        let record = install_runtime(s, c, log)?;
        next.server = record.server;
        next.backend = record.backend;
        log.work(
            Work::new(Phase::Runtime, 1, 1, 15000, false),
            format!("{:?} runtime verified", next.backend),
        );
    }
    if options.runtime_only {
        return Ok(next);
    }
    let hw = hardware::detect_for(&next, c);
    let chosen = if options.model_ids.is_empty() {
        recommend(&hw, &next)?.models
    } else {
        options
            .model_ids
            .iter()
            .map(|id| pinned(id).map(|p| p.0))
            .collect::<Result<Vec<_>>>()?
    };
    anyhow::ensure!(
        !chosen.is_empty(),
        Fault::new("NG650", "No models selected")
    );
    let (existing, _) = crate::runtime::inventory_available(&next.models_dir)?;
    let pairs = chosen
        .iter()
        .map(|m| pinned(&m.id))
        .collect::<Result<Vec<_>>>()?;
    let bytes: u64 = pairs
        .iter()
        .map(|(_, p)| p.weights.bytes + p.projector.bytes)
        .sum();
    let units = bytes.div_ceil(1024) as usize;
    log.work(
        Work::new(Phase::Download, 0, units, 2, false),
        "Preparing verified model downloads",
    );
    log.work(
        Work::new(Phase::Activate, 0, chosen.len(), 1500, false),
        "Preparing model registration",
    );
    let mut done = 0u64;
    for (index, (model, update)) in pairs.iter().enumerate() {
        c.check()?;
        // Keep user-updated and third-party registrations; never silently downgrade them.
        let healthy = if let Some(m) = existing
            .iter()
            .find(|m| m.id == model.id && m.endpoint.is_none())
        {
            let digest = storage::fingerprint(&(
                storage::hash_file(&m.weights, c)?,
                storage::hash_file(&m.projector, c)?,
            ))?;
            !m.sha256.is_empty() && digest == m.sha256
        } else {
            false
        };
        if !healthy {
            let log_outer = log.clone();
            let base = done;
            let projector = update.projector.file.clone();
            let weights_bytes = update.weights.bytes;
            let events = Activity {
                path: None,
                callback: Some(Arc::new(move |e| {
                    if e.stage == "download" {
                        let offset = if e.message.starts_with(&projector) {
                            weights_bytes
                        } else {
                            0
                        };
                        log_outer.work(
                            Work::new(
                                Phase::Download,
                                ((base + offset + e.done as u64) / 1024) as usize,
                                units,
                                2,
                                true,
                            ),
                            e.message.clone(),
                        );
                    }
                    log_outer.publish(e);
                })),
            };
            models::install(&next.models_dir, model, update, c, &events)?;
        } else {
            log.emit(
                "models",
                format!("{} already installed and checksum-verified", model.name),
                0,
                index,
                chosen.len(),
                "",
            );
        }
        done += update.weights.bytes + update.projector.bytes;
        log.work(
            Work::new(
                Phase::Download,
                done.div_ceil(1024) as usize,
                units,
                2,
                false,
            ),
            "Model files verified",
        );
        log.work(
            Work::new(Phase::Activate, index + 1, chosen.len(), 1500, false),
            format!("{} ready", model.name),
        );
    }
    c.check()?;
    Ok(next)
}

fn runtime_downloads(backend: &str) -> Vec<Download> {
    let assets: Vec<Download> = serde_json::from_str(include_str!("../runtime-downloads.json"))
        .expect("compiled runtime downloads");
    let marker = match backend {
        "cuda13" => "cuda-13.4",
        "cuda12" => "cuda-12.4",
        "vulkan" => "vulkan",
        _ => "cpu",
    };
    assets
        .into_iter()
        .filter(|a| a.file.contains(&format!("win-{marker}-x64")))
        .collect()
}
fn runtime_choices(backend: Backend, nvidia: bool, require: bool) -> Vec<&'static str> {
    let mut choices = match backend {
        Backend::Cpu => vec!["cpu"],
        Backend::Cuda => vec!["cuda13", "cuda12"],
        Backend::Vulkan => vec!["vulkan"],
        Backend::Auto if nvidia => vec!["cuda13", "cuda12", "vulkan"],
        Backend::Auto => vec!["vulkan"],
    };
    if !require && !choices.contains(&"cpu") {
        choices.push("cpu");
    }
    choices
}
fn install_runtime(s: &Settings, c: &Control, log: &Activity) -> Result<RuntimeRecord> {
    if !cfg!(windows) {
        anyhow::ensure!(
            s.server.is_file(),
            Fault::new(
                "NG650",
                "Automatic runtime installation supports Windows x64. Select a native llama-server or use models-only."
            )
        );
        return Ok(RuntimeRecord {
            server: s.server.clone(),
            backend: s.backend,
            build: "user-selected".into(),
        });
    }
    let root = s
        .server
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("runtime"));
    // Keep versioned runtime folders alongside the original runtime, never nested per upgrade.
    let root = if root
        .ancestors()
        .any(|p| p.file_name().is_some_and(|n| n == "ai-backends"))
    {
        root.ancestors()
            .find(|p| p.file_name().is_some_and(|n| n == "ai-backends"))
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    } else {
        root.to_path_buf()
    };
    fs::create_dir_all(&root)?;
    let root = root.canonicalize()?;
    let candidates = runtime_choices(
        s.backend,
        !hardware::detect().gpus.is_empty(),
        s.require_acceleration,
    );
    let mut failures = vec![];
    for backend in candidates {
        c.check()?;
        let result = (|| -> Result<RuntimeRecord> {
            let cache = root.join("downloads");
            fs::create_dir_all(&cache)?;
            let backends = root.join("ai-backends");
            fs::create_dir_all(&backends)?;
            let staging = tempfile::Builder::new()
                .prefix(&format!("{backend}-b11998-"))
                .tempdir_in(&backends)?;
            log.emit(
                "runtime",
                format!("Installing and verifying {backend} runtime"),
                0,
                0,
                0,
                "",
            );
            let assets = runtime_downloads(backend);
            anyhow::ensure!(
                !assets.is_empty(),
                Fault::new("NG650", "Runtime package metadata missing")
            );
            for a in assets {
                let archive = cache.join(&a.file);
                models::download(&a.url, &archive, &a.asset(), c, log)?;
                extract_runtime(&archive, staging.path(), c)?;
            }
            let server = staging.path().join("llama-server.exe");
            anyhow::ensure!(
                server.is_file(),
                Fault::new("NG650", "Runtime archive has no llama-server")
            );
            // App-local MSVC runtime accompanies the installed app and is needed in each new folder.
            for name in [
                "vcruntime140.dll",
                "vcruntime140_1.dll",
                "msvcp140.dll",
                "concrt140.dll",
            ] {
                let src = root.join(name);
                if src.is_file() {
                    fs::copy(src, staging.path().join(name))?;
                }
            }
            let (ok, out, err) = crate::scan::run(
                &server,
                &["--list-devices".into()],
                Duration::from_secs(30),
                c,
            )?;
            anyhow::ensure!(
                ok,
                Fault::new("NG650", format!("Runtime startup failed: {err}"))
            );
            let detected = hardware::parse_devices(&format!("{out}\n{err}"));
            let selected = if backend.starts_with("cuda") {
                Backend::Cuda
            } else if backend == "vulkan" {
                Backend::Vulkan
            } else {
                Backend::Cpu
            };
            anyhow::ensure!(
                selected == Backend::Cpu
                    || detected.iter().any(|g| g.backend
                        == if selected == Backend::Cuda {
                            "cuda"
                        } else {
                            "vulkan"
                        }),
                Fault::new(
                    "NG650",
                    "Runtime cannot access the requested GPU; trying another compatible backend"
                )
            );
            c.check()?;
            let folder = staging.keep();
            let record = RuntimeRecord {
                server: folder.join("llama-server.exe"),
                backend: selected,
                build: "b11998".into(),
            };
            storage::atomic_json(&root.join("active-ai-runtime.json"), &record)?;
            Ok(record)
        })();
        match result {
            Ok(record) => return Ok(record),
            Err(e) if e.downcast_ref::<Fault>().is_some_and(|f| f.code == "NG900") => {
                return Err(e);
            }
            Err(e) => {
                log.emit(
                    "runtime",
                    format!("{backend} unavailable: {e:#}"),
                    0,
                    0,
                    0,
                    "NG650",
                );
                failures.push(format!("{backend}: {e:#}"));
            }
        }
    }
    Err(Fault::new(
        "NG650",
        format!(
            "No compatible runtime could be installed. {}",
            failures.join("; ")
        ),
    )
    .into())
}

fn extract_runtime(archive: &Path, dir: &Path, c: &Control) -> Result<()> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive)?)?;
    anyhow::ensure!(
        zip.len() <= 1000,
        Fault::new("NG650", "Runtime archive has too many entries")
    );
    let mut expanded = 0u64;
    for i in 0..zip.len() {
        c.check()?;
        let mut entry = zip.by_index(i)?;
        let path = entry
            .enclosed_name()
            .ok_or_else(|| Fault::new("NG650", "Invalid runtime archive path"))?;
        anyhow::ensure!(
            !entry.is_symlink(),
            Fault::new("NG650", "Runtime archive contains a symlink")
        );
        if entry.is_dir() {
            continue;
        }
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        let lower = file.to_ascii_lowercase();
        if !(lower.ends_with(".dll")
            || lower == "llama-server.exe"
            || ["license", "copying", "notice"]
                .iter()
                .any(|s| lower.contains(s)))
        {
            continue;
        }
        expanded = expanded.saturating_add(entry.size());
        anyhow::ensure!(
            expanded <= 4_000_000_000 && entry.size() <= 2_000_000_000,
            Fault::new("NG650", "Runtime archive exceeds extraction limits")
        );
        let mut out = fs::File::create(dir.join(file))?;
        let mut bytes = [0u8; 65536];
        loop {
            c.check()?;
            let n = entry.read(&mut bytes)?;
            if n == 0 {
                break;
            }
            out.write_all(&bytes[..n])?;
        }
        out.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_bundled_models_have_immutable_matching_downloads() {
        for model in planning::catalog() {
            let (m, u) = pinned(&model.id).unwrap();
            assert_eq!(u.revision.len(), 40);
            assert_eq!(u.weights.sha256.len(), 64);
            assert_eq!(m.hub.as_ref().unwrap().weights, u.weights.file);
            assert!(u.weights.bytes > 0 && u.projector.bytes > 0);
        }
        assert!(pinned("../bad").is_err());
    }
    #[test]
    fn model_only_setup_reuses_verified_newer_readers_without_network_or_downgrade() {
        let dir = tempfile::tempdir().unwrap();
        let (mut m, _) = pinned("glm-ocr").unwrap();
        m.weights = dir.path().join("weights.gguf");
        m.projector = dir.path().join("projector.gguf");
        fs::write(&m.weights, b"GGUFconfirmed-weights").unwrap();
        fs::write(&m.projector, b"GGUFconfirmed-projector").unwrap();
        m.revision = "f".repeat(40);
        let registry = dir.path().join("models");
        let stored = models::save_custom(&registry, m, &Control::default()).unwrap();
        let s = Settings {
            models_dir: registry.clone(),
            backend: Backend::Cpu,
            server: dir.path().join("absent-server"),
            ..Default::default()
        };
        let next = install(
            &s,
            &Options {
                models_only: true,
                model_ids: vec!["glm-ocr".into()],
                ..Default::default()
            },
            &Control::default(),
            &Activity::default(),
        )
        .unwrap();
        assert_eq!(next.server, s.server);
        let found = crate::runtime::inventory(&registry).unwrap();
        assert_eq!(found[0].revision, stored.revision);
        assert!(!registry.join("glm-ocr.previous.json").exists());
        let c = Control::default();
        c.cancelled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(install(&s, &Options::default(), &c, &Activity::default()).is_err());
    }
    #[test]
    fn setup_recommendations_scale_models_with_available_memory() {
        let profile = |vram: u64| Hardware {
            ram_mib: 32768,
            free_ram_mib: 30000,
            logical_cores: 24,
            physical_cores: 16,
            gpus: vec![hardware::Gpu {
                index: 0,
                total_mib: vram,
                free_mib: vram,
                backend: "cuda".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let s = Settings {
            preset: Preset::Maximum,
            ..Default::default()
        };
        let small = recommend(&profile(4096), &s).unwrap();
        let large = recommend(&profile(32768), &s).unwrap();
        assert_eq!(small.models.len(), 3);
        assert_eq!(large.models.len(), 3);
        assert!(large.bytes > small.bytes);
        assert!(small.models.iter().all(|m| m.resident_mib < 4096));
        assert!(large.models.iter().any(|m| m.resident_mib > 16000));
    }
    #[test]
    fn runtime_fallback_respects_explicit_backend_and_acceleration() {
        assert_eq!(
            runtime_choices(Backend::Cuda, true, true),
            vec!["cuda13", "cuda12"]
        );
        assert_eq!(runtime_choices(Backend::Cpu, true, false), vec!["cpu"]);
        assert_eq!(runtime_downloads("cuda13").len(), 2);
        assert_eq!(runtime_downloads("cpu").len(), 1);
    }
    #[test]
    fn runtime_extraction_keeps_only_runtime_files_and_rejects_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("runtime.zip");
        let file = fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        for name in ["bin/llama-server.exe", "bin/ggml.dll", "bin/llama-cli.exe"] {
            zip.start_file(name, opts).unwrap();
            zip.write_all(b"runtime").unwrap();
        }
        zip.finish().unwrap();
        let output = dir.path().join("extracted");
        fs::create_dir(&output).unwrap();
        extract_runtime(&archive, &output, &Control::default()).unwrap();
        assert!(output.join("llama-server.exe").exists());
        assert!(!output.join("llama-cli.exe").exists());
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.start_file("../escape.dll", opts).unwrap();
        zip.write_all(b"bad").unwrap();
        zip.finish().unwrap();
        assert!(extract_runtime(&archive, &output, &Control::default()).is_err());
        assert!(!dir.path().join("escape.dll").exists());
    }
}
