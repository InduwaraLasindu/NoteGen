use crate::types::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub type Clarifier =
    Arc<dyn Fn(crate::assistance::Batch) -> Result<Vec<crate::assistance::Answer>> + Send + Sync>;
#[derive(Clone, Default)]
pub struct Control {
    pub cancelled: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
    pub clarify: Option<Clarifier>,
    pub downloads: Option<crate::downloads::Manager>,
    pub transfer_cancelled: Option<Arc<AtomicBool>>,
    /// Ephemeral, already-private workspace directory. Never persisted in settings.
    pub private_temp_root: Option<PathBuf>,
}
fn checked_temp_root(c: &Control) -> Result<Option<&Path>> {
    c.check()?;
    if let Some(root) = &c.private_temp_root {
        crate::watch::no_links(root)?;
        anyhow::ensure!(
            fs::metadata(root)?.is_dir(),
            Fault::new(
                "NG682",
                "Private temporary storage is unavailable; no global temporary fallback was used"
            )
        );
        Ok(Some(root))
    } else {
        Ok(None)
    }
}
pub fn temporary_dir(c: &Control, prefix: &str) -> Result<tempfile::TempDir> {
    anyhow::ensure!(
        !prefix.is_empty()
            && prefix.len() <= 64
            && prefix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
        Fault::new("NG682", "Invalid temporary workspace prefix")
    );
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    Ok(if let Some(root) = checked_temp_root(c)? {
        builder.tempdir_in(root)?
    } else {
        builder.tempdir()?
    })
}
pub fn temporary_file(c: &Control) -> Result<fs::File> {
    Ok(if let Some(root) = checked_temp_root(c)? {
        tempfile::tempfile_in(root)?
    } else {
        tempfile::tempfile()?
    })
}
/// Override native-child temporary storage without changing the parent process environment.
pub fn configure_temporary_environment(
    command: &mut std::process::Command,
    c: &Control,
) -> Result<()> {
    if let Some(root) = checked_temp_root(c)? {
        command
            .env("TEMP", root)
            .env("TMP", root)
            .env("TMPDIR", root);
    }
    Ok(())
}

