//! Explicit retention-managed sessions. This is neither encryption nor secure erasure.
//! Native OCR/process/OS temporary files, caches, pagefile and crash dumps are unmanaged.
use crate::{
    pipeline,
    storage::{Activity, Control, atomic_json, fingerprint, hash_file, read_json},
    types::*,
    watch::no_links,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::Ordering,
    time::UNIX_EPOCH,
};
use zip::{ZipWriter, write::SimpleFileOptions};

const MARKER: &str = "notegen-session.json";
const STATE: &str = "retention.json";
const LIMIT: usize = 50_000;
pub const RETENTION_LIMITS: &str = "Keep is the default. Only explicitly selected unchanged tracked regular files can be discarded after a verified external export. Source originals are untouched. Marker, retention state, locks, unknown and modified files are preserved. Failed/interrupted conversion work remains untracked and requires manual preservation; it is never inferred as owned from filenames. Native OCR/process temporary files, OS caches, pagefile and crash dumps are unmanaged. This is not encryption, secure erasure, or complete process isolation.";

#[derive(Clone, Debug)]
pub struct Session {
    pub root: PathBuf,
    id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    schema: u32,
    kind: String,
    root: PathBuf,
    id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OwnedFile {
    /// Validated relative path under this marked session only.
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub modified_nanos: u128,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExportReceipt {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub kind: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    directory: String,
    files: Vec<OwnedFile>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema: u32,
    id: String,
    phase: String,
    source: Option<PathBuf>,
    output: Option<PathBuf>,
    owned: Vec<OwnedFile>,
    exports: Vec<ExportReceipt>,
    cleanup: Option<Journal>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preserved {
    pub path: PathBuf,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetentionPreview {
    pub root: PathBuf,
    pub phase: String,
    pub files: Vec<OwnedFile>,
    pub preserved: Vec<Preserved>,
    pub reclaimable_bytes: u64,
    pub export: Option<ExportReceipt>,
    pub export_verified: bool,
    pub token: String,
    pub note: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CleanupReport {
    pub removed: Vec<PathBuf>,
    pub removed_bytes: u64,
    pub preserved: Vec<Preserved>,
    pub cancelled: bool,
    pub recovery_required: bool,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunReport {
    pub success: bool,
    pub cancelled: bool,
    pub project: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub error: Option<String>,
    /// Cancelled runs defer expensive hashing until a later explicit inspect().
    pub retention: Option<RetentionPreview>,
}
fn fail(message: &str) -> anyhow::Error {
    Fault::new("NG680", message).into()
}
fn digest_valid(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().count() <= 12
        && path.components().all(|c| match c {
            Component::Normal(value) => value.to_str().is_some_and(|s| {
                !s.is_empty()
                    && s.len() <= 240
                    && !s.contains([':', '\\', '/'])
                    && !s.chars().any(char::is_control)
            }),
            _ => false,
        })
}
fn safe(session: &Session, relative_path: &Path) -> Result<PathBuf> {
    anyhow::ensure!(
        relative(relative_path),
        fail("Unsafe session-relative path")
    );
    let path = session.root.join(relative_path);
    no_links(&path)?;
    if path.exists() {
        anyhow::ensure!(
            path.canonicalize()?.starts_with(&session.root),
            fail("Session artifact escaped its root")
        );
    }
    Ok(path)
}
fn modified(metadata: &fs::Metadata) -> Result<u128> {
    Ok(metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos())
}
fn snapshot(session: &Session, path: &Path, c: &Control) -> Result<OwnedFile> {
    let full = safe(session, path)?;
    let before = fs::symlink_metadata(&full)?;
    anyhow::ensure!(
        before.is_file(),
        fail("Session artifact is not a regular file")
    );
    let sha256 = hash_file(&full, c)?;
    let after = fs::symlink_metadata(&full)?;
    anyhow::ensure!(
        before.len() == after.len() && modified(&before)? == modified(&after)?,
        fail("Session artifact changed while inspecting it")
    );
    no_links(&full)?;
    Ok(OwnedFile {
        path: path.into(),
        bytes: after.len(),
        sha256,
        modified_nanos: modified(&after)?,
    })
}
fn valid_state(session: &Session, state: &State) -> Result<()> {
    anyhow::ensure!(
        state.schema == 1
            && state.id == session.id
            && [
                "created",
                "prepared",
                "running",
                "converted",
                "failed",
                "interrupted",
                "discarded"
            ]
            .contains(&state.phase.as_str())
            && state.owned.len() <= LIMIT
            && state.exports.len() <= 100,
        fail("Invalid session retention manifest")
    );
    let mut paths = BTreeSet::new();
    for file in &state.owned {
        anyhow::ensure!(
            relative(&file.path)
                && (file.path.starts_with("source") || file.path.starts_with("work"))
                && digest_valid(&file.sha256)
                && paths.insert(&file.path),
            fail("Invalid or duplicate owned session artifact")
        );
    }
    for path in [&state.source, &state.output].into_iter().flatten() {
        anyhow::ensure!(relative(path), fail("Invalid session source/output path"));
    }
    for receipt in &state.exports {
        anyhow::ensure!(
            receipt.path.is_absolute()
                && !receipt.path.starts_with(&session.root)
                && digest_valid(&receipt.sha256)
                && ["docx", "recovery_zip"].contains(&receipt.kind.as_str()),
            fail("Invalid external export receipt")
        );
    }
    if let Some(journal) = &state.cleanup {
        anyhow::ensure!(
            journal.directory.starts_with("discard-")
                && relative(Path::new(&journal.directory))
                && Path::new(&journal.directory).components().count() == 1
                && journal.files.len() <= LIMIT
                && !journal.files.is_empty()
                && journal.files.iter().all(|f| state.owned.contains(f)),
            fail("Invalid session cleanup journal")
        );
        anyhow::ensure!(
            journal
                .files
                .iter()
                .map(|f| &f.path)
                .collect::<BTreeSet<_>>()
                .len()
                == journal.files.len(),
            fail("Duplicate staged cleanup artifact")
        );
    }
    Ok(())
}
fn load(session: &Session) -> Result<State> {
    let checked = Session::read(&session.root)?;
    anyhow::ensure!(checked.id == session.id, fail("Session identity changed"));
    let state: State = read_json(&session.root.join(STATE), 32_000_000)?;
    valid_state(session, &state)?;
    Ok(state)
}
fn save(session: &Session, state: &State) -> Result<()> {
    valid_state(session, state)?;
    no_links(&session.root.join(STATE))?;
    atomic_json(&session.root.join(STATE), state)
}
fn lease(session: &Session) -> Result<fs::File> {
    load(session)?;
    let path = session.root.join("session.lock");
    no_links(&path)?;
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    file.try_lock()
        .map_err(|_| fail("Session is busy in another operation"))?;
    Ok(file)
}
impl Session {
    /// Creates a dedicated new folder. No existing folder is adopted for ownership.
    pub fn create(parent: &Path, c: &Control) -> Result<Self> {
        c.check()?;
        no_links(parent)?;
        let parent = parent.canonicalize()?;
        anyhow::ensure!(
            parent.is_dir(),
            fail("Choose an existing session parent folder")
        );
        let temporary = tempfile::Builder::new()
            .prefix("notegen-session-")
            .tempdir_in(&parent)?;
        let root = temporary.path().canonicalize()?;
        let id = fingerprint(&(root.clone(), chrono::Utc::now().timestamp_nanos_opt()))?;
        let session = Self {
            root: root.clone(),
            id: id.clone(),
        };
        atomic_json(
            &root.join(MARKER),
            &Marker {
                schema: 1,
                kind: "notegen_retention_session".into(),
                root,
                id: id.clone(),
            },
        )?;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(session.root.join("session.lock"))?;
        save(
            &session,
            &State {
                schema: 1,
                id,
                phase: "created".into(),
                source: None,
                output: None,
                owned: vec![],
                exports: vec![],
                cleanup: None,
            },
        )?;
        let _ = temporary.keep();
        Ok(session)
    }
    /// Reopens only an app-marked folder at its original canonical location.
    pub fn read(root: &Path) -> Result<Self> {
        no_links(root)?;
        let root = root.canonicalize()?;
        anyhow::ensure!(
            root.is_dir()
                && root
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.starts_with("notegen-session-")),
            fail("Not an app-created retention session folder")
        );
        for name in [MARKER, STATE, "session.lock"] {
            no_links(&root.join(name))?;
            anyhow::ensure!(
                fs::symlink_metadata(root.join(name))?.is_file(),
                fail("Session marker/state/lock must be regular files")
            );
        }
        let marker: Marker = read_json(&root.join(MARKER), 16_000)?;
        anyhow::ensure!(
            marker.schema == 1
                && marker.kind == "notegen_retention_session"
                && marker.root == root
                && digest_valid(&marker.id),
            fail("Session marker identity/location is invalid")
        );
        Ok(Self {
            root,
            id: marker.id,
        })
    }
}
fn copy_protected(source: &Path, target: &Path, c: &Control) -> Result<()> {
    no_links(source)?;
    no_links(target)?;
    anyhow::ensure!(!target.exists(), fail("Session/export output collision"));
    let parent = target
        .parent()
        .ok_or_else(|| fail("Output needs a parent folder"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    let before = hash_file(source, c)?;
    let mut input = fs::File::open(source)?;
    let mut buffer = [0u8; 65536];
    let mut hash = Sha256::new();
    loop {
        c.check()?;
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        temp.write_all(&buffer[..n])?;
        hash.update(&buffer[..n]);
    }
    anyhow::ensure!(
        hex::encode(hash.finalize()) == before && hash_file(source, c)? == before,
        fail("Source changed while copying; original was not modified")
    );
    temp.flush()?;
    temp.as_file().sync_all()?;
    c.check()?;
    no_links(target)?;
    temp.persist_noclobber(target)
        .map_err(|_| fail("Output collision; existing file preserved"))?;
    Ok(())
}
fn prepare_locked(
    session: &Session,
    source: &Path,
    state: &mut State,
    c: &Control,
) -> Result<PathBuf> {
    anyhow::ensure!(
        state.phase == "created" && state.cleanup.is_none(),
        fail("This session already has an input; create a new session for another conversion")
    );
    no_links(source)?;
    let source = source.canonicalize()?;
    anyhow::ensure!(
        !source.starts_with(&session.root),
        fail("Choose an original outside this session")
    );
    let files = crate::inputs::files(&source)?;
    let original_digest = crate::inputs::hash(&source, c)?;
    let mut bytes = 0u64;
    for file in &files {
        no_links(file)?;
        let size = fs::metadata(file)?.len();
        bytes = bytes
            .checked_add(size)
            .ok_or_else(|| fail("Input size overflow"))?;
        anyhow::ensure!(
            size <= 2_000_000_000 && bytes <= 20_000_000_000,
            fail("Session source copy exceeds the 2 GB/file or 20 GB total bound")
        );
    }
    let directory = safe(session, Path::new("source"))?;
    fs::create_dir(&directory)?;
    // File names are normalized while preserving the source folder's page order.
    for (i, file) in files.iter().enumerate() {
        c.check()?;
        let extension = file
            .extension()
            .and_then(|s| s.to_str())
            .ok_or_else(|| fail("Unsupported source extension"))?
            .to_ascii_lowercase();
        let path = PathBuf::from("source").join(format!("page-{:05}.{extension}", i + 1));
        copy_protected(file, &safe(session, &path)?, c)?;
        state.owned.push(snapshot(session, &path, c)?);
        save(session, state)?;
    }
    anyhow::ensure!(
        crate::inputs::hash(&source, c)? == original_digest,
        fail("Source selection changed during copying; retained copies require inspection")
    );
    let copied = if source.is_dir() {
        PathBuf::from("source")
    } else {
        state.owned[0].path.clone()
    };
    state.source = Some(copied.clone());
    state.phase = "prepared".into();
    save(session, state)?;
    Ok(session.root.join(copied))
}
/// Copy a bounded input into the marked session, without writing the user's original.
pub fn copy_source(session: &Session, source: &Path, c: &Control) -> Result<PathBuf> {
    let _lease = lease(session)?;
    let mut state = load(session)?;
    let result = prepare_locked(session, source, &mut state, c);
    if result.is_err() {
        state.phase = "failed".into();
        save(session, &state)?;
    }
    result
}
fn walk(session: &Session) -> Result<Vec<PathBuf>> {
    let mut pending = vec![(session.root.clone(), 0usize)];
    let mut files = vec![];
    let mut count = 0usize;
    while let Some((directory, depth)) = pending.pop() {
        anyhow::ensure!(
            depth <= 12,
            fail("Session folder nesting exceeds its inspection bound")
        );
        no_links(&directory)?;
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            count += 1;
            anyhow::ensure!(
                count <= LIMIT,
                fail("Session inventory exceeds 50,000 entries; nothing was removed")
            );
            no_links(&entry.path())?;
            if entry.file_type()?.is_dir() {
                pending.push((entry.path(), depth + 1));
            } else if entry.file_type()?.is_file() {
                files.push(entry.path().strip_prefix(&session.root)?.to_owned());
            } else {
                return Err(fail("Session inventory contains a non-regular artifact"));
            }
        }
    }
    files.sort();
    Ok(files)
}
fn produced(path: &Path, actual_project: &Path) -> bool {
    if path == Path::new("work/result.docx") {
        return true;
    }
    let Ok(local) = path.strip_prefix(actual_project) else {
        return false;
    };
    let components: Vec<_> = local.iter().filter_map(|s| s.to_str()).collect();
    if components.len() == 2 && components[0] == "responses" {
        return components[1].ends_with(".txt");
    }
    if components.len() != 1 {
        return false;
    }
    let name = components[0];
    [
        "project.json",
        "project-metadata.json",
        "activity.jsonl",
        "hardware.json",
        "plan.json",
        "preparation-report.json",
        "input-invalidated.json",
    ]
    .contains(&name)
        || name.starts_with("server-") && name.ends_with(".log")
        || name.strip_prefix("page-").is_some_and(|tail| {
            tail.as_bytes()
                .get(..5)
                .is_some_and(|digits| digits.iter().all(u8::is_ascii_digit))
                && tail
                    .get(5..)
                    .is_some_and(|suffix| [".png", ".json", ".edits.json"].contains(&suffix))
        })
}
fn seal_completed(session: &Session, state: &mut State, project: &Path, c: &Control) -> Result<()> {
    no_links(project)?;
    let project = project.canonicalize()?;
    let expected_parent = safe(session, Path::new("work/.notegen"))?.canonicalize()?;
    anyhow::ensure!(
        project.is_dir() && project.parent() == Some(expected_parent.as_path()),
        fail("Completed project is not inside this session's exact work folder")
    );
    let actual = project.strip_prefix(&session.root)?;
    for path in walk(session)? {
        if produced(&path, actual) && !state.owned.iter().any(|f| f.path == path) {
            state.owned.push(snapshot(session, &path, c)?);
        }
    }
    Ok(())
}
/// Runs sequential conversion with only app-owned local reader processes. Keep remains the default.
/// Activity callbacks/clarifiers are intentionally not forwarded; cancellation/pause remain shared.
pub fn run(
    session: &Session,
    source: &Path,
    settings: &Settings,
    c: &Control,
    _activity: &Activity,
) -> Result<RunReport> {
    let _lease = lease(session)?;
    let mut state = load(session)?;
    anyhow::ensure!(
        !settings.external_server,
        fail("Retention sessions require internally launched local readers")
    );
    let (models, errors) = crate::runtime::inventory_available(&settings.models_dir)?;
    anyhow::ensure!(
        errors.is_empty() && models.iter().all(|m| m.endpoint.is_none()),
        fail("Retention sessions reject endpoint readers and invalid registry entries")
    );
    let mut settings = settings.clone();
    settings.assistance.ask_questions = false;
    settings.assistance.max_questions = 0;
    settings.assistance.web_lookup = false;
    settings.assistance.sources.clear();
    settings.experience.check_updates = false;
    settings.experience.live_preview = false;
    settings.writer.use_examples = false;
    settings.writer.id = None;
    settings.writer_reference = None;
    settings.validate()?;
    let copied = prepare_locked(session, source, &mut state, c);
    let copied = match copied {
        Ok(path) => path,
        Err(error) => {
            state.phase = "failed".into();
            save(session, &state)?;
            return Err(error);
        }
    };
    let work = safe(session, Path::new("work"))?;
    fs::create_dir(&work)?;
    state.phase = "running".into();
    save(session, &state)?;
    let control = Control {
        cancelled: c.cancelled.clone(),
        paused: c.paused.clone(),
        clarify: None,
    };
    let output = work.join("result.docx");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pipeline::convert(&copied, &output, &settings, &control, &Activity::default())
    }))
    .unwrap_or_else(|_| {
        Err(fail(
            "Conversion stopped unexpectedly; session artifacts were retained",
        ))
    });
    let cancelled = c.cancelled.load(Ordering::Relaxed);
    let (success, project, error) = match result {
        Ok(done) => {
            if !cancelled {
                // Only the exact successfully returned project can establish ownership.
                // Sibling folders and all failed/interrupted work remain unknown.
                seal_completed(session, &mut state, &done.project.root, c)?;
            }
            state.output = Some(PathBuf::from("work/result.docx"));
            (true, Some(done.project.root), None)
        }
        Err(error) => (
            false,
            None,
            Some(error.to_string().chars().take(2000).collect()),
        ),
    };
    state.phase = if success { "converted" } else { "failed" }.into();
    save(session, &state)?;
    Ok(RunReport {
        success,
        cancelled,
        project,
        output: state.output.as_ref().map(|p| session.root.join(p)),
        error,
        retention: if cancelled {
            None
        } else {
            Some(inspect_locked(session, &state, c)?)
        },
    })
}
fn output_path(session: &Session, target: &Path, extension: &str) -> Result<PathBuf> {
    no_links(target)?;
    anyhow::ensure!(
        !target.exists()
            && target
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(extension)),
        fail("Choose a new export file with the required extension")
    );
    let absolute = std::path::absolute(target)?;
    let parent = absolute
        .parent()
        .ok_or_else(|| fail("Choose an export parent folder"))?
        .canonicalize()?;
    anyhow::ensure!(
        !parent.starts_with(&session.root),
        fail("Verified export must be outside the session")
    );
    Ok(parent.join(
        absolute
            .file_name()
            .ok_or_else(|| fail("Choose an export filename"))?,
    ))
}
fn receipt(
    session: &Session,
    state: &mut State,
    path: PathBuf,
    kind: &str,
    expected_sha256: &str,
    expected_bytes: u64,
    c: &Control,
) -> Result<ExportReceipt> {
    no_links(&path)?;
    let value = ExportReceipt {
        bytes: fs::metadata(&path)?.len(),
        sha256: hash_file(&path, c)?,
        path,
        kind: kind.into(),
    };
    anyhow::ensure!(
        value.sha256 == expected_sha256 && value.bytes == expected_bytes,
        fail("Export changed before verification; retained artifacts were kept")
    );
    anyhow::ensure!(
        state.exports.len() < 100,
        fail("Session already has 100 export receipts; keep the existing exports")
    );
    state.exports.push(value.clone());
    save(session, state)?;
    Ok(value)
}
/// Verify and copy the completed Word output outside the session without overwriting any file.
pub fn export(session: &Session, target: &Path, c: &Control) -> Result<ExportReceipt> {
    let _lease = lease(session)?;
    let mut state = load(session)?;
    anyhow::ensure!(
        state.phase == "converted" && state.cleanup.is_none(),
        fail("No completed session output is available")
    );
    anyhow::ensure!(
        state.exports.len() < 100,
        fail("Session already has 100 export receipts")
    );
    let output = state
        .output
        .as_ref()
        .ok_or_else(|| fail("Session has no output"))?;
    let owned = state
        .owned
        .iter()
        .find(|f| &f.path == output)
        .ok_or_else(|| fail("Output is not a tracked session artifact"))?
        .clone();
    anyhow::ensure!(
        owned == snapshot(session, output, c)?,
        fail("Session output changed; preserve and inspect it manually")
    );
    let target = output_path(session, target, "docx")?;
    copy_protected(&safe(session, output)?, &target, c)?;
    receipt(
        session,
        &mut state,
        target,
        "docx",
        &owned.sha256,
        owned.bytes,
        c,
    )
}
/// Explicit recovery export includes tracked scans/evidence only; unknown files stay in the session.
pub fn export_recovery(session: &Session, target: &Path, c: &Control) -> Result<ExportReceipt> {
    let _lease = lease(session)?;
    let mut state = load(session)?;
    anyhow::ensure!(
        state.exports.len() < 100,
        fail("Session already has 100 export receipts")
    );
    anyhow::ensure!(
        state.phase != "running" && state.cleanup.is_none() && !state.owned.is_empty(),
        fail("Recover interrupted operations before exporting retained artifacts")
    );
    let target = output_path(session, target, "zip")?;
    let mut temporary = tempfile::NamedTempFile::new_in(target.parent().unwrap())?;
    {
        let mut zip = ZipWriter::new(temporary.as_file_mut());
        for file in &state.owned {
            c.check()?;
            anyhow::ensure!(
                *file == snapshot(session, &file.path, c)?,
                fail(
                    "A retained artifact changed; unknown/modified files require manual preservation"
                )
            );
            zip.start_file(
                file.path.to_string_lossy().replace('\\', "/"),
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )?;
            let mut input = fs::File::open(safe(session, &file.path)?)?;
            let mut buffer = [0u8; 65536];
            let mut hash = Sha256::new();
            loop {
                c.check()?;
                let n = input.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                zip.write_all(&buffer[..n])?;
                hash.update(&buffer[..n]);
            }
            anyhow::ensure!(
                hex::encode(hash.finalize()) == file.sha256,
                fail("Artifact changed during recovery export")
            );
        }
        zip.finish()?;
    }
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    let expected_sha256 = hash_file(temporary.path(), c)?;
    let expected_bytes = temporary.as_file().metadata()?.len();
    c.check()?;
    no_links(&target)?;
    temporary
        .persist_noclobber(&target)
        .map_err(|_| fail("Recovery export collision; existing file preserved"))?;
    receipt(
        session,
        &mut state,
        target,
        "recovery_zip",
        &expected_sha256,
        expected_bytes,
        c,
    )
}
fn verified_export(session: &Session, value: &ExportReceipt, c: &Control) -> bool {
    !value.path.starts_with(&session.root)
        && no_links(&value.path).is_ok()
        && fs::metadata(&value.path).is_ok_and(|m| m.is_file() && m.len() == value.bytes)
        && hash_file(&value.path, c).is_ok_and(|hash| hash == value.sha256)
}
fn inspect_locked(session: &Session, state: &State, c: &Control) -> Result<RetentionPreview> {
    c.check()?;
    anyhow::ensure!(
        state.cleanup.is_none(),
        fail("Interrupted staged cleanup requires recover() before a new preview")
    );
    let mut files = vec![];
    let mut preserved = vec![];
    let mut observed = BTreeSet::new();
    let owned: BTreeMap<_, _> = state.owned.iter().map(|f| (&f.path, f)).collect();
    for path in walk(session)? {
        c.check()?;
        observed.insert(path.clone());
        if let Some(expected) = owned.get(&path) {
            let current = snapshot(session, &path, c)?;
            if **expected == current {
                files.push(current);
            } else {
                preserved.push(Preserved {
                    path,
                    reason: "Modified after recording; preserved".into(),
                });
            }
        } else {
            preserved.push(Preserved {
                path,
                reason: "Marker/state/lock or untracked file; preserved".into(),
            });
        }
    }
    for file in &state.owned {
        if !observed.contains(&file.path) {
            preserved.push(Preserved {
                path: file.path.clone(),
                reason: "Tracked artifact is missing; no deletion attempted".into(),
            });
        }
    }
    let export = state.exports.last().cloned();
    let export_verified = export
        .as_ref()
        .is_some_and(|value| verified_export(session, value, c));
    let token = fingerprint(&(
        session.id.clone(),
        fingerprint(state)?,
        &files,
        &preserved,
        &export,
        export_verified,
    ))?;
    Ok(RetentionPreview {
        root: session.root.clone(),
        phase: state.phase.clone(),
        reclaimable_bytes: files.iter().map(|f| f.bytes).sum(),
        files,
        preserved,
        export,
        export_verified,
        token,
        note: RETENTION_LIMITS.into(),
    })
}
pub fn inspect(session: &Session, c: &Control) -> Result<RetentionPreview> {
    let _lease = lease(session)?;
    inspect_locked(session, &load(session)?, c)
}
fn restore_staged(session: &Session, state: &mut State, c: &Control) -> Result<Vec<Preserved>> {
    let Some(journal) = state.cleanup.clone() else {
        return Ok(vec![]);
    };
    let directory = safe(session, Path::new(&journal.directory))?;
    let mut unresolved = vec![];
    let mut already_removed = BTreeSet::new();
    for (i, original) in journal.files.iter().enumerate() {
        c.check()?;
        let staged = directory.join(format!("{i:05}.retained"));
        no_links(&staged)?;
        let target = safe(session, &original.path)?;
        if !staged.exists() {
            if !target.exists() {
                already_removed.insert(original.path.clone());
            }
            continue;
        }
        if !target.exists() && fs::hard_link(&staged, &target).is_ok() {
            fs::remove_file(&staged)?;
        } else {
            unresolved.push(Preserved {
                path: staged.strip_prefix(&session.root)?.into(),
                reason: "Recovery destination collision; retained in staged folder".into(),
            });
        }
    }
    if unresolved.is_empty() {
        if directory.exists() {
            let _ = fs::remove_dir(&directory);
        }
        state.cleanup = None;
        state
            .owned
            .retain(|file| !already_removed.contains(&file.path));
        save(session, state)?;
    }
    Ok(unresolved)
}
/// Recover an interrupted run/cleanup without adopting unknown files or overwriting collisions.
pub fn recover(session: &Session, c: &Control) -> Result<Vec<Preserved>> {
    let _lease = lease(session)?;
    let mut state = load(session)?;
    let preserved = restore_staged(session, &mut state, c)?;
    if state.phase == "running" {
        state.phase = "interrupted".into();
        save(session, &state)?;
    }
    Ok(preserved)
}
/// Explicitly discard only selected unchanged artifacts from this exact fresh preview.
/// Files are first staged with a persistent recovery journal; folders are never recursively removed.
pub fn discard(
    session: &Session,
    inspected: &RetentionPreview,
    selected: &[PathBuf],
    c: &Control,
) -> Result<CleanupReport> {
    let _lease = lease(session)?;
    let mut state = load(session)?;
    let fresh = inspect_locked(session, &state, c)?;
    anyhow::ensure!(
        fresh.token == inspected.token && fingerprint(&fresh)? == fingerprint(inspected)?,
        fail("Retention preview was changed or is stale; inspect again")
    );
    anyhow::ensure!(
        fresh.export_verified
            && !selected.is_empty()
            && selected.len() <= LIMIT
            && selected.iter().collect::<BTreeSet<_>>().len() == selected.len(),
        fail("Select distinct tracked artifacts after a verified external export")
    );
    let by_path: BTreeMap<_, _> = fresh.files.iter().map(|f| (&f.path, f)).collect();
    let files = selected
        .iter()
        .map(|p| {
            by_path
                .get(p)
                .copied()
                .cloned()
                .ok_or_else(|| fail("Selected artifact is not an unchanged tracked file"))
        })
        .collect::<Result<Vec<_>>>()?;
    let directory = format!(
        "discard-{}",
        &fingerprint(&(session.id.clone(), &fresh.token, &files))?[..24]
    );
    fs::create_dir(safe(session, Path::new(&directory))?)?;
    state.cleanup = Some(Journal {
        directory: directory.clone(),
        files: files.clone(),
    });
    save(session, &state)?;
    let staged_result = (|| -> Result<()> {
        for (i, file) in files.iter().enumerate() {
            c.check()?;
            anyhow::ensure!(
                *file == snapshot(session, &file.path, c)?,
                fail("Selected session artifact changed during staging")
            );
            let staged = safe(
                session,
                &PathBuf::from(&directory).join(format!("{i:05}.retained")),
            )?;
            anyhow::ensure!(!staged.exists(), fail("Staged cleanup collision"));
            fs::rename(safe(session, &file.path)?, &staged)?;
            no_links(&staged)?;
            anyhow::ensure!(
                hash_file(&staged, c)? == file.sha256,
                fail("Staged artifact changed; retained for recovery")
            );
        }
        c.check()?;
        anyhow::ensure!(
            fresh
                .export
                .as_ref()
                .is_some_and(|r| verified_export(session, r, c)),
            fail("Verified export disappeared or changed before discard")
        );
        Ok(())
    })();
    let mut report = CleanupReport {
        preserved: fresh.preserved,
        note: RETENTION_LIMITS.into(),
        ..Default::default()
    };
    report.preserved.extend(
        fresh
            .files
            .iter()
            .filter(|f| !selected.contains(&f.path))
            .map(|f| Preserved {
                path: f.path.clone(),
                reason: "Kept; not selected for discard".into(),
            }),
    );
    if staged_result.is_err() {
        report.cancelled = c.cancelled.load(Ordering::Relaxed);
        report
            .preserved
            .extend(restore_staged(session, &mut state, &Control::default())?);
        report.recovery_required = state.cleanup.is_some();
        report.preserved.extend(files.iter().map(|file| Preserved {
            path: file.path.clone(),
            reason: "Discard stopped during staging; data retained or restored".into(),
        }));
        report.note.push_str(" Staging stopped; surviving artifacts were restored or retained in the cleanup journal. Inspect/recover before retrying.");
        return Ok(report);
    }
    // All selected data and the outside export were checked before the first irreversible removal.
    for (i, file) in files.iter().enumerate() {
        let staged = safe(
            session,
            &PathBuf::from(&directory).join(format!("{i:05}.retained")),
        )?;
        if c.cancelled.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        if !hash_file(&staged, c).is_ok_and(|hash| hash == file.sha256) {
            break;
        }
        if fs::remove_file(&staged).is_err() {
            break;
        }
        report.removed.push(file.path.clone());
        report.removed_bytes += file.bytes;
    }
    save(session, &state)?;
    report
        .preserved
        .extend(restore_staged(session, &mut state, &Control::default())?);
    report.recovery_required = state.cleanup.is_some();
    report.preserved.extend(
        files
            .iter()
            .filter(|file| !report.removed.contains(&file.path))
            .map(|file| Preserved {
                path: file.path.clone(),
                reason: "Not removed; retained or restored after partial cleanup".into(),
            }),
    );
    if state.owned.is_empty() {
        state.phase = "discarded".into();
        state.source = None;
        state.output = None;
        save(session, &state)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retained(temp: &tempfile::TempDir) -> Session {
        let session = Session::create(temp.path(), &Control::default()).unwrap();
        fs::create_dir(session.root.join("work")).unwrap();
        fs::write(
            session.root.join("work/result.docx"),
            b"Disposable completed-output fixture",
        )
        .unwrap();
        fs::write(
            session.root.join("work/evidence.txt"),
            b"Disposable tracked evidence",
        )
        .unwrap();
        let mut state = load(&session).unwrap();
        state.phase = "converted".into();
        state.output = Some("work/result.docx".into());
        state.owned = ["work/result.docx", "work/evidence.txt"]
            .iter()
            .map(|path| snapshot(&session, Path::new(path), &Control::default()).unwrap())
            .collect();
        save(&session, &state).unwrap();
        session
    }
    #[test]
    fn fresh_selected_discard_preserves_unknown_modified_and_metadata_files() {
        let temp = tempfile::tempdir().unwrap();
        let session = retained(&temp);
        let output = temp.path().join("confirmed.docx");
        export(&session, &output, &Control::default()).unwrap();
        fs::write(
            session.root.join("work/user-notes.txt"),
            b"Untracked user data",
        )
        .unwrap();
        fs::write(
            session.root.join("work/evidence.txt"),
            b"User modified the evidence",
        )
        .unwrap();
        let preview = inspect(&session, &Control::default()).unwrap();
        assert!(preview.export_verified);
        assert_eq!(preview.files.len(), 1);
        assert!(
            preview
                .preserved
                .iter()
                .any(|p| p.path == Path::new("work/evidence.txt") && p.reason.contains("Modified"))
        );
        let result = discard(
            &session,
            &preview,
            &[PathBuf::from("work/result.docx")],
            &Control::default(),
        )
        .unwrap();
        assert_eq!(result.removed, [PathBuf::from("work/result.docx")]);
        assert!(!result.recovery_required);
        assert_eq!(
            fs::read(&output).unwrap(),
            b"Disposable completed-output fixture"
        );
        assert_eq!(
            fs::read(session.root.join("work/user-notes.txt")).unwrap(),
            b"Untracked user data"
        );
        assert_eq!(
            fs::read(session.root.join("work/evidence.txt")).unwrap(),
            b"User modified the evidence"
        );
        assert!(session.root.join(MARKER).is_file());
        assert!(session.root.join(STATE).is_file());
        assert!(Session::read(&session.root).is_ok());
        assert!(load(&session).unwrap().cleanup.is_none());
    }
    #[test]
    fn stale_or_tampered_previews_lost_exports_and_cancellation_never_remove_files() {
        let temp = tempfile::tempdir().unwrap();
        let session = retained(&temp);
        let output = temp.path().join("confirmed.docx");
        export(&session, &output, &Control::default()).unwrap();
        let preview = inspect(&session, &Control::default()).unwrap();
        let selection = [PathBuf::from("work/result.docx")];
        let mut tampered = preview.clone();
        tampered.files[0].bytes += 1;
        assert!(discard(&session, &tampered, &selection, &Control::default()).is_err());
        let mut tampered = preview.clone();
        tampered.token = "0".repeat(64);
        assert!(discard(&session, &tampered, &selection, &Control::default()).is_err());
        let c = Control::default();
        c.cancelled.store(true, Ordering::Relaxed);
        assert!(discard(&session, &preview, &selection, &c).is_err());
        fs::remove_file(&output).unwrap();
        assert!(discard(&session, &preview, &selection, &Control::default()).is_err());
        let fresh = inspect(&session, &Control::default()).unwrap();
        assert!(!fresh.export_verified);
        assert!(discard(&session, &fresh, &selection, &Control::default()).is_err());
        assert!(session.root.join("work/result.docx").is_file());
        assert!(
            export(
                &session,
                &session.root.join("internal.docx"),
                &Control::default()
            )
            .is_err()
        );
    }
    #[test]
    fn recovery_restores_staged_data_and_never_overwrites_a_collision() {
        let temp = tempfile::tempdir().unwrap();
        let session = retained(&temp);
        let mut state = load(&session).unwrap();
        let file = state.owned[0].clone();
        let directory = "discard-recovery-test";
        fs::create_dir(session.root.join(directory)).unwrap();
        let staged = session.root.join(directory).join("00000.retained");
        fs::rename(session.root.join(&file.path), &staged).unwrap();
        state.cleanup = Some(Journal {
            directory: directory.into(),
            files: vec![file.clone()],
        });
        save(&session, &state).unwrap();
        assert!(inspect(&session, &Control::default()).is_err());
        fs::write(session.root.join(&file.path), b"New user file at old path").unwrap();
        let preserved = recover(&session, &Control::default()).unwrap();
        assert_eq!(preserved.len(), 1);
        assert_eq!(
            fs::read(session.root.join(&file.path)).unwrap(),
            b"New user file at old path"
        );
        assert!(staged.exists());
        fs::remove_file(session.root.join(&file.path)).unwrap();
        assert!(recover(&session, &Control::default()).unwrap().is_empty());
        assert_eq!(
            fs::read(session.root.join(&file.path)).unwrap(),
            b"Disposable completed-output fixture"
        );
        assert!(!staged.exists());
        assert!(load(&session).unwrap().cleanup.is_none());
    }
    #[test]
    fn recovery_export_is_verified_and_collision_protected() {
        let temp = tempfile::tempdir().unwrap();
        let session = retained(&temp);
        let zip = temp.path().join("recovery.zip");
        let receipt = export_recovery(&session, &zip, &Control::default()).unwrap();
        assert_eq!(receipt.kind, "recovery_zip");
        let archive = zip::ZipArchive::new(fs::File::open(&zip).unwrap()).unwrap();
        assert_eq!(archive.len(), 2);
        assert!(export_recovery(&session, &zip, &Control::default()).is_err());
        let preview = inspect(&session, &Control::default()).unwrap();
        let paths = preview
            .files
            .iter()
            .map(|f| f.path.clone())
            .collect::<Vec<_>>();
        let report = discard(&session, &preview, &paths, &Control::default()).unwrap();
        assert_eq!(report.removed.len(), 2);
        assert_eq!(
            inspect(&session, &Control::default()).unwrap().phase,
            "discarded"
        );
        assert!(zip.exists());
    }
    #[test]
    fn non_ascii_unknown_names_do_not_panic_or_become_owned() {
        let actual = Path::new("work/.notegen/source-hash-key");
        assert!(!produced(
            Path::new("work/.notegen/page-00001-key/page-தமிழ்.json"),
            actual
        ));
        assert!(produced(
            Path::new("work/.notegen/source-hash-key/page-00001.json"),
            actual
        ));
        assert!(!produced(
            Path::new("work/.notegen/source-hash-key/user-notes.txt"),
            actual
        ));
        assert!(!relative(Path::new("../outside")));
        assert!(!relative(Path::new("work/bad:stream")));
    }
    #[test]
    fn only_exact_successful_project_is_adopted_and_unknown_siblings_survive_discard() {
        let temp = tempfile::tempdir().unwrap();
        let session = retained(&temp);
        let actual = session.root.join("work/.notegen/source-actual");
        let fake = session.root.join("work/.notegen/source-unknown");
        fs::create_dir_all(&actual).unwrap();
        fs::create_dir_all(&fake).unwrap();
        for project in [&actual, &fake] {
            fs::write(project.join("page-00001.json"), b"Fixture checkpoint").unwrap();
            fs::write(
                project.join("preparation-report.json"),
                b"Fixture preparation report",
            )
            .unwrap();
        }
        let mut state = load(&session).unwrap();
        seal_completed(&session, &mut state, &actual, &Control::default()).unwrap();
        assert!(state.owned.iter().any(|f| f.path == Path::new("work/.notegen/source-actual/preparation-report.json")));
        assert!(
            !state
                .owned
                .iter()
                .any(|f| f.path.starts_with("work/.notegen/source-unknown"))
        );
        save(&session, &state).unwrap();
        export(
            &session,
            &temp.path().join("confirmed.docx"),
            &Control::default(),
        )
        .unwrap();
        let preview = inspect(&session, &Control::default()).unwrap();
        assert!(
            preview
                .preserved
                .iter()
                .any(|f| f.path == Path::new("work/.notegen/source-unknown/page-00001.json"))
        );
        let selected = preview
            .files
            .iter()
            .map(|f| f.path.clone())
            .collect::<Vec<_>>();
        discard(&session, &preview, &selected, &Control::default()).unwrap();
        assert!(fake.join("page-00001.json").is_file());
        assert!(fake.join("preparation-report.json").is_file());
        assert!(!actual.join("preparation-report.json").exists());
    }
}
