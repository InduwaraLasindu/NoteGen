//! Previewed cleanup of verified runtime-download caches only. Never a data cleaner.
use crate::{
    setup,
    storage::{Control, fingerprint, hash_file, read_json},
    types::*,
    watch::no_links,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub modified_nanos: u128,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CleanupPreview {
    pub files: Vec<CacheFile>,
    pub reclaimable_bytes: u64,
    pub protected_roots: Vec<PathBuf>,
    pub token: String,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkippedFile {
    pub path: PathBuf,
    pub reason: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CleanupReport {
    pub removed: Vec<PathBuf>,
    pub removed_bytes: u64,
    pub skipped: Vec<SkippedFile>,
    pub cancelled: bool,
}
fn absolute(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        Ok(path.canonicalize()?)
    } else {
        Ok(std::path::absolute(path)?)
    }
}
fn protected(settings: &Settings, project_roots: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut paths = vec![
        absolute(&settings.writer.root)?,
        absolute(&settings.server)?,
    ];
    for root in project_roots {
        paths.push(absolute(root)?);
    }
    if settings.models_dir.exists() {
        no_links(&settings.models_dir)?;
        let entries = fs::read_dir(&settings.models_dir)?.collect::<std::io::Result<Vec<_>>>()?;
        anyhow::ensure!(
            entries.len() <= 10000,
            Fault::new(
                "NG650",
                "Too many model registry files for a bounded cleanup preview"
            )
        );
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if !(name.ends_with(".notegen.json") || name.ends_with(".previous.json")) {
                continue;
            }
            no_links(&path)?;
            let model: crate::planning::Model = read_json(&path, 100_000)?;
            crate::models::validate_model(&model)?;
            if model.endpoint.is_none() {
                for asset in [&model.weights, &model.projector] {
                    let full = if asset.is_absolute() {
                        asset.clone()
                    } else {
                        settings.models_dir.join(asset)
                    };
                    paths.push(absolute(&full)?);
                }
            }
        }
    }
    let runtime = setup::runtime_root(settings);
    for name in ["active-ai-runtime.json", "previous-ai-runtime.json"] {
        let path = runtime.join(name);
        if path.is_file() {
            no_links(&path)?;
            let record: setup::RuntimeRecord = read_json(&path, 100_000)?;
            let server = absolute(&record.server)?;
            if let Some(parent) = server.parent() {
                paths.push(parent.into());
            }
            paths.push(server);
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}
fn is_protected(path: &Path, protected: &[PathBuf]) -> bool {
    protected.iter().any(|p| path == p || path.starts_with(p))
}
fn pins() -> Result<Vec<setup::Download>> {
    Ok(serde_json::from_str(include_str!(
        "../runtime-downloads.json"
    ))?)
}
fn preview_with_pins(
    settings: &Settings,
    project_roots: &[PathBuf],
    pins: &[setup::Download],
    c: &Control,
) -> Result<CleanupPreview> {
    c.check()?;
    let protected_roots = protected(settings, project_roots)?;
    let cache = setup::runtime_root(settings).join("downloads");
    no_links(&cache)?;
    let mut files = vec![];
    if cache.exists() {
        let root = cache.canonicalize()?;
        anyhow::ensure!(
            root.is_dir(),
            Fault::new("NG650", "Runtime cache is not a directory")
        );
        for pin in pins {
            c.check()?;
            anyhow::ensure!(
                !pin.file.contains(['/', '\\', ':']) && pin.file.ends_with(".zip"),
                Fault::new("NG650", "Invalid known runtime cache filename")
            );
            let path = root.join(&pin.file);
            if !path.exists() {
                continue;
            }
            no_links(&path)?;
            let canonical = path.canonicalize()?;
            anyhow::ensure!(
                canonical.parent() == Some(root.as_path()),
                Fault::new("NG650", "Cache file escaped its folder")
            );
            if is_protected(&canonical, &protected_roots) {
                continue;
            }
            let metadata = path.metadata()?;
            if !metadata.is_file() || metadata.len() != pin.bytes {
                continue;
            }
            let hash = hash_file(&path, c)?;
            if hash != pin.sha256 {
                continue;
            }
            files.push(CacheFile {
                path: canonical,
                bytes: metadata.len(),
                sha256: hash,
                modified_nanos: metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos(),
            });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    let bytes = files.iter().map(|f| f.bytes).sum();
    let token = fingerprint(&(&files, &protected_roots))?;
    Ok(CleanupPreview{files,reclaimable_bytes:bytes,protected_roots,token,note:"Only complete runtime-download ZIPs matching this build's known publisher size and SHA256 are reclaimable. Unknown files, partial downloads, GGUF weights, model/runtime versions, rollback registrations, writer data and all supplied project roots are preserved. Removing a cached archive may require downloading it again for a future reinstall. No folders are deleted.".into()})
}
pub fn preview(
    settings: &Settings,
    project_roots: &[PathBuf],
    c: &Control,
) -> Result<CleanupPreview> {
    preview_with_pins(settings, project_roots, &pins()?, c)
}
fn apply_with_pins(
    settings: &Settings,
    project_roots: &[PathBuf],
    selection: &[PathBuf],
    token: &str,
    pins: &[setup::Download],
    c: &Control,
) -> Result<CleanupReport> {
    c.check()?;
    let chosen = selection.iter().collect::<BTreeSet<_>>();
    anyhow::ensure!(
        !chosen.is_empty() && chosen.len() == selection.len(),
        Fault::new("NG650", "Choose distinct files from a cleanup preview")
    );
    let current = preview_with_pins(settings, project_roots, pins, c)?;
    anyhow::ensure!(
        token == current.token,
        Fault::new(
            "NG650",
            "Cleanup preview is stale; inspect a fresh preview before removing files"
        )
    );
    anyhow::ensure!(
        chosen
            .iter()
            .all(|p| current.files.iter().any(|f| &f.path == *p)),
        Fault::new(
            "NG650",
            "Selection contains a file outside the verified cleanup preview"
        )
    );
    let mut report = CleanupReport::default();
    for file in current.files.iter().filter(|f| chosen.contains(&f.path)) {
        if c.check().is_err() {
            report.cancelled = true;
            break;
        }
        let operation = || -> Result<()> {
            no_links(&file.path)?;
            let fresh_protection = protected(settings, project_roots)?;
            anyhow::ensure!(
                !is_protected(&file.path, &fresh_protection),
                Fault::new(
                    "NG650",
                    "File became active, pinned or protected after the preview"
                )
            );
            let metadata = file.path.metadata()?;
            anyhow::ensure!(
                metadata.is_file()
                    && metadata.len() == file.bytes
                    && metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos()
                        == file.modified_nanos
                    && hash_file(&file.path, c)? == file.sha256,
                Fault::new("NG650", "Cache file changed after the preview")
            );
            fs::remove_file(&file.path)?;
            Ok(())
        };
        match operation() {
            Ok(()) => {
                report.removed.push(file.path.clone());
                report.removed_bytes += file.bytes;
            }
            Err(error) => report.skipped.push(SkippedFile {
                path: file.path.clone(),
                reason: error.to_string(),
            }),
        }
    }
    Ok(report)
}
pub fn apply(
    settings: &Settings,
    project_roots: &[PathBuf],
    selection: &[PathBuf],
    token: &str,
    c: &Control,
) -> Result<CleanupReport> {
    apply_with_pins(settings, project_roots, selection, token, &pins()?, c)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> (Settings, Vec<setup::Download>, PathBuf) {
        let runtime = root.join("runtime");
        let cache = runtime.join("downloads");
        fs::create_dir_all(&cache).unwrap();
        let path = cache.join("llama-fixture.zip");
        fs::write(&path, b"PK verified fixture cache").unwrap();
        let pin = setup::Download {
            file: "llama-fixture.zip".into(),
            bytes: path.metadata().unwrap().len(),
            sha256: hash_file(&path, &Control::default()).unwrap(),
            url: "https://github.com/fixture".into(),
        };
        let settings = Settings {
            server: runtime.join("llama-server.exe"),
            models_dir: root.join("models"),
            ..Default::default()
        };
        (settings, vec![pin], path)
    }
    #[test]
    fn selected_verified_cache_file_is_removed_and_other_files_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let (settings, pins, path) = fixture(dir.path());
        fs::write(
            path.parent().unwrap().join("user-notes.png"),
            b"private scan",
        )
        .unwrap();
        fs::write(path.parent().unwrap().join("partial.zip.part"), b"partial").unwrap();
        let preview = preview_with_pins(&settings, &[], &pins, &Control::default()).unwrap();
        assert_eq!(preview.files.len(), 1);
        assert_eq!(preview.reclaimable_bytes, pins[0].bytes);
        let report = apply_with_pins(
            &settings,
            &[],
            &[path.canonicalize().unwrap()],
            &preview.token,
            &pins,
            &Control::default(),
        )
        .unwrap();
        assert_eq!(report.removed.len(), 1);
        assert!(!path.exists());
        assert!(path.parent().unwrap().join("user-notes.png").exists());
        assert!(path.parent().unwrap().join("partial.zip.part").exists());
    }
    #[test]
    fn stale_token_and_project_or_active_runtime_protection_prevent_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let (settings, pins, path) = fixture(dir.path());
        let c = Control::default();
        let preview = preview_with_pins(&settings, &[], &pins, &c).unwrap();
        let selection = vec![path.canonicalize().unwrap()];
        fs::write(&path, b"changed cache").unwrap();
        assert!(apply_with_pins(&settings, &[], &selection, &preview.token, &pins, &c).is_err());
        assert!(path.exists());
        fs::write(&path, b"PK verified fixture cache").unwrap();
        assert!(
            preview_with_pins(&settings, &[path.parent().unwrap().into()], &pins, &c)
                .unwrap()
                .files
                .is_empty()
        );
        let record = setup::RuntimeRecord {
            server: path.clone(),
            backend: Backend::Cpu,
            build: "fixture".into(),
        };
        crate::storage::atomic_json(
            &setup::runtime_root(&settings).join("active-ai-runtime.json"),
            &record,
        )
        .unwrap();
        assert!(
            preview_with_pins(&settings, &[], &pins, &c)
                .unwrap()
                .files
                .is_empty()
        );
    }
    #[test]
    fn rollback_model_asset_pins_and_current_writer_roots_are_protected() {
        let dir = tempfile::tempdir().unwrap();
        let (mut settings, pins, path) = fixture(dir.path());
        fs::create_dir_all(&settings.models_dir).unwrap();
        let (mut model, _) = setup::pinned("glm-ocr").unwrap();
        model.weights = path.clone();
        model.projector = path.clone();
        crate::storage::atomic_json(&settings.models_dir.join("glm-ocr.previous.json"), &model)
            .unwrap();
        assert!(
            preview_with_pins(&settings, &[], &pins, &Control::default())
                .unwrap()
                .files
                .is_empty()
        );
        fs::remove_file(settings.models_dir.join("glm-ocr.previous.json")).unwrap();
        settings.writer.root = path.parent().unwrap().into();
        assert!(
            preview_with_pins(&settings, &[], &pins, &Control::default())
                .unwrap()
                .files
                .is_empty()
        );
    }
}