impl Control {
    /// A nonblocking cancellation check for open HTTP responses. Pausing a
    /// transfer must drop its response before waiting, rather than hold a socket.
    pub fn check_cancelled(&self) -> Result<()> {
        anyhow::ensure!(
            !self.cancelled.load(Ordering::Relaxed)
                && !self
                    .transfer_cancelled
                    .as_ref()
                    .is_some_and(|flag| flag.load(Ordering::Relaxed)),
            Fault::new("NG900", "Cancelled; completed work is preserved")
        );
        Ok(())
    }
    pub fn check(&self) -> Result<()> {
        while self.paused.load(Ordering::Relaxed) {
            self.check_cancelled()?;
            thread::sleep(Duration::from_millis(50));
        }
        self.check_cancelled()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub timestamp: String,
    pub stage: String,
    pub message: String,
    pub page: usize,
    pub done: usize,
    pub total: usize,
    pub code: String,
    pub warning: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<crate::progress::Work>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<crate::telemetry::Preview>,
}
#[derive(Clone, Default)]
pub struct Activity {
    pub path: Option<PathBuf>,
    pub callback: Option<Arc<dyn Fn(Event) + Send + Sync>>,
}
impl Activity {
    pub fn work(&self, work: crate::progress::Work, message: impl Into<String>) {
        self.publish(Event {
            timestamp: chrono::Utc::now().to_rfc3339(),
            stage: "progress".into(),
            message: message.into(),
            page: 0,
            done: work.done as usize,
            total: work.total as usize,
            code: String::new(),
            warning: false,
            work: Some(work),
            preview: None,
        });
    }
    pub fn capture_response(&self, model: &str, page: usize, raw: &str) -> Result<()> {
        let Some(parent) = self.path.as_ref().and_then(|p| p.parent()) else {
            return Ok(());
        };
        let folder = parent.join("responses");
        fs::create_dir_all(&folder)?;
        let model = model
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let mut file = tempfile::Builder::new()
            .prefix(&format!("{model}-page-{page:05}-"))
            .suffix(".txt")
            .tempfile_in(&folder)?;
        file.write_all(raw.as_bytes())?;
        file.as_file().sync_all()?;
        let (_, path) = file.keep()?;
        self.emit(
            "evidence",
            format!("Raw reader response saved: {}", path.display()),
            page,
            0,
            0,
            "",
        );
        Ok(())
    }
    pub fn emit(
        &self,
        stage: &str,
        message: impl Into<String>,
        page: usize,
        done: usize,
        total: usize,
        code: &str,
    ) {
        let e = Event {
            timestamp: chrono::Utc::now().to_rfc3339(),
            stage: stage.into(),
            message: message.into(),
            page,
            done,
            total,
            code: code.into(),
            warning: !code.is_empty(),
            work: None,
            preview: None,
        };
        self.publish(e);
    }
    pub(crate) fn publish(&self, e: Event) {
        if let Some(p) = &self.path {
            if let Some(d) = p.parent() {
                let _ = fs::create_dir_all(d);
            }
            if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(p) {
                let _ = writeln!(f, "{}", serde_json::to_string(&e).unwrap_or_default());
            }
        }
        if let Some(c) = &self.callback {
            c(e);
        }
    }
    /// Draft previews are ephemeral UI data, not activity-log transcripts.
    pub fn preview(&self, preview: crate::telemetry::Preview) {
        if let Some(callback) = &self.callback {
            callback(Event {
                timestamp: chrono::Utc::now().to_rfc3339(),
                stage: "preview".into(),
                message: String::new(),
                page: preview.page,
                done: 0,
                total: 0,
                code: String::new(),
                warning: false,
                work: None,
                preview: Some(preview),
            });
        }
    }
}
pub fn hash_file(path: &Path, c: &Control) -> Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut b = vec![0u8; 1024 * 1024];
    loop {
        c.check()?;
        let n = f.read(&mut b)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(hex::encode(h.finalize()))
}
pub fn fingerprint<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}
pub fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut f = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut f, value)?;
    f.flush()?;
    f.as_file().sync_all()?;
    f.persist(path).map_err(|e| e.error)?;
    Ok(())
}
/// Create a new JSON artifact without a race that could replace an existing file.
pub fn protected_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path)
        .map_err(|_| Fault::new("NG401", "Output exists or its folder is unavailable"))?;
    Ok(())
}
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> Result<T> {
    let mut file = fs::File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.len() <= limit,
        Fault::new("NG500", "JSON file exceeds its size limit")
    );
    let mut bytes = Vec::new();
    (&mut file).take(limit + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= limit,
        Fault::new("NG500", "JSON file exceeds its size limit")
    );
    Ok(serde_json::from_slice(&bytes)?)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub input_sha256: String,
    pub fingerprint: String,
    pub pages: Vec<usize>,
    pub models: Vec<String>,
    pub settings: Settings,
}
#[derive(Clone, Debug)]
pub struct Project {
    pub root: PathBuf,
    pub manifest: Manifest,
}
impl Project {
    pub fn open(root: PathBuf, manifest: Manifest) -> Result<Self> {
        fs::create_dir_all(&root)?;
        crate::migrations::recover(&root, &Control::default())?;
        let p = root.join("project.json");
        if p.exists() {
            let old: Manifest = read_json(&p, 1_000_000)?;
            anyhow::ensure!(
                old.schema == manifest.schema
                    && old.input_sha256 == manifest.input_sha256
                    && old.fingerprint == manifest.fingerprint
                    && old.pages == manifest.pages,
                Fault::new("NG500", "Project input, models or settings do not match")
            );
        } else {
            atomic_json(&p, &manifest)?;
        }
        Ok(Self { root, manifest })
    }
    pub fn read(root: PathBuf) -> Result<Self> {
        crate::migrations::recover(&root, &Control::default())?;
        let m: Manifest = read_json(&root.join("project.json"), 1_000_000)?;
        anyhow::ensure!(
            m.schema == 2,
            Fault::new("NG500", "Unsupported project schema")
        );
        anyhow::ensure!(
            m.pages.len() <= 3000
                && m.pages.iter().all(|n| (1..=3000).contains(n))
                && m.pages
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    == m.pages.len(),
            Fault::new("NG500", "Invalid project page list")
        );
        m.settings.validate()?;
        let project = Self { root, manifest: m };
        crate::review::recover(&project)?;
        Ok(project)
    }
    pub fn image(&self, n: usize) -> PathBuf {
        self.root.join(format!("page-{n:05}.png"))
    }
    pub fn page_path(&self, n: usize) -> PathBuf {
        self.root.join(format!("page-{n:05}.json"))
    }
    pub fn load(&self, n: usize) -> Result<Page> {
        self.ensure_current()?;
        let manual = self.root.join(format!("page-{n:05}.edits.json"));
        let path = if manual.is_file() {
            manual
        } else {
            self.page_path(n)
        };
        let p: Page = read_json(&path, 32_000_000)?;
        anyhow::ensure!(
            p.page == n && p.image == format!("page-{n:05}.png"),
            Fault::new("NG500", "Checkpoint page or source path mismatch")
        );
        p.validate()?;
        Ok(p)
    }
    pub fn save(&self, p: &Page) -> Result<()> {
        self.ensure_current()?;
        anyhow::ensure!(
            self.manifest.pages.contains(&p.page) && p.image == format!("page-{:05}.png", p.page),
            Fault::new("NG500", "Checkpoint page or source path mismatch")
        );
        p.validate()?;
        let path = if p.edited || p.reviewed {
            self.root.join(format!("page-{:05}.edits.json", p.page))
        } else {
            self.page_path(p.page)
        };
        atomic_json(&path, p)
    }
    pub fn ensure_current(&self) -> Result<()> {
        crate::migrations::ensure_readable(&self.root)?;
        let current: Manifest = read_json(&self.root.join("project.json"), 1_000_000)?;
        anyhow::ensure!(
            current.schema == self.manifest.schema
                && current.input_sha256 == self.manifest.input_sha256
                && current.fingerprint == self.manifest.fingerprint
                && current.pages == self.manifest.pages
                && current.models == self.manifest.models,
            Fault::new(
                "NG500",
                "Project layout or identity changed; reopen before reading or saving"
            )
        );
        Ok(())
    }
}

