//! Sequential local watch worker. Run on a CLI/worker thread, never the GUI thread.
//! The pause marker/configuration pauses new imports; Control also pauses/cancels
//! an active conversion. No external automation or background installation occurs.
use crate::{
    storage::{self, Activity, Control, atomic_json, fingerprint, read_json},
    types::*,
    watch::{self, ImportCandidate, WatchConfig, WatchState},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    thread,
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Failure {
    filename: String,
    sha256: String,
    attempts: u32,
    retry_after: i64,
    error: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ServiceState {
    schema: u32,
    kind: String,
    watch: WatchState,
    failures: BTreeMap<String, Failure>,
}
impl Default for ServiceState {
    fn default() -> Self {
        Self {
            schema: 1,
            kind: "notegen_watch_service".into(),
            watch: WatchState::default(),
            failures: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WatchSummary {
    pub converted: u64,
    pub failed_attempts: u64,
    pub skipped_duplicate_files: u64,
    #[serde(default)]
    pub duplicate_summary_truncated: bool,
    pub waiting_stable: usize,
    pub rejected_files: usize,
    pub cancelled: bool,
    pub output_files: Vec<PathBuf>,
    pub note: String,
}
const MAX_SUMMARY_NAMES: usize = 10_000;
fn observe_duplicates(
    names: &mut BTreeSet<String>,
    incoming: impl IntoIterator<Item = String>,
    summary: &mut WatchSummary,
) {
    let was_truncated = summary.duplicate_summary_truncated;
    for name in incoming {
        if names.contains(&name) {
            continue;
        }
        if names.len() < MAX_SUMMARY_NAMES {
            names.insert(name);
        } else {
            summary.duplicate_summary_truncated = true;
        }
    }
    summary.skipped_duplicate_files = names.len() as u64;
    if summary.duplicate_summary_truncated && !was_truncated {
        if !summary.note.is_empty() {
            summary.note.push('\n');
        }
        summary.note.push_str("Duplicate-file summary is capped at 10,000 distinct filenames; additional skipped duplicate filenames were observed. Import receipts remain complete and separate.");
    }
}
fn refresh_rejections(
    saved: &mut BTreeMap<String, String>,
    current: &[watch::RejectedFile],
) -> Vec<watch::RejectedFile> {
    let current_names = current
        .iter()
        .take(MAX_SUMMARY_NAMES)
        .map(|rejected| rejected.filename.as_str())
        .collect::<BTreeSet<_>>();
    saved.retain(|name, _| current_names.contains(name.as_str()));
    let mut changed = vec![];
    for rejected in current.iter().take(MAX_SUMMARY_NAMES) {
        if saved.get(&rejected.filename) != Some(&rejected.reason) {
            saved.insert(rejected.filename.clone(), rejected.reason.clone());
            changed.push(rejected.clone());
        }
    }
    changed
}
fn path_identity(path: &Path) -> Result<PathBuf> {
    watch::no_links(path)?;
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let absolute = std::path::absolute(path)?;
    let mut clean = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            _ => clean.push(part.as_os_str()),
        }
    }
    // Canonicalize the nearest existing ancestor so Windows verbatim prefixes
    // remain consistent when comparing existing and not-yet-created paths.
    let mut ancestor = clean.as_path();
    let mut missing = vec![];
    while !ancestor.exists() {
        missing.push(
            ancestor
                .file_name()
                .ok_or_else(|| Fault::new("NG102", "Cannot resolve watch output/state path"))?
                .to_os_string(),
        );
        ancestor = ancestor
            .parent()
            .ok_or_else(|| Fault::new("NG102", "Cannot resolve watch output/state parent"))?;
    }
    let mut result = ancestor.canonicalize()?;
    for component in missing.into_iter().rev() {
        result.push(component);
    }
    Ok(result)
}
fn read_config(path: &Path) -> Result<WatchConfig> {
    watch::no_links(path)?;
    anyhow::ensure!(
        path.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json")),
        Fault::new("NG102", "Watch configuration must be a .json file")
    );
    let value: serde_json::Value = read_json(path, 100_000)?;
    let object = value
        .as_object()
        .ok_or_else(|| Fault::new("NG102", "Watch configuration must be a JSON object"))?;
    anyhow::ensure!(
        object.contains_key("folder")
            && object.keys().all(|k| [
                "folder",
                "extensions",
                "stable_seconds",
                "paused",
                "duplicate_policy",
                "max_file_bytes"
            ]
            .contains(&k.as_str())),
        Fault::new(
            "NG102",
            "Watch configuration needs an explicit folder and only supported import rules"
        )
    );
    Ok(serde_json::from_value(value)?)
}
fn validate_layout(
    config_path: &Path,
    state_path: &Path,
    output_dir: &Path,
    config: &WatchConfig,
    pause_file: Option<&Path>,
) -> Result<()> {
    let source = path_identity(&config.folder)?;
    anyhow::ensure!(
        source.is_dir(),
        Fault::new("NG100", "Choose an existing watched import folder")
    );
    let configuration = path_identity(config_path)?;
    let state = path_identity(state_path)?;
    let output = path_identity(output_dir)?;
    anyhow::ensure!(
        state_path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
            && state != configuration
            && !state.starts_with(&output)
            && !configuration.starts_with(&output),
        Fault::new(
            "NG102",
            "Keep distinct JSON configuration/state files outside the conversion output folder"
        )
    );
    anyhow::ensure!(
        !output.starts_with(&source) && !source.starts_with(&output),
        Fault::new(
            "NG102",
            "Choose separate watched-import and output folders; neither may contain the other"
        )
    );
    if let Some(pause) = pause_file {
        let pause = path_identity(pause)?;
        anyhow::ensure!(
            pause != state && pause != configuration,
            Fault::new(
                "NG102",
                "The pause marker must differ from configuration and state files"
            )
        );
    }
    Ok(())
}
fn load_state(path: &Path) -> Result<ServiceState> {
    if !path.exists() {
        return Ok(ServiceState::default());
    }
    watch::no_links(path)?;
    let value: serde_json::Value = read_json(path, 16_000_000)?;
    let object = value.as_object().ok_or_else(|| {
        Fault::new(
            "NG500",
            "Existing watch state is not a supported JSON object; it was preserved",
        )
    })?;
    if object.get("kind").and_then(|v| v.as_str()) == Some("notegen_watch_service") {
        anyhow::ensure!(
            object
                .keys()
                .all(|k| ["schema", "kind", "watch", "failures"].contains(&k.as_str())),
            Fault::new(
                "NG500",
                "Unknown watch service state fields; existing file preserved"
            )
        );
        let state: ServiceState = serde_json::from_value(value)?;
        anyhow::ensure!(
            state.schema == 1 && state.failures.len() <= 10000,
            Fault::new("NG500", "Unsupported or oversized watch service state")
        );
        return Ok(state);
    }
    anyhow::ensure!(
        object.len() == 4
            && object.keys().all(
                |k| ["schema", "folder_identity", "observations", "imported"].contains(&k.as_str())
            )
            && object.get("schema").and_then(|v| v.as_u64()) == Some(1),
        Fault::new(
            "NG500",
            "Existing JSON is not watch state; choose a new state path to preserve it"
        )
    );
    Ok(ServiceState {
        watch: serde_json::from_value(value)?,
        ..Default::default()
    })
}
/// First failure waits 30 seconds; subsequent failures cap at five minutes.
pub fn retry_delay(attempts: u32) -> u64 {
    30u64
        .saturating_mul(1u64 << attempts.saturating_sub(1).min(4))
        .min(300)
}
fn key(candidate: &ImportCandidate) -> Result<String> {
    fingerprint(&(&candidate.filename, &candidate.sha256))
}
fn output_path(output_dir: &Path, candidate: &ImportCandidate) -> Result<PathBuf> {
    let stem = Path::new(&candidate.filename)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .take(64)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let stem = if stem.is_empty() { "scan" } else { &stem };
    anyhow::ensure!(
        candidate.sha256.len() == 64 && candidate.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        Fault::new("NG500", "Invalid watched input fingerprint")
    );
    for suffix in 0..10000 {
        let name = if suffix == 0 {
            format!("{stem}-{}.docx", &candidate.sha256[..12])
        } else {
            format!("{stem}-{}-{suffix}.docx", &candidate.sha256[..12])
        };
        let path = output_dir.join(name);
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(Fault::new(
        "NG401",
        "All watched output names already exist; choose another output folder",
    )
    .into())
}
fn persist(path: &Path, state: &ServiceState, last_saved: &mut String) -> Result<()> {
    anyhow::ensure!(
        serde_json::to_vec(state)?.len() <= 16_000_000,
        Fault::new(
            "NG500",
            "Watch state reached its 16 MB limit; preserve it and choose a new state file before continuing"
        )
    );
    let next = fingerprint(state)?;
    if &next != last_saved {
        watch::no_links(path)?;
        atomic_json(path, state)?;
        *last_saved = next;
    }
    Ok(())
}
fn cancelled(c: &Control) -> bool {
    c.cancelled.load(std::sync::atomic::Ordering::Relaxed)
}

/// Blocking worker for explicit local imports. `once` polls once and converts
/// the files already stable in that poll. Failed/cancelled inputs remain pending.
#[allow(clippy::too_many_arguments)]
pub fn run(
    config_path: &Path,
    state_path: &Path,
    output_dir: &Path,
    settings: &Settings,
    c: &Control,
    log: &Activity,
    once: bool,
    pause_file: Option<&Path>,
) -> Result<WatchSummary> {
    let mut summary=WatchSummary{note:"No original scans are changed. Completed outputs and recognition checkpoints are retained; only successfully converted, unchanged scans are marked imported. Pause/config changes stop new imports between conversions. Retrying a failed input reuses compatible pipeline checkpoints. The summary retains at most 256 output paths.".into(),..Default::default()};
    if cancelled(c) {
        summary.cancelled = true;
        return Ok(summary);
    }
    c.check()?;
    let initial = read_config(config_path)?;
    validate_layout(config_path, state_path, output_dir, &initial, pause_file)?;
    settings.validate()?;
    let lease_path = state_path.with_extension("watch.lock");
    watch::no_links(&lease_path)?;
    if let Some(pause) = pause_file {
        anyhow::ensure!(
            path_identity(pause)? != path_identity(&lease_path)?,
            Fault::new(
                "NG102",
                "The pause marker must differ from the watcher lease file"
            )
        );
    }
    if let Some(parent) = lease_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let lease = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lease_path)?;
    lease.try_lock().map_err(|_| {
        Fault::new(
            "NG500",
            "Another watched-import worker is using this state file",
        )
    })?;
    let _lease = lease;
    let mut state = load_state(state_path)?;
    let mut last_saved = if state_path.exists() {
        fingerprint(&state)?
    } else {
        String::new()
    };
    fs::create_dir_all(output_dir)?;
    watch::no_links(output_dir)?;
    let output_dir = output_dir.canonicalize()?;
    let mut conversion_settings = settings.clone();
    conversion_settings.assistance.ask_questions = false;
    conversion_settings.assistance.web_lookup = false;
    let mut duplicate_names = BTreeSet::new();
    let mut rejected_reasons: BTreeMap<String, String> = BTreeMap::new();
    let mut was_paused = None;
    log.emit(
        "watch",
        "Watching stable local scans with explicit import rules",
        0,
        0,
        0,
        "",
    );
    loop {
        if cancelled(c) {
            summary.cancelled = true;
            break;
        }
        c.check()?;
        let mut config = read_config(config_path)?;
        validate_layout(config_path, state_path, &output_dir, &config, pause_file)?;
        config.paused |= pause_file.is_some_and(Path::exists);
        if was_paused != Some(config.paused) {
            log.emit(
                "watch",
                if config.paused {
                    "Watched imports paused; no new conversions will start"
                } else {
                    "Watched imports active"
                },
                0,
                0,
                0,
                "",
            );
            was_paused = Some(config.paused);
        }
        let now = chrono::Utc::now().timestamp();
        let report = watch::scan(&config, &mut state.watch, now, c)?;
        summary.waiting_stable = report.waiting_stable.len();
        summary.rejected_files = report.rejected.len();
        for rejected in refresh_rejections(&mut rejected_reasons, &report.rejected) {
            log.emit(
                "watch",
                format!(
                    "{} was not imported: {}",
                    rejected.filename, rejected.reason
                ),
                0,
                0,
                0,
                "NG100",
            );
        }
        observe_duplicates(
            &mut duplicate_names,
            report.skipped_duplicates,
            &mut summary,
        );
        state.failures.retain(|_, failure| {
            state
                .watch
                .observations
                .get(&failure.filename)
                .is_some_and(|o| o.sha256.as_deref() == Some(failure.sha256.as_str()))
        });
        persist(state_path, &state, &mut last_saved)?;
        for original in report.ready {
            if cancelled(c) {
                summary.cancelled = true;
                break;
            }
            c.check()?;
            let now = chrono::Utc::now().timestamp();
            let candidate_key = key(&original)?;
            if state
                .failures
                .get(&candidate_key)
                .is_some_and(|failure| now < failure.retry_after)
            {
                continue;
            }
            let mut current_config = read_config(config_path)?;
            validate_layout(
                config_path,
                state_path,
                &output_dir,
                &current_config,
                pause_file,
            )?;
            current_config.paused |= pause_file.is_some_and(Path::exists);
            if current_config.paused {
                break;
            }
            let fresh = watch::scan(&current_config, &mut state.watch, now, c)?;
            let Some(candidate) = fresh.ready.into_iter().find(|value| {
                value.filename == original.filename && value.sha256 == original.sha256
            }) else {
                continue;
            };
            let attempt = || -> Result<PathBuf> {
                anyhow::ensure!(
                    storage::hash_file(&candidate.path, c)? == candidate.sha256,
                    Fault::new("NG500", "Watched input changed before conversion")
                );
                let output = output_path(&output_dir, &candidate)?;
                let completed = crate::pipeline::convert(
                    &candidate.path,
                    &output,
                    &conversion_settings,
                    c,
                    log,
                )?;
                anyhow::ensure!(
                    storage::hash_file(&candidate.path, c)? == candidate.sha256,
                    Fault::new(
                        "NG500",
                        "Watched input changed during conversion; output retained for inspection, import remains pending"
                    )
                );
                watch::record_import(
                    &current_config,
                    &mut state.watch,
                    &candidate,
                    &format!("local-project-{}", fingerprint(&completed.project.root)?),
                    chrono::Utc::now().timestamp(),
                    c,
                )?;
                Ok(output)
            };
            let mut attempt = attempt;
            match attempt() {
                Ok(output) => {
                    state.failures.remove(&candidate_key);
                    summary.converted = summary.converted.saturating_add(1);
                    if summary.output_files.len() == 256 {
                        summary.output_files.remove(0);
                    }
                    summary.output_files.push(output);
                    log.emit(
                        "watch",
                        format!(
                            "Converted {} and recorded its import fingerprint",
                            candidate.filename
                        ),
                        0,
                        0,
                        0,
                        "",
                    );
                }
                Err(error) => {
                    if cancelled(c)
                        || error
                            .downcast_ref::<Fault>()
                            .is_some_and(|f| f.code == "NG900")
                    {
                        summary.cancelled = true;
                        break;
                    }
                    let attempts = state
                        .failures
                        .get(&candidate_key)
                        .map_or(1, |old| old.attempts.saturating_add(1));
                    let delay = retry_delay(attempts);
                    state.failures.insert(
                        candidate_key,
                        Failure {
                            filename: candidate.filename.clone(),
                            sha256: candidate.sha256.clone(),
                            attempts,
                            retry_after: chrono::Utc::now()
                                .timestamp()
                                .saturating_add(delay as i64),
                            error: error.to_string().chars().take(240).collect(),
                        },
                    );
                    summary.failed_attempts = summary.failed_attempts.saturating_add(1);
                    log.emit(
                        "watch",
                        format!(
                            "{} remains pending; retry after {delay}s: {error}",
                            candidate.filename
                        ),
                        0,
                        0,
                        0,
                        "NG500",
                    );
                }
            }
            persist(state_path, &state, &mut last_saved)?;
        }
        persist(state_path, &state, &mut last_saved)?;
        if once || summary.cancelled {
            break;
        }
        // Short, bounded polling keeps cancellation and the pause marker responsive.
        thread::sleep(Duration::from_millis(250));
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_summary_caps_names_without_counting_repeats_and_marks_overflow_once() {
        let mut names = BTreeSet::new();
        let mut summary = WatchSummary::default();
        observe_duplicates(
            &mut names,
            (0..MAX_SUMMARY_NAMES).map(|n| format!("scan-{n}.png")),
            &mut summary,
        );
        assert_eq!(summary.skipped_duplicate_files, 10_000);
        assert!(!summary.duplicate_summary_truncated);
        observe_duplicates(
            &mut names,
            ["scan-0.png".into(), "scan-9999.png".into()],
            &mut summary,
        );
        assert!(!summary.duplicate_summary_truncated);
        observe_duplicates(
            &mut names,
            (10_000..20_000).map(|n| format!("scan-{n}.png")),
            &mut summary,
        );
        assert_eq!(names.len(), MAX_SUMMARY_NAMES);
        assert_eq!(summary.skipped_duplicate_files, 10_000);
        assert!(summary.duplicate_summary_truncated);
        assert!(
            summary
                .note
                .contains("additional skipped duplicate filenames")
        );
        let note = summary.note.clone();
        observe_duplicates(&mut names, ["another-duplicate.png".into()], &mut summary);
        assert_eq!(summary.note, note);
    }
    #[test]
    fn rejection_deduplication_tracks_only_current_poll_and_reannounces_changed_errors() {
        let rejected = |filename: &str, reason: &str| watch::RejectedFile {
            filename: filename.into(),
            reason: reason.into(),
        };
        let mut saved = BTreeMap::new();
        let first = vec![rejected("a.png", "locked"), rejected("b.pdf", "empty")];
        assert_eq!(refresh_rejections(&mut saved, &first).len(), 2);
        assert!(refresh_rejections(&mut saved, &first).is_empty());
        let next = vec![rejected("b.pdf", "too large"), rejected("c.png", "locked")];
        assert_eq!(refresh_rejections(&mut saved, &next).len(), 2);
        assert!(!saved.contains_key("a.png"));
        assert_eq!(saved.len(), 2);
        assert_eq!(saved["b.pdf"], "too large");
        assert!(refresh_rejections(&mut saved, &[]).is_empty());
        assert!(saved.is_empty());
    }
    #[test]
    fn many_rejection_polls_stay_bounded_without_pruning_successful_import_receipts() {
        let mut state = ServiceState::default();
        state.watch.imported.insert(
            "completed.png".into(),
            watch::ImportedFile {
                filename: "completed.png".into(),
                sha256: "a".repeat(64),
                bytes: 12,
                modified_nanos: 1,
                project_id: "completed-project".into(),
                imported_at: 10,
            },
        );
        let before = fingerprint(&state.watch.imported).unwrap();
        let mut saved = BTreeMap::new();
        for poll in 0..3 {
            let current = (0..MAX_SUMMARY_NAMES + 5)
                .map(|n| watch::RejectedFile {
                    filename: format!("poll-{poll}-file-{n}.pdf"),
                    reason: "locked".into(),
                })
                .collect::<Vec<_>>();
            refresh_rejections(&mut saved, &current);
            assert_eq!(saved.len(), MAX_SUMMARY_NAMES);
            assert!(
                saved
                    .keys()
                    .all(|name| name.starts_with(&format!("poll-{poll}-")))
            );
        }
        assert_eq!(fingerprint(&state.watch.imported).unwrap(), before);
        assert!(state.watch.imported.contains_key("completed.png"));
    }
    #[test]
    fn retry_backoff_is_exponential_bounded_and_handles_overflow() {
        assert_eq!(
            (1..=6).map(retry_delay).collect::<Vec<_>>(),
            vec![30, 60, 120, 240, 300, 300]
        );
        assert_eq!(retry_delay(u32::MAX), 300);
    }
    #[test]
    fn output_planning_never_overwrites_existing_files_or_uses_untrusted_stem_punctuation() {
        let dir = tempfile::tempdir().unwrap();
        let candidate = ImportCandidate {
            path: dir.path().join("scan.pdf"),
            filename: "../../CON:notes.pdf".into(),
            sha256: "a".repeat(64),
            bytes: 1,
            modified_nanos: 0,
            duplicate_of: None,
            observation_token: String::new(),
        };
        let first = output_path(dir.path(), &candidate).unwrap();
        assert_eq!(first.parent(), Some(dir.path()));
        assert!(!first.file_name().unwrap().to_string_lossy().contains(':'));
        fs::write(&first, b"keep").unwrap();
        let second = output_path(dir.path(), &candidate).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(first).unwrap(), b"keep");
    }
    #[test]
    fn layouts_reject_nested_output_config_state_aliases_and_scan_state_suffixes() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("imports");
        fs::create_dir(&source).unwrap();
        let config = WatchConfig {
            folder: source.clone(),
            ..Default::default()
        };
        let config_path = dir.path().join("watch.json");
        fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        assert!(
            validate_layout(
                &config_path,
                &dir.path().join("state.json"),
                &source.join("outputs"),
                &config,
                None
            )
            .is_err()
        );
        assert!(
            validate_layout(
                &config_path,
                &config_path,
                &dir.path().join("outputs"),
                &config,
                None
            )
            .is_err()
        );
        assert!(
            validate_layout(
                &config_path,
                &source.join("scan.png"),
                &dir.path().join("outputs"),
                &config,
                None
            )
            .is_err()
        );
    }
    #[test]
    fn unknown_existing_json_state_and_unknown_config_fields_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("private.json");
        fs::write(&state, b"{\"private_document\":true}").unwrap();
        let before = fs::read(&state).unwrap();
        assert!(load_state(&state).is_err());
        assert_eq!(fs::read(&state).unwrap(), before);
        let config = dir.path().join("config.json");
        fs::write(&config, b"{\"folder\":\"imports\",\"unexpected\":true}").unwrap();
        assert!(read_config(&config).is_err());
    }
    #[test]
    fn empty_once_worker_persists_observation_state_and_honors_explicit_pause_without_native_inference()
     {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("imports");
        fs::create_dir(&source).unwrap();
        let config = WatchConfig {
            folder: source,
            paused: true,
            ..Default::default()
        };
        let config_path = dir.path().join("watch.json");
        atomic_json(&config_path, &config).unwrap();
        let state_path = dir.path().join("state.json");
        let result = run(
            &config_path,
            &state_path,
            &dir.path().join("outputs"),
            &Settings::default(),
            &Control::default(),
            &Activity::default(),
            true,
            None,
        )
        .unwrap();
        assert_eq!(result.converted, 0);
        let state = load_state(&state_path).unwrap();
        assert_eq!(state.watch.schema, 1);
        assert!(state.watch.imported.is_empty());
    }
}
