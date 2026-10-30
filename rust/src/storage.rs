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

#[derive(Clone, Default)]
pub struct Control {
    pub cancelled: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
}
impl Control {
    pub fn check(&self) -> Result<()> {
        while self.paused.load(Ordering::Relaxed) && !self.cancelled.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(50));
        }
        anyhow::ensure!(
            !self.cancelled.load(Ordering::Relaxed),
            Fault::new("NG900", "Cancelled; completed work is preserved")
        );
        Ok(())
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
}
#[derive(Clone, Default)]
pub struct Activity {
    pub path: Option<PathBuf>,
    pub callback: Option<Arc<dyn Fn(Event) + Send + Sync>>,
}
impl Activity {
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
        };
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
pub fn fingerprint<T: Serialize>(value: &T) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}
pub fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut f = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut f, value)?;
    f.flush()?;
    f.as_file().sync_all()?;
    f.persist(path).map_err(|e| e.error)?;
    Ok(())
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
        let p = root.join("project.json");
        if p.exists() {
            let old: Manifest = serde_json::from_reader(fs::File::open(&p)?)?;
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
        let m: Manifest = serde_json::from_reader(fs::File::open(root.join("project.json"))?)?;
        anyhow::ensure!(
            m.schema == 2,
            Fault::new("NG500", "Unsupported project schema")
        );
        Ok(Self { root, manifest: m })
    }
    pub fn image(&self, n: usize) -> PathBuf {
        self.root.join(format!("page-{n:05}.png"))
    }
    pub fn page_path(&self, n: usize) -> PathBuf {
        self.root.join(format!("page-{n:05}.json"))
    }
    pub fn load(&self, n: usize) -> Result<Page> {
        let p: Page = serde_json::from_reader(fs::File::open(self.page_path(n))?)?;
        anyhow::ensure!(
            p.page == n && p.image == format!("page-{n:05}.png"),
            Fault::new("NG500", "Checkpoint page or source path mismatch")
        );
        p.content.validate()?;
        Ok(p)
    }
    pub fn save(&self, p: &Page) -> Result<()> {
        p.content.validate()?;
        atomic_json(&self.page_path(p.page), p)
    }
}