#[cfg(test)]
mod private_temp_tests {
    use super::*;
    #[test]
    fn temporary_artifacts_and_child_environment_stay_under_the_selected_private_root() {
        let root = tempfile::tempdir().unwrap();
        let control = Control {
            private_temp_root: Some(root.path().into()),
            ..Default::default()
        };
        let directory = temporary_dir(&control, "notegen-private-").unwrap();
        assert_eq!(directory.path().parent(), Some(root.path()));
        let mut file = temporary_file(&control).unwrap();
        file.write_all(b"private stdout").unwrap();
        assert_eq!(file.metadata().unwrap().len(), 14);
        let previous = std::env::var_os("TEMP");
        let mut command = std::process::Command::new("unused-native-command");
        configure_temporary_environment(&mut command, &control).unwrap();
        for name in ["TEMP", "TMP", "TMPDIR"] {
            let value = command
                .get_envs()
                .find(|(key, _)| *key == name)
                .and_then(|(_, value)| value)
                .unwrap();
            assert_eq!(Path::new(value), root.path());
        }
        assert_eq!(std::env::var_os("TEMP"), previous);
    }
    #[test]
    fn missing_cancelled_or_invalid_private_temp_requests_never_fall_back() {
        let root = tempfile::tempdir().unwrap();
        let control = Control {
            private_temp_root: Some(root.path().join("missing")),
            ..Default::default()
        };
        assert!(temporary_dir(&control, "private-").is_err());
        assert!(temporary_file(&control).is_err());
        assert!(
            configure_temporary_environment(&mut std::process::Command::new("unused"), &control)
                .is_err()
        );
        assert!(!root.path().join("missing").exists());
        let control = Control {
            private_temp_root: Some(root.path().into()),
            ..Default::default()
        };
        assert!(temporary_dir(&control, "../outside").is_err());
        control.cancelled.store(true, Ordering::Relaxed);
        assert!(temporary_dir(&control, "private-").is_err());
        assert!(temporary_file(&control).is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
