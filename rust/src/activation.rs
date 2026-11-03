//! Bounded recovery journal for application-owned AI metadata only.
//! Large model/runtime files are immutable and never moved or deleted here.
use crate::{
    diagnostics::{self, AssetHealth, RepairFile},
    storage::{Control, fingerprint, protected_json, read_json},
    types::*,
    watch::no_links,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};
const FILE_BYTES: u64 = 1_000_000;
const JOURNAL_BYTES: u64 = 64_000_000;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Scope {
    Models,
    Runtime,
    Settings,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Snapshot {
    pub bytes: u64,
    pub sha256: String,
    pub encoded: String,
}
impl Snapshot {
    pub(crate) fn content(&self) -> Result<Vec<u8>> {
        decode(self)
    }
}
#[derive(Clone)]
pub(crate) struct Change {
    pub scope: Scope,
    pub file: String,
    pub expected: Option<Snapshot>,
    pub content: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    scope: Scope,
    file: String,
    old: Option<Snapshot>,
    new: Snapshot,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    id: String,
    models: PathBuf,
    runtime: PathBuf,
    settings: Option<PathBuf>,
    finished: bool,
    entries: Vec<Entry>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub journal: PathBuf,
    pub applied: usize,
    pub recovered: bool,
}
fn fault(message: &str) -> anyhow::Error {
    Fault::new("NG661", message).into()
}
fn payload(bytes: &[u8]) -> Snapshot {
    use sha2::Digest;
    Snapshot {
        bytes: bytes.len() as u64,
        sha256: hex::encode(sha2::Sha256::digest(bytes)),
        encoded: base64::engine::general_purpose::STANDARD.encode(bytes),
    }
}
fn decode(snapshot: &Snapshot) -> Result<Vec<u8>> {
    anyhow::ensure!(
        snapshot.bytes <= FILE_BYTES && snapshot.encoded.len() <= 1_400_000,
        fault("Activation metadata exceeds its bound")
    );
    let bytes = base64::engine::general_purpose::STANDARD.decode(&snapshot.encoded)?;
    let expected = payload(&bytes);
    anyhow::ensure!(
        snapshot.bytes == expected.bytes && snapshot.sha256 == expected.sha256,
        fault("Activation metadata snapshot changed")
    );
    Ok(bytes)
}
pub(crate) fn snapshot(path: &Path, c: &Control) -> Result<Option<Snapshot>> {
    c.check_cancelled()?;
    no_links(path)?;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(
        file.metadata()?.is_file(),
        fault("AI metadata target is not a regular file")
    );
    let mut bytes = vec![];
    file.take(FILE_BYTES + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= FILE_BYTES,
        fault("AI metadata target exceeds its bound")
    );
    Ok(Some(payload(&bytes)))
}
fn same(a: &Option<Snapshot>, b: &Option<Snapshot>) -> bool {
    a.as_ref().map(|s| (&s.sha256, s.bytes)) == b.as_ref().map(|s| (&s.sha256, s.bytes))
}
fn record_id(record: &Record) -> Result<String> {
    fingerprint(&(
        1,
        &record.models,
        &record.runtime,
        &record.settings,
        record.finished,
        &record.entries,
    ))
}
fn target(record: &Record, entry: &Entry) -> Result<PathBuf> {
    anyhow::ensure!(
        !entry.file.is_empty()
            && !entry.file.contains(['/', '\\', ':'])
            && !entry.file.chars().any(char::is_control),
        fault("Invalid activation metadata filename")
    );
    let path = match entry.scope {
        Scope::Models => {
            let id = entry
                .file
                .strip_suffix(".notegen.json")
                .or_else(|| entry.file.strip_suffix(".previous.json"))
                .ok_or_else(|| fault("Activation cannot change arbitrary registry files"))?;
            anyhow::ensure!(
                crate::writers::valid_id(id),
                fault("Invalid activation reader ID")
            );
            record.models.join(&entry.file)
        }
        Scope::Runtime => {
            anyhow::ensure!(
                ["active-ai-runtime.json", "previous-ai-runtime.json"]
                    .contains(&entry.file.as_str()),
                fault("Activation cannot change arbitrary runtime files")
            );
            record.runtime.join(&entry.file)
        }
        Scope::Settings => {
            let path = record
                .settings
                .as_ref()
                .ok_or_else(|| fault("No settings target was selected"))?;
            anyhow::ensure!(
                path.file_name()
                    .is_some_and(|name| name == entry.file.as_str()),
                fault("Activation settings target changed")
            );
            path.clone()
        }
    };
    no_links(&path)?;
    Ok(path)
}
fn validate(record: &Record) -> Result<()> {
    anyhow::ensure!(
        record.schema == 1
            && record.entries.len() <= 131
            && !record.entries.is_empty()
            && record.models.is_absolute()
            && record.runtime.is_absolute()
            && record.id == record_id(record)?
            && serde_json::to_vec_pretty(record)?.len() as u64 <= JOURNAL_BYTES,
        fault("Invalid AI activation journal")
    );
    let mut seen = std::collections::BTreeSet::new();
    for entry in &record.entries {
        let path = target(record, entry)?;
        anyhow::ensure!(
            seen.insert(path),
            fault("Duplicate activation metadata target")
        );
        decode(&entry.new)?;
        if let Some(old) = &entry.old {
            decode(old)?;
        }
        if entry.scope == Scope::Settings {
            let known = serde_json::to_value(Settings::default())?;
            let keys = known.as_object().unwrap();
            for snapshot in std::iter::once(&entry.new).chain(entry.old.iter()) {
                let bytes = decode(snapshot)?;
                let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                let object = value.as_object().ok_or_else(|| {
                    fault("Settings activation requires an application settings object")
                })?;
                anyhow::ensure!(
                    ["preset", "server", "models_dir"]
                        .iter()
                        .all(|key| object.contains_key(*key))
                        && object.keys().all(|key| keys.contains_key(key)),
                    fault(
                        "Existing settings format is not recognized; preserve it and choose a new settings output"
                    )
                );
                serde_json::from_value::<Settings>(value)?.validate()?;
            }
        }
    }
    Ok(())
}
fn root_for(models: &Path, _settings: Option<&Path>) -> Result<PathBuf> {
    let base = models;
    no_links(base)?;
    fs::create_dir_all(base)?;
    let root = base.canonicalize()?.join(".notegen-activation");
    no_links(&root)?;
    fs::create_dir_all(&root)?;
    Ok(root)
}
fn lock(root: &Path, c: &Control) -> Result<fs::File> {
    let path = root.join("activation.lock");
    no_links(&path)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    loop {
        c.check_cancelled()?;
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(25)),
            Err(error) => return Err(error.into()),
        }
    }
}
fn stage(parent: &Path, snapshot: &Snapshot) -> Result<tempfile::TempPath> {
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&decode(snapshot)?)?;
    file.as_file().sync_all()?;
    Ok(file.into_temp_path())
}
fn publish(path: &Path, new: &Snapshot, old: &Option<Snapshot>, c: &Control) -> Result<()> {
    let expected = RepairFile {
        key: "ai-activation-metadata".into(),
        expected_bytes: new.bytes,
        expected_sha256: new.sha256.clone(),
        current_bytes: old.as_ref().map(|s| s.bytes),
        current_sha256: old.as_ref().map(|s| s.sha256.clone()),
        health: if old.is_some() {
            AssetHealth::Corrupt
        } else {
            AssetHealth::Missing
        },
    };
    diagnostics::publish_repair(stage(path.parent().unwrap(), new)?, path, &expected, c)?;
    Ok(())
}
fn save(path: &Path, record: &mut Record) -> Result<()> {
    record.id = record_id(record)?;
    validate(record)?;
    crate::storage::atomic_json(path, record)
}
fn rollback(path: &Path, record: &mut Record) -> Result<usize> {
    // Small metadata recovery must finish even after the initiating cancellation.
    let recovery = Control::default();
    let mut restored = 0;
    for (index, entry) in record.entries.iter().enumerate().rev() {
        let target = target(record, entry)?;
        let current = snapshot(&target, &recovery)?;
        if same(&current, &entry.old) {
            continue;
        }
        if !same(&current, &Some(entry.new.clone())) && current.is_some() {
            return Err(fault(
                "Activation recovery found a concurrent metadata change; all versions are retained for manual inspection",
            ));
        }
        if let Some(old) = &entry.old {
            publish(&target, old, &current, &recovery)?;
        } else if current.is_some() {
            let saved = path
                .parent()
                .unwrap()
                .join(format!("unactivated-{index}.json"));
            no_links(&saved)?;
            let mut captured = tempfile::TempPath::try_from_path(target.clone())?;
            captured.disable_cleanup(true);
            captured
                .persist_noclobber(&saved)
                .map_err(|error| error.error)?;
            if !same(&snapshot(&saved, &recovery)?, &Some(entry.new.clone())) {
                let mut source = tempfile::TempPath::try_from_path(saved)?;
                source.disable_cleanup(true);
                let _ = source.persist_noclobber(&target);
                return Err(fault(
                    "Concurrent metadata changed during recovery; captured bytes were preserved",
                ));
            }
        }
        restored += 1;
    }
    record.finished = true;
    save(path, record)?;
    Ok(restored)
}
fn recover_locked(
    root: &Path,
    models: &Path,
    runtime: &Path,
    settings: Option<&Path>,
) -> Result<Vec<Receipt>> {
    let mut receipts = vec![];
    let mut seen = 0;
    for item in fs::read_dir(root)? {
        let folder = item?.path();
        if !folder
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("transaction-"))
        {
            continue;
        }
        seen += 1;
        anyhow::ensure!(
            seen <= 1000,
            fault("Too many retained activation transactions")
        );
        no_links(&folder)?;
        let path = folder.join("journal.json");
        if !path.exists() {
            continue;
        }
        no_links(&path)?;
        let mut record: Record = read_json(&path, JOURNAL_BYTES)?;
        validate(&record)?;
        if record.finished {
            continue;
        }
        anyhow::ensure!(
            record.models == models
                && record.runtime == runtime
                && record.settings.as_deref() == settings,
            fault(
                "Activation recovery roots changed; preserve this journal and inspect it explicitly"
            )
        );
        if !record.finished {
            let applied = rollback(&path, &mut record)?;
            receipts.push(Receipt {
                journal: path,
                applied,
                recovered: true,
            });
        }
    }
    Ok(receipts)
}
/// Recover every unfinished transaction belonging to this registry. Targets are
/// restricted by the journal to AI descriptors and a recognized settings object.
pub fn recover_all(models: &Path, c: &Control) -> Result<Vec<Receipt>> {
    let models = canonical_folder(models)?;
    let root = root_for(&models, None)?;
    let _guard = lock(&root, c)?;
    let mut out = vec![];
    let mut count = 0;
    for item in fs::read_dir(&root)? {
        let folder = item?.path();
        if !folder
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("transaction-"))
        {
            continue;
        }
        count += 1;
        anyhow::ensure!(
            count <= 1000,
            fault("Too many retained activation transactions")
        );
        no_links(&folder)?;
        let path = folder.join("journal.json");
        if !path.exists() {
            continue;
        }
        no_links(&path)?;
        let mut record: Record = read_json(&path, JOURNAL_BYTES)?;
        validate(&record)?;
        if record.finished {
            continue;
        }
        anyhow::ensure!(
            record.models == models,
            fault("Activation registry moved; explicit recovery migration is required")
        );
        let applied = rollback(&path, &mut record)?;
        out.push(Receipt {
            journal: path,
            applied,
            recovered: true,
        });
    }
    Ok(out)
}
fn canonical_folder(path: &Path) -> Result<PathBuf> {
    no_links(path)?;
    fs::create_dir_all(path)?;
    Ok(path.canonicalize()?)
}
fn canonical_settings(path: Option<&Path>) -> Result<Option<PathBuf>> {
    path.map(|path| {
        no_links(path)?;
        let absolute = std::path::absolute(path)?;
        let parent = canonical_folder(
            absolute
                .parent()
                .ok_or_else(|| fault("Choose a settings filename"))?,
        )?;
        Ok(parent.join(
            absolute
                .file_name()
                .ok_or_else(|| fault("Choose a settings filename"))?,
        ))
    })
    .transpose()
}
pub fn recover(
    models: &Path,
    runtime: &Path,
    settings: Option<&Path>,
    c: &Control,
) -> Result<Vec<Receipt>> {
    let models = canonical_folder(models)?;
    let runtime = canonical_folder(runtime)?;
    let settings = canonical_settings(settings)?;
    let root = root_for(&models, settings.as_deref())?;
    let _lock = lock(&root, c)?;
    recover_locked(&root, &models, &runtime, settings.as_deref())
}
/// Crate-private: new working model/runtime pointers require freshly verified
/// opaque canary proofs. Explicit first-install settings may select only a
/// retained pending runtime, with no active model/runtime pointer publication.
pub(crate) fn commit(
    models: &Path,
    runtime: &Path,
    settings: Option<&Path>,
    changes: Vec<Change>,
    c: &Control,
) -> Result<Receipt> {
    commit_with(models, runtime, settings, changes, c, |_| Ok(()))
}
fn commit_with(
    models: &Path,
    runtime: &Path,
    settings: Option<&Path>,
    changes: Vec<Change>,
    c: &Control,
    mut after_write: impl FnMut(usize) -> Result<()>,
) -> Result<Receipt> {
    c.check_cancelled()?;
    let models = canonical_folder(models)?;
    let runtime = canonical_folder(runtime)?;
    let settings = canonical_settings(settings)?;
    let root = root_for(&models, settings.as_deref())?;
    let _lock = lock(&root, c)?;
    recover_locked(&root, &models, &runtime, settings.as_deref())?;
    let entries = changes
        .into_iter()
        .map(|change| Entry {
            scope: change.scope,
            file: change.file,
            old: change.expected,
            new: payload(&change.content),
        })
        .collect();
    let mut record = Record {
        schema: 1,
        id: String::new(),
        models,
        runtime,
        settings,
        finished: false,
        entries,
    };
    record.id = record_id(&record)?;
    validate(&record)?;
    for entry in &record.entries {
        anyhow::ensure!(
            same(&snapshot(&target(&record, entry)?, c)?, &entry.old),
            fault("AI metadata changed while the candidate was being validated")
        );
    }
    let folder = tempfile::Builder::new()
        .prefix("transaction-")
        .tempdir_in(&root)?;
    let path = folder.path().join("journal.json");
    protected_json(&path, &record)?;
    let folder = folder.keep();
    let path = folder.join("journal.json");
    let result = (|| -> Result<()> {
        for (index, entry) in record.entries.iter().enumerate() {
            c.check_cancelled()?;
            publish(&target(&record, entry)?, &entry.new, &entry.old, c)?;
            after_write(index)?;
        }
        c.check_cancelled()?;
        Ok(())
    })();
    if let Err(error) = result {
        if let Err(recovery) = rollback(&path, &mut record) {
            return Err(error.context(format!("AI metadata recovery needs inspection: {recovery}")));
        }
        return Err(error);
    }
    record.finished = true;
    if let Err(error) = save(&path, &mut record) {
        record.finished = false;
        if let Err(recovery) = rollback(&path, &mut record) {
            return Err(error.context(format!(
                "AI metadata completion/recovery needs inspection: {recovery}"
            )));
        }
        return Err(error);
    }
    Ok(Receipt {
        journal: path,
        applied: record.entries.len(),
        recovered: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    struct Fixture {
        _root: tempfile::TempDir,
        models: PathBuf,
        runtime: PathBuf,
        settings: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let models = root.path().join("models");
            let runtime = root.path().join("runtime");
            fs::create_dir_all(&models).unwrap();
            fs::create_dir_all(&runtime).unwrap();
            let settings = root.path().join("settings.json");
            let old = Settings {
                models_dir: models.clone(),
                server: runtime.join("old-server.exe"),
                shared_model_store: root.path().join("store"),
                ..Default::default()
            };
            fs::write(&settings, serde_json::to_vec_pretty(&old).unwrap()).unwrap();
            fs::write(models.join("reader.notegen.json"), b"old-model").unwrap();
            fs::write(runtime.join("active-ai-runtime.json"), b"old-runtime").unwrap();
            Self {
                _root: root,
                models,
                runtime,
                settings,
            }
        }
        fn changes(&self) -> Vec<Change> {
            let control = Control::default();
            let mut next: Settings = read_json(&self.settings, FILE_BYTES).unwrap();
            next.server = self.runtime.join("new-server.exe");
            [
                (Scope::Models, "reader.notegen.json", b"new-model".to_vec()),
                (
                    Scope::Runtime,
                    "active-ai-runtime.json",
                    b"new-runtime".to_vec(),
                ),
                (
                    Scope::Settings,
                    "settings.json",
                    serde_json::to_vec_pretty(&next).unwrap(),
                ),
            ]
            .into_iter()
            .map(|(scope, file, content)| {
                let path = match scope {
                    Scope::Models => self.models.join(file),
                    Scope::Runtime => self.runtime.join(file),
                    Scope::Settings => self.settings.clone(),
                };
                Change {
                    scope,
                    file: file.into(),
                    expected: snapshot(&path, &control).unwrap(),
                    content,
                }
            })
            .collect()
        }
        fn journal(&self) -> PathBuf {
            let root = root_for(&self.models, None).unwrap();
            fs::read_dir(root)
                .unwrap()
                .map(|entry| entry.unwrap().path().join("journal.json"))
                .find(|path| path.is_file())
                .unwrap()
        }
        fn assert_originals(&self, settings: &[u8]) {
            assert_eq!(
                fs::read(self.models.join("reader.notegen.json")).unwrap(),
                b"old-model"
            );
            assert_eq!(
                fs::read(self.runtime.join("active-ai-runtime.json")).unwrap(),
                b"old-runtime"
            );
            assert_eq!(fs::read(&self.settings).unwrap(), settings);
        }
    }
    #[test]
    fn successful_commit_binds_models_runtime_settings_and_retains_original_snapshots() {
        let f = Fixture::new();
        let original = fs::read(&f.settings).unwrap();
        let receipt = commit(
            &f.models,
            &f.runtime,
            Some(&f.settings),
            f.changes(),
            &Control::default(),
        )
        .unwrap();
        assert_eq!(receipt.applied, 3);
        assert!(!receipt.recovered);
        let record: Record = read_json(&receipt.journal, JOURNAL_BYTES).unwrap();
        validate(&record).unwrap();
        assert!(record.finished);
        assert_eq!(
            record.entries[2].old.as_ref().unwrap().content().unwrap(),
            original
        );
        assert_eq!(
            fs::read(f.models.join("reader.notegen.json")).unwrap(),
            b"new-model"
        );
        assert_eq!(
            fs::read(f.runtime.join("active-ai-runtime.json")).unwrap(),
            b"new-runtime"
        );
        let current: Settings = read_json(&f.settings, FILE_BYTES).unwrap();
        assert_eq!(current.server, f.runtime.join("new-server.exe"));
        assert!(
            recover_all(&f.models, &Control::default())
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn stale_preview_rejects_every_write_before_publishing_any_target() {
        let f = Fixture::new();
        let changes = f.changes();
        let original = fs::read(&f.settings).unwrap();
        fs::write(
            f.runtime.join("active-ai-runtime.json"),
            b"concurrent-runtime",
        )
        .unwrap();
        assert!(
            commit(
                &f.models,
                &f.runtime,
                Some(&f.settings),
                changes,
                &Control::default()
            )
            .is_err()
        );
        assert_eq!(
            fs::read(f.models.join("reader.notegen.json")).unwrap(),
            b"old-model"
        );
        assert_eq!(
            fs::read(f.runtime.join("active-ai-runtime.json")).unwrap(),
            b"concurrent-runtime"
        );
        assert_eq!(fs::read(&f.settings).unwrap(), original);
    }
    #[test]
    fn failure_after_each_metadata_write_rolls_back_every_scope() {
        for stop in 0..3 {
            let f = Fixture::new();
            let original = fs::read(&f.settings).unwrap();
            let error = commit_with(
                &f.models,
                &f.runtime,
                Some(&f.settings),
                f.changes(),
                &Control::default(),
                |index| {
                    anyhow::ensure!(index != stop, "synthetic metadata publication failure");
                    Ok(())
                },
            )
            .unwrap_err();
            assert!(error.to_string().contains("synthetic"));
            f.assert_originals(&original);
            let record: Record = read_json(&f.journal(), JOURNAL_BYTES).unwrap();
            assert!(record.finished);
        }
    }
    #[test]
    fn cancellation_after_publication_rolls_back_without_ignoring_the_original_cancel() {
        let f = Fixture::new();
        let original = fs::read(&f.settings).unwrap();
        let control = Control::default();
        let error = commit_with(
            &f.models,
            &f.runtime,
            Some(&f.settings),
            f.changes(),
            &control,
            |_| {
                control.cancelled.store(true, Ordering::Release);
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG900");
        f.assert_originals(&original);
        assert!(control.cancelled.load(Ordering::Acquire));
    }
    #[test]
    fn interrupted_first_install_is_recovered_and_unactivated_bytes_are_retained() {
        let f = Fixture::new();
        let target = f.models.join("new-reader.notegen.json");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = commit_with(
                &f.models,
                &f.runtime,
                None,
                vec![Change {
                    scope: Scope::Models,
                    file: "new-reader.notegen.json".into(),
                    expected: None,
                    content: b"unactivated-new-reader".to_vec(),
                }],
                &Control::default(),
                |_| panic!("simulate process interruption after metadata publish"),
            );
        }));
        assert!(result.is_err());
        assert!(target.is_file());
        let receipt = recover_all(&f.models, &Control::default()).unwrap();
        assert_eq!(receipt.len(), 1);
        assert!(receipt[0].recovered);
        assert_eq!(receipt[0].applied, 1);
        assert!(!target.exists());
        assert_eq!(
            fs::read(
                receipt[0]
                    .journal
                    .parent()
                    .unwrap()
                    .join("unactivated-0.json")
            )
            .unwrap(),
            b"unactivated-new-reader"
        );
        assert!(
            recover_all(&f.models, &Control::default())
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn interrupted_existing_install_restores_models_runtime_and_settings_on_reopen() {
        let f = Fixture::new();
        let original = fs::read(&f.settings).unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = commit_with(
                &f.models,
                &f.runtime,
                Some(&f.settings),
                f.changes(),
                &Control::default(),
                |index| {
                    if index == 2 {
                        panic!("simulate interrupted completion marker");
                    }
                    Ok(())
                },
            );
        }));
        assert!(result.is_err());
        assert_eq!(
            recover_all(&f.models, &Control::default()).unwrap().len(),
            1
        );
        f.assert_originals(&original);
    }
    #[test]
    fn recovery_preserves_concurrent_metadata_and_both_journal_snapshots() {
        let f = Fixture::new();
        let target = f.models.join("reader.notegen.json");
        let first = f.changes().remove(0);
        let error = commit_with(
            &f.models,
            &f.runtime,
            None,
            vec![first],
            &Control::default(),
            |_| {
                fs::write(&target, b"concurrent-user-registration")?;
                anyhow::bail!("synthetic failure after concurrent replacement")
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("inspection"));
        assert_eq!(fs::read(target).unwrap(), b"concurrent-user-registration");
        assert!(recover_all(&f.models, &Control::default()).is_err());
        let record: Record = read_json(&f.journal(), JOURNAL_BYTES).unwrap();
        assert!(!record.finished);
        assert_eq!(
            record.entries[0].old.as_ref().unwrap().content().unwrap(),
            b"old-model"
        );
        assert_eq!(record.entries[0].new.content().unwrap(), b"new-model");
    }
    #[test]
    fn arbitrary_targets_duplicate_targets_and_non_settings_objects_are_rejected() {
        let f = Fixture::new();
        let original = fs::read(&f.settings).unwrap();
        for (scope, file, content) in [
            (Scope::Models, "../scan.png", b"bad".to_vec()),
            (Scope::Runtime, "user-scans.json", b"bad".to_vec()),
            (
                Scope::Settings,
                "settings.json",
                br#"{"private":"unrecognized user document"}"#.to_vec(),
            ),
        ] {
            assert!(
                commit(
                    &f.models,
                    &f.runtime,
                    Some(&f.settings),
                    vec![Change {
                        scope,
                        file: file.into(),
                        expected: None,
                        content
                    }],
                    &Control::default()
                )
                .is_err()
            );
        }
        let mut duplicate = f.changes();
        duplicate.push(duplicate[0].clone());
        assert!(
            commit(
                &f.models,
                &f.runtime,
                Some(&f.settings),
                duplicate,
                &Control::default()
            )
            .is_err()
        );
        f.assert_originals(&original);
    }
    #[cfg(unix)]
    #[test]
    fn metadata_symlink_is_never_followed_or_replaced() {
        let f = Fixture::new();
        let source = f._root.path().join("private-scan.json");
        fs::write(&source, b"private original").unwrap();
        std::os::unix::fs::symlink(&source, f.models.join("linked.notegen.json")).unwrap();
        assert!(
            commit(
                &f.models,
                &f.runtime,
                None,
                vec![Change {
                    scope: Scope::Models,
                    file: "linked.notegen.json".into(),
                    expected: None,
                    content: b"new".to_vec()
                }],
                &Control::default()
            )
            .is_err()
        );
        assert_eq!(fs::read(source).unwrap(), b"private original");
    }
}
