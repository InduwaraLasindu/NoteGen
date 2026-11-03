//! Explicit local folder polling. A scan never marks a file imported.
use crate::{
    storage::{Control, fingerprint, hash_file},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DuplicatePolicy {
    #[default]
    Skip,
    NewRevision,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchConfig {
    pub folder: PathBuf,
    pub extensions: Vec<String>,
    pub stable_seconds: u64,
    pub paused: bool,
    pub duplicate_policy: DuplicatePolicy,
    pub max_file_bytes: u64,
}
impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            folder: "imports".into(),
            extensions: vec![
                "pdf".into(),
                "png".into(),
                "jpg".into(),
                "jpeg".into(),
                "tif".into(),
                "tiff".into(),
            ],
            stable_seconds: 3,
            paused: false,
            duplicate_policy: DuplicatePolicy::Skip,
            max_file_bytes: 1_000_000_000,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub bytes: u64,
    pub modified_nanos: u128,
    pub stable_since: i64,
    pub sha256: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportedFile {
    pub filename: String,
    pub sha256: String,
    pub bytes: u64,
    pub modified_nanos: u128,
    pub project_id: String,
    pub imported_at: i64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchState {
    pub schema: u32,
    pub folder_identity: String,
    pub observations: BTreeMap<String, Observation>,
    pub imported: BTreeMap<String, ImportedFile>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportCandidate {
    pub path: PathBuf,
    pub filename: String,
    pub sha256: String,
    pub bytes: u64,
    pub modified_nanos: u128,
    pub duplicate_of: Option<ImportedFile>,
    pub observation_token: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RejectedFile {
    pub filename: String,
    pub reason: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScanReport {
    pub paused: bool,
    pub ready: Vec<ImportCandidate>,
    pub waiting_stable: Vec<String>,
    pub skipped_duplicates: Vec<String>,
    pub rejected: Vec<RejectedFile>,
}
pub(crate) fn no_links(path: &Path) -> Result<()> {
    for ancestor in std::path::absolute(path)?.ancestors() {
        let metadata = match fs::symlink_metadata(ancestor) {
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
                "NG100",
                "Watched/cache paths cannot contain symlinks or junctions"
            )
        );
    }
    Ok(())
}
fn folder(config: &WatchConfig) -> Result<PathBuf> {
    anyhow::ensure!(
        config.stable_seconds <= 86400
            && (1..=10_000_000_000).contains(&config.max_file_bytes)
            && !config.extensions.is_empty()
            && config.extensions.len() <= 6
            && config
                .extensions
                .iter()
                .all(|s| ["pdf", "png", "jpg", "jpeg", "tif", "tiff"].contains(&s.as_str())),
        Fault::new(
            "NG102",
            "Invalid watched-folder file types, stability time or file-size limit"
        )
    );
    no_links(&config.folder)?;
    let folder = config.folder.canonicalize()?;
    anyhow::ensure!(
        folder.is_dir(),
        Fault::new("NG100", "Choose an existing import folder")
    );
    Ok(folder)
}
fn signature(path: &Path) -> Result<(u64, u128)> {
    let metadata = fs::metadata(path)?;
    anyhow::ensure!(
        metadata.is_file(),
        Fault::new("NG100", "Import candidate is not a regular file")
    );
    Ok((
        metadata.len(),
        metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos(),
    ))
}
fn token(identity: &str, filename: &str, sha: &str, bytes: u64, modified: u128) -> Result<String> {
    fingerprint(&(identity, filename, sha, bytes, modified))
}
fn bind(state: &mut WatchState, folder: &Path) -> Result<()> {
    let identity = fingerprint(&folder.to_string_lossy())?;
    if state.schema == 0 {
        anyhow::ensure!(
            state.observations.is_empty() && state.imported.is_empty(),
            Fault::new(
                "NG500",
                "Uninitialized watch state already contains records"
            )
        );
        state.schema = 1;
        state.folder_identity = identity.clone();
    }
    anyhow::ensure!(
        state.schema == 1
            && state.folder_identity == identity
            && state.observations.len() <= 10000
            && state.imported.len() <= 10000,
        Fault::new(
            "NG500",
            "Watch state belongs to another folder or exceeds its record limit"
        )
    );
    Ok(())
}
pub fn scan(
    config: &WatchConfig,
    state: &mut WatchState,
    now: i64,
    c: &Control,
) -> Result<ScanReport> {
    c.check()?;
    let root = folder(config)?;
    bind(state, &root)?;
    if config.paused {
        return Ok(ScanReport {
            paused: true,
            ..Default::default()
        });
    }
    let mut report = ScanReport::default();
    let mut offered = BTreeSet::new();
    let mut present = BTreeSet::new();
    let mut entries = fs::read_dir(&root)?.collect::<std::io::Result<Vec<_>>>()?;
    anyhow::ensure!(
        entries.len() <= 10000,
        Fault::new(
            "NG100",
            "Watched folder exceeds 10,000 entries; choose a smaller folder"
        )
    );
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        c.check()?;
        let filename = entry.file_name().to_string_lossy().to_string();
        if entry.file_name().to_str().is_none() {
            report.rejected.push(RejectedFile {
                filename,
                reason: "Filename cannot be represented as Unicode; rename it before importing"
                    .into(),
            });
            continue;
        }
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            continue;
        }
        if path
            .extension()
            .and_then(|s| s.to_str())
            .is_none_or(|s| !config.extensions.iter().any(|e| e.eq_ignore_ascii_case(s)))
        {
            continue;
        }
        if let Err(error) = no_links(&path) {
            report.rejected.push(RejectedFile {
                filename,
                reason: error.to_string(),
            });
            continue;
        }
        let (bytes, modified) = match signature(&path) {
            Ok(v) => v,
            Err(error) => {
                report.rejected.push(RejectedFile {
                    filename,
                    reason: error.to_string(),
                });
                continue;
            }
        };
        if bytes == 0 || bytes > config.max_file_bytes {
            report.rejected.push(RejectedFile {
                filename,
                reason: "File is empty or exceeds the explicit import size limit".into(),
            });
            continue;
        }
        present.insert(filename.clone());
        let observation = state
            .observations
            .entry(filename.clone())
            .or_insert(Observation {
                bytes,
                modified_nanos: modified,
                stable_since: now,
                sha256: None,
            });
        if observation.bytes != bytes
            || observation.modified_nanos != modified
            || now < observation.stable_since
        {
            *observation = Observation {
                bytes,
                modified_nanos: modified,
                stable_since: now,
                sha256: None,
            };
        }
        if now.saturating_sub(observation.stable_since) < config.stable_seconds as i64 {
            report.waiting_stable.push(filename);
            continue;
        }
        // Copy/restore tools may preserve both length and modification time.
        // Revalidate stable bytes rather than treating metadata or a cached hash as identity.
        let sha = match hash_file(&path, c) {
            Ok(sha) => sha,
            Err(error) => {
                c.check()?;
                report.rejected.push(RejectedFile {
                    filename,
                    reason: error.to_string(),
                });
                continue;
            }
        };
        let after = match signature(&path) {
            Ok(value) => value,
            Err(error) => {
                report.rejected.push(RejectedFile {
                    filename,
                    reason: error.to_string(),
                });
                continue;
            }
        };
        if after != (bytes, modified) {
            *observation = Observation {
                bytes: after.0,
                modified_nanos: after.1,
                stable_since: now,
                sha256: None,
            };
            report.waiting_stable.push(filename);
            continue;
        }
        if observation.sha256.as_ref().is_some_and(|old| old != &sha) {
            observation.stable_since = now;
            observation.sha256 = Some(sha.clone());
            if config.stable_seconds > 0 {
                report.waiting_stable.push(filename);
                continue;
            }
        }
        observation.sha256 = Some(sha.clone());
        if state
            .imported
            .get(&filename)
            .is_some_and(|r| r.sha256 == sha && r.bytes == bytes && r.modified_nanos == modified)
        {
            continue;
        }
        let duplicate = state.imported.values().find(|r| r.sha256 == sha).cloned();
        if (duplicate.is_some() || offered.contains(&sha))
            && config.duplicate_policy == DuplicatePolicy::Skip
        {
            report.skipped_duplicates.push(filename);
            continue;
        }
        offered.insert(sha.clone());
        report.ready.push(ImportCandidate {
            path,
            filename: filename.clone(),
            sha256: sha.clone(),
            bytes,
            modified_nanos: modified,
            duplicate_of: duplicate,
            observation_token: token(&state.folder_identity, &filename, &sha, bytes, modified)?,
        });
    }
    state.observations.retain(|name, _| present.contains(name));
    Ok(report)
}
/// Call only after queue admission or successful conversion. Failed admission
/// leaves the candidate ready on the next poll instead of silently consuming it.
pub fn record_import(
    config: &WatchConfig,
    state: &mut WatchState,
    candidate: &ImportCandidate,
    project_id: &str,
    now: i64,
    c: &Control,
) -> Result<()> {
    c.check()?;
    anyhow::ensure!(
        !config.paused,
        Fault::new("NG100", "Watched import is paused")
    );
    anyhow::ensure!(
        !project_id.trim().is_empty() && project_id.len() <= 200,
        Fault::new("NG500", "Enter the admitted job/project identifier")
    );
    let root = folder(config)?;
    bind(state, &root)?;
    anyhow::ensure!(
        candidate.bytes <= config.max_file_bytes
            && candidate
                .path
                .extension()
                .and_then(|v| v.to_str())
                .is_some_and(|v| config.extensions.iter().any(|e| e.eq_ignore_ascii_case(v))),
        Fault::new(
            "NG500",
            "Watch import rules changed after the candidate preview"
        )
    );
    no_links(&candidate.path)?;
    anyhow::ensure!(
        candidate.path.canonicalize()?.parent() == Some(root.as_path())
            && candidate.path.file_name().and_then(|v| v.to_str())
                == Some(candidate.filename.as_str()),
        Fault::new("NG100", "Import candidate escaped the watched folder")
    );
    let observed = state.observations.get(&candidate.filename).ok_or_else(|| {
        Fault::new(
            "NG500",
            "Import candidate was not observed; scan the folder again",
        )
    })?;
    anyhow::ensure!(
        observed.bytes == candidate.bytes
            && observed.modified_nanos == candidate.modified_nanos
            && observed.sha256.as_deref() == Some(candidate.sha256.as_str())
            && now.saturating_sub(observed.stable_since) >= config.stable_seconds as i64
            && candidate.observation_token
                == token(
                    &state.folder_identity,
                    &candidate.filename,
                    &candidate.sha256,
                    candidate.bytes,
                    candidate.modified_nanos
                )?,
        Fault::new("NG500", "Import candidate is stale or was not stable")
    );
    anyhow::ensure!(
        signature(&candidate.path)? == (candidate.bytes, candidate.modified_nanos)
            && hash_file(&candidate.path, c)? == candidate.sha256
            && signature(&candidate.path)? == (candidate.bytes, candidate.modified_nanos),
        Fault::new(
            "NG500",
            "Watched file changed after queue preview; it was not marked imported"
        )
    );
    anyhow::ensure!(
        state.imported.contains_key(&candidate.filename) || state.imported.len() < 10000,
        Fault::new(
            "NG500",
            "Watch import history reached 10,000 records; preserve/export it before starting fresh state"
        )
    );
    state.imported.insert(
        candidate.filename.clone(),
        ImportedFile {
            filename: candidate.filename.clone(),
            sha256: candidate.sha256.clone(),
            bytes: candidate.bytes,
            modified_nanos: candidate.modified_nanos,
            project_id: project_id.into(),
            imported_at: now,
        },
    );
    Ok(())
}
