//! Exact private project dependencies. Resolution never changes global registrations.
use crate::{
    canary::{self, RuntimeIdentity},
    model_store::{Kind, ObjectId, Store, VersionReference},
    planning::Model,
    storage::{Activity, Control, Project, fingerprint, read_json},
    types::*,
    watch::no_links,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

const LIMIT: u64 = 4_000_000;
pub const LIMITS: &str = "A lock selects exact local model/projector bytes and runtime executable/adjacent libraries without changing the global registry. Operating-system libraries, drivers and hardware are not pinned. HTTP readers cannot be restored by content hash. Legacy projects that saved only model IDs cannot reveal their original revisions: manual pinning explicitly selects versions for future work. Unknown/custom missing bytes require an exact-file import; they are never guessed or substituted. Channel preferences govern explicit update previews and never silently change a lock.";
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    RecordedConversion,
    FutureSelection,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub channel: UpdateChannel,
    pub defer_until: Option<chrono::DateTime<chrono::Utc>>,
    pub prefer_locked: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            channel: UpdateChannel::Stable,
            defer_until: None,
            prefer_locked: true,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSource {
    pub build: String,
    pub backend: Backend,
    pub assets: Vec<crate::setup::Download>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimePin {
    pub identity: RuntimeIdentity,
    pub source: Option<RuntimeSource>,
    pub backend: Backend,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lockfile {
    pub schema: u32,
    pub id: String,
    pub input_sha256: String,
    pub origin: Origin,
    pub preferences: Preferences,
    pub models: Vec<VersionReference>,
    pub runtime: Option<RuntimePin>,
}
pub struct Resolved {
    pub settings: Settings,
    pub models: Vec<Model>,
    pub input_sha256: String,
    pub lock_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WriteReceipt {
    pub path: PathBuf,
    pub lock_id: String,
    pub previous_copy: Option<PathBuf>,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MissingObject {
    pub object: ObjectId,
    pub kind: Kind,
    pub model_id: Option<String>,
    pub role: String,
    pub publisher_url: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestorePlan {
    pub lock_path: PathBuf,
    pub lock_id: String,
    pub missing: Vec<MissingObject>,
    pub missing_object_bytes: u64,
    pub runtime_materialization_needed: bool,
    pub additional_disk_bytes: u64,
    pub download_bytes: u64,
    pub token: String,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RestoreReport {
    pub restored_objects: usize,
    pub remaining_manual: Vec<MissingObject>,
    pub note: String,
}
fn fault(message: &str) -> anyhow::Error {
    Fault::new("NG670", message).into()
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn lock_id(lock: &Lockfile) -> Result<String> {
    fingerprint(&(
        1,
        &lock.input_sha256,
        lock.origin,
        &lock.preferences,
        &lock.models,
        &lock.runtime,
    ))
}
fn validate_reference(reference: &VersionReference) -> Result<()> {
    crate::models::validate_model(&reference.model)?;
    reference.weights.validate()?;
    reference.projector.validate()?;
    anyhow::ensure!(
        reference.schema == 1
            && digest(&reference.id)
            && reference.model.endpoint.is_none()
            && reference.model.weights.as_os_str().is_empty()
            && reference.model.projector.as_os_str().is_empty()
            && reference.model.sha256
                == fingerprint(&(&reference.weights.sha256, &reference.projector.sha256))?
            && reference.id
                == fingerprint(&(
                    1,
                    &reference.model,
                    &reference.weights,
                    &reference.projector
                ))?
            && serde_json::to_vec(reference)?.len() <= 100000,
        fault("Pinned reader reference is inconsistent or excessive")
    );
    Ok(())
}
pub fn validate(lock: &Lockfile) -> Result<()> {
    anyhow::ensure!(
        lock.schema == 1
            && digest(&lock.input_sha256)
            && digest(&lock.id)
            && lock.models.len() <= 32
            && lock.runtime.is_some() == !lock.models.is_empty()
            && lock.id == lock_id(lock)?
            && serde_json::to_vec_pretty(lock)?.len() <= LIMIT as usize,
        fault("Invalid exact project model lock")
    );
    let mut ids = BTreeSet::new();
    for model in &lock.models {
        validate_reference(model)?;
        anyhow::ensure!(
            ids.insert(&model.model.id),
            fault("Project lock has duplicate reader IDs")
        );
    }
    if let Some(pin) = &lock.runtime {
        canary::validate_runtime_identity(&pin.identity)?;
        if let Some(source) = &pin.source {
            crate::maintenance::validate_runtime_release(&source.build, &source.assets)?;
        }
    }
    Ok(())
}
pub fn read(path: &Path, c: &Control) -> Result<Lockfile> {
    c.check()?;
    no_links(path)?;
    let lock: Lockfile = read_json(path, LIMIT)?;
    validate(&lock)?;
    Ok(lock)
}
fn runtime_object(file: &canary::RuntimeFile) -> ObjectId {
    ObjectId {
        sha256: file.sha256.clone(),
        bytes: file.bytes,
    }
}
fn runtime_namespace(store: &Store, pin: &RuntimePin) -> PathBuf {
    store.root.join("runtimes").join(&pin.identity.fingerprint)
}
fn cached_runtime(store: &Store, pin: &RuntimePin, c: &Control) -> Result<Option<PathBuf>> {
    let namespace = runtime_namespace(store, pin);
    no_links(&namespace)?;
    if !namespace.exists() {
        return Ok(None);
    }
    let mut folders = vec![];
    for entry in fs::read_dir(&namespace)? {
        c.check()?;
        anyhow::ensure!(
            folders.len() < 1000,
            fault("Too many materialized runtime versions")
        );
        folders.push(entry?.path());
    }
    folders.sort();
    for folder in folders {
        no_links(&folder)?;
        let marker = folder.join("notegen-runtime.json");
        if !marker.exists() {
            continue;
        }
        no_links(&marker)?;
        let found: RuntimeIdentity = read_json(&marker, 1_000_000)?;
        canary::validate_runtime_identity(&found)?;
        anyhow::ensure!(
            found == pin.identity,
            fault("Stored runtime marker disagrees with its namespace")
        );
        let server = folder.join(&found.server);
        anyhow::ensure!(
            canary::runtime_identity(&server, c)? == found,
            fault("Materialized pinned runtime changed; preserve it and restore to a clean store")
        );
        return Ok(Some(server));
    }
    Ok(None)
}
fn materialize_runtime(store: &Store, pin: &RuntimePin, c: &Control) -> Result<PathBuf> {
    canary::validate_runtime_identity(&pin.identity)?;
    let lockpath = store
        .root
        .join("locks")
        .join(format!("runtime-{}.lock", pin.identity.fingerprint));
    no_links(&lockpath)?;
    let guard = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lockpath)?;
    loop {
        c.check()?;
        match guard.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(50)),
            Err(error) => return Err(error.into()),
        }
    }
    if let Some(server) = cached_runtime(store, pin, c)? {
        return Ok(server);
    }
    let namespace = runtime_namespace(store, pin);
    no_links(&namespace)?;
    fs::create_dir_all(&namespace)?;
    let staged = tempfile::Builder::new()
        .prefix("version-")
        .tempdir_in(&namespace)?;
    for file in &pin.identity.files {
        c.check()?;
        let id = runtime_object(file);
        let source = store
            .resolve(&id, Kind::RuntimeFile, c)?
            .ok_or_else(|| fault("Pinned runtime object is missing"))?;
        let path = staged.path().join(&file.name);
        let mut source = fs::File::open(source)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let mut buffer = [0u8; 65536];
        let mut bytes = 0;
        loop {
            c.check()?;
            let count = source.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            anyhow::ensure!(
                bytes <= id.bytes,
                fault("Runtime object grew during materialization")
            );
            output.write_all(&buffer[..count])?;
        }
        output.sync_all()?;
        drop(output);
        anyhow::ensure!(
            crate::model_store::inspect(&path, id.bytes, Kind::RuntimeFile, c)? == id,
            fault("Pinned runtime copy failed verification")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &path,
                fs::Permissions::from_mode(if file.name == pin.identity.server {
                    0o555
                } else {
                    0o444
                }),
            )?;
        }
        #[cfg(not(unix))]
        {
            let mut permissions = fs::metadata(&path)?.permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&path, permissions)?;
        }
    }
    let folder = staged.keep();
    crate::storage::protected_json(&folder.join("notegen-runtime.json"), &pin.identity)?;
    let server = folder.join(&pin.identity.server);
    anyhow::ensure!(
        canary::runtime_identity(&server, c)? == pin.identity,
        fault("Materialized runtime identity changed")
    );
    Ok(server)
}
pub fn capture(
    project: &Project,
    models: &[Model],
    settings: &Settings,
    origin: Origin,
    c: &Control,
) -> Result<Lockfile> {
    c.check()?;
    anyhow::ensure!(
        digest(&project.manifest.input_sha256)
            && models.len() <= 32
            && models
                .iter()
                .all(|m| project.manifest.models.contains(&m.id)),
        fault("Selected readers do not belong to this project")
    );
    let store = Store::open(&settings.shared_model_store)?;
    let mut refs = vec![];
    for model in models {
        refs.push(store.adopt_model(model, c)?.reference);
    }
    let runtime = if refs.is_empty() {
        None
    } else {
        let identity = canary::runtime_identity(&settings.server, c)?;
        let parent = settings.server.parent().unwrap();
        for file in &identity.files {
            store.ingest(
                &parent.join(&file.name),
                &runtime_object(file),
                Kind::RuntimeFile,
                c,
            )?;
        }
        let metadata = parent.join("notegen-runtime-source.json");
        no_links(&metadata)?;
        let source = if metadata.exists() {
            let source: RuntimeSource = read_json(&metadata, 1_000_000)?;
            crate::maintenance::validate_runtime_release(&source.build, &source.assets)?;
            Some(source)
        } else {
            None
        };
        let backend = source
            .as_ref()
            .map_or(settings.backend, |source| source.backend);
        let pin = RuntimePin {
            identity,
            source,
            backend,
        };
        materialize_runtime(&store, &pin, c)?;
        Some(pin)
    };
    let mut lock = Lockfile {
        schema: 1,
        id: String::new(),
        input_sha256: project.manifest.input_sha256.clone(),
        origin,
        preferences: Preferences::default(),
        models: refs,
        runtime,
    };
    lock.id = lock_id(&lock)?;
    validate(&lock)?;
    Ok(lock)
}
pub fn write(
    project: &Project,
    lock: &Lockfile,
    expected_current: Option<&str>,
    c: &Control,
) -> Result<WriteReceipt> {
    c.check()?;
    validate(lock)?;
    anyhow::ensure!(
        lock.input_sha256 == project.manifest.input_sha256,
        fault("Project lock belongs to another source")
    );
    no_links(&project.root)?;
    let current: crate::storage::Manifest =
        read_json(&project.root.join("project.json"), 1_000_000)?;
    anyhow::ensure!(
        fingerprint(&current)? == fingerprint(&project.manifest)?,
        fault("Project manifest changed after dependency selection")
    );
    let path = project.root.join("model-lock.json");
    no_links(&path)?;
    let old = if path.exists() {
        let old = read(&path, c)?;
        anyhow::ensure!(
            Some(old.id.as_str()) == expected_current,
            fault("Project lock changed after preview")
        );
        Some(crate::model_store::inspect(
            &path,
            LIMIT,
            Kind::RuntimeFile,
            c,
        )?)
    } else {
        anyhow::ensure!(
            expected_current.is_none(),
            fault("Expected project lock disappeared")
        );
        None
    };
    let bytes = serde_json::to_vec_pretty(lock)?;
    let mut staged = tempfile::NamedTempFile::new_in(&project.root)?;
    staged.write_all(&bytes)?;
    staged.as_file().sync_all()?;
    use sha2::Digest;
    let expected = crate::diagnostics::RepairFile {
        key: "project-model-lock".into(),
        expected_bytes: bytes.len() as u64,
        expected_sha256: hex::encode(sha2::Sha256::digest(&bytes)),
        current_bytes: old.as_ref().map(|id| id.bytes),
        current_sha256: old.as_ref().map(|id| id.sha256.clone()),
        health: if old.is_some() {
            crate::diagnostics::AssetHealth::Corrupt
        } else {
            crate::diagnostics::AssetHealth::Missing
        },
    };
    let previous_copy =
        crate::diagnostics::publish_repair(staged.into_temp_path(), &path, &expected, c)?;
    Ok(WriteReceipt {
        path,
        lock_id: lock.id.clone(),
        previous_copy,
        note: LIMITS.into(),
    })
}
pub fn set_preferences(
    project: &Project,
    expected_id: &str,
    preferences: Preferences,
    c: &Control,
) -> Result<WriteReceipt> {
    let path = project.root.join("model-lock.json");
    let mut lock = read(&path, c)?;
    anyhow::ensure!(
        lock.id == expected_id,
        fault("Project preferences changed after preview")
    );
    lock.preferences = preferences;
    lock.id = lock_id(&lock)?;
    write(project, &lock, Some(expected_id), c)
}
pub fn update_allowed(
    preferences: &Preferences,
    prerelease: Option<bool>,
    manual: bool,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if !manual
        && (preferences.prefer_locked || preferences.defer_until.is_some_and(|date| now < date))
    {
        return false;
    }
    match prerelease {
        Some(true) => preferences.channel == UpdateChannel::Preview,
        Some(false) => true,
        None => manual,
    }
}
fn resolve_model(store: &Store, reference: &VersionReference, c: &Control) -> Result<Model> {
    validate_reference(reference)?;
    let mut model = reference.model.clone();
    model.weights = store
        .resolve(&reference.weights, Kind::Gguf, c)?
        .ok_or_else(|| {
            fault("Pinned weights are missing; preview and restore exact dependencies")
        })?;
    model.projector = store
        .resolve(&reference.projector, Kind::Gguf, c)?
        .ok_or_else(|| {
            fault("Pinned projector is missing; preview and restore exact dependencies")
        })?;
    Ok(model)
}
pub fn resolve_settings(settings: &Settings, c: &Control) -> Result<Option<Resolved>> {
    if settings.preset == Preset::Fast {
        return Ok(None);
    }
    let Some(path) = &settings.project_lock else {
        return Ok(None);
    };
    let lock = read(path, c)?;
    let store = Store::read(&settings.shared_model_store)?;
    let models = lock
        .models
        .iter()
        .map(|m| resolve_model(&store, m, c))
        .collect::<Result<Vec<_>>>()?;
    let mut safe = settings.clone();
    safe.automatic_models = false;
    safe.model_ids = models.iter().map(|m| m.id.clone()).collect();
    safe.external_server = false;
    if let Some(pin) = &lock.runtime {
        safe.server = cached_runtime(&store, pin, c)?.ok_or_else(|| {
            fault("Exact pinned runtime is unavailable; restore this project's dependencies")
        })?;
        safe.backend = pin.backend;
    }
    Ok(Some(Resolved {
        settings: safe,
        models,
        input_sha256: lock.input_sha256,
        lock_id: lock.id,
    }))
}
pub fn resolve_for_project(
    project: &Project,
    settings: &Settings,
    c: &Control,
) -> Result<Option<Resolved>> {
    let mut selected = settings.clone();
    let path = project.root.join("model-lock.json");
    if selected.project_lock.is_none() && path.exists() {
        selected.project_lock = Some(path);
    }
    let resolved = resolve_settings(&selected, c)?;
    anyhow::ensure!(
        resolved
            .as_ref()
            .is_none_or(|r| r.input_sha256 == project.manifest.input_sha256),
        fault("Selected model lock belongs to another project source")
    );
    Ok(resolved)
}
pub fn ensure_conversion_lock(
    project: &Project,
    models: &[Model],
    settings: &Settings,
    c: &Control,
) -> Result<Option<PathBuf>> {
    if settings.preset == Preset::Fast
        || models.is_empty()
        || models.iter().any(|m| m.endpoint.is_some())
    {
        return Ok(None);
    }
    let path = project.root.join("model-lock.json");
    if path.exists() {
        let lock = read(&path, c)?;
        anyhow::ensure!(
            lock.input_sha256 == project.manifest.input_sha256,
            fault("Existing exact project lock belongs to another source")
        );
        return Ok(Some(path));
    }
    if let Some(selected) = &settings.project_lock {
        let lock = read(selected, c)?;
        write(project, &lock, None, c)?;
    } else {
        let lock = capture(project, models, settings, Origin::RecordedConversion, c)?;
        write(project, &lock, None, c)?;
    }
    Ok(Some(path))
}
fn model_url(reference: &VersionReference, role: &str) -> Option<String> {
    let hub = reference.model.hub.as_ref()?;
    if reference.model.revision.len() != 40
        || !reference
            .model
            .revision
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    Some(format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        hub.repository,
        reference.model.revision,
        if role == "weights" {
            &hub.weights
        } else {
            &hub.projector
        }
    ))
}
pub fn preview_restore(path: &Path, settings: &Settings, c: &Control) -> Result<RestorePlan> {
    let lock = read(path, c)?;
    let store = Store::open(&settings.shared_model_store)?;
    let mut missing = vec![];
    let mut seen = BTreeSet::new();
    for model in &lock.models {
        for (role, object) in [("weights", &model.weights), ("projector", &model.projector)] {
            if store.resolve(object, Kind::Gguf, c)?.is_none() && seen.insert(object.sha256.clone())
            {
                missing.push(MissingObject {
                    object: object.clone(),
                    kind: Kind::Gguf,
                    model_id: Some(model.model.id.clone()),
                    role: role.into(),
                    publisher_url: model_url(model, role),
                });
            }
        }
    }
    if let Some(pin) = &lock.runtime {
        for file in &pin.identity.files {
            let object = runtime_object(file);
            if store.resolve(&object, Kind::RuntimeFile, c)?.is_none()
                && seen.insert(object.sha256.clone())
            {
                missing.push(MissingObject {
                    object,
                    kind: Kind::RuntimeFile,
                    model_id: None,
                    role: file.name.clone(),
                    publisher_url: None,
                });
            }
        }
    }
    let missing_object_bytes = missing.iter().map(|m| m.object.bytes).sum();
    let runtime_materialization_needed = lock
        .runtime
        .as_ref()
        .map(|pin| cached_runtime(&store, pin, c).map(|path| path.is_none()))
        .transpose()?
        .unwrap_or(false);
    let additional_disk_bytes = missing_object_bytes
        + if runtime_materialization_needed {
            lock.runtime.as_ref().map_or(0, |pin| {
                pin.identity.files.iter().map(|file| file.bytes).sum()
            })
        } else {
            0
        };
    let mut download_bytes = missing
        .iter()
        .filter(|item| item.publisher_url.is_some())
        .map(|item| item.object.bytes)
        .sum::<u64>();
    let mut archive_bytes = 0;
    let needs_runtime_objects = missing.iter().any(|item| item.kind == Kind::RuntimeFile);
    if needs_runtime_objects {
        if let Some(source) = lock.runtime.as_ref().and_then(|pin| pin.source.as_ref()) {
            for asset in &source.assets {
                let id = ObjectId {
                    sha256: asset.sha256.clone(),
                    bytes: asset.bytes,
                };
                if store.resolve(&id, Kind::Archive, c)?.is_none() {
                    archive_bytes += asset.bytes;
                }
            }
        }
    }
    download_bytes += archive_bytes;
    let additional_disk_bytes = additional_disk_bytes
        + archive_bytes
        + if needs_runtime_objects
            && lock
                .runtime
                .as_ref()
                .is_some_and(|pin| pin.source.is_some())
        {
            4_000_000_000
        } else {
            0
        };
    let lock_path = path.canonicalize()?;
    let token = fingerprint(&(
        1,
        &lock.id,
        &lock_path,
        &store.root,
        &missing,
        runtime_materialization_needed,
        additional_disk_bytes,
        download_bytes,
    ))?;
    Ok(RestorePlan {
        lock_path,
        lock_id: lock.id,
        missing,
        missing_object_bytes,
        runtime_materialization_needed,
        additional_disk_bytes,
        download_bytes,
        token,
        note: LIMITS.into(),
    })
}
pub fn restore(
    plan: &RestorePlan,
    token: &str,
    settings: &Settings,
    c: &Control,
    log: &Activity,
) -> Result<RestoreReport> {
    let fresh = preview_restore(&plan.lock_path, settings, c)?;
    anyhow::ensure!(
        digest(token)
            && token == plan.token
            && token == fresh.token
            && fingerprint(plan)? == fingerprint(&fresh)?,
        fault("Dependency restore preview changed; refresh its exact file list")
    );
    let store = Store::open(&settings.shared_model_store)?;
    let mut restored_objects = 0;
    for missing in &fresh.missing {
        if let Some(url) = &missing.publisher_url {
            let asset = crate::models::Asset {
                file: format!("{}.gguf", missing.role),
                bytes: missing.object.bytes,
                sha256: missing.object.sha256.clone(),
            };
            store.download_object(url, &asset, missing.kind, c, log)?;
            restored_objects += 1;
        }
    }
    let lock = read(&plan.lock_path, c)?;
    if let Some(pin) = &lock.runtime {
        for file in &pin.identity.files {
            let id = runtime_object(file);
            if store.resolve(&id, Kind::RuntimeFile, c)?.is_some() {
                continue;
            }
            let mut candidates = vec![
                settings
                    .server
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(&file.name),
                crate::setup::runtime_root(settings).join(&file.name),
            ];
            if let Ok(exe) = std::env::current_exe() {
                if let Some(folder) = exe.parent() {
                    candidates.push(folder.join("runtime").join(&file.name));
                }
            }
            for path in candidates {
                if path.is_file()
                    && crate::model_store::inspect(&path, id.bytes, Kind::RuntimeFile, c)
                        .is_ok_and(|candidate| candidate == id)
                {
                    store.ingest(&path, &id, Kind::RuntimeFile, c)?;
                    restored_objects += 1;
                    break;
                }
            }
        }
        if pin.identity.files.iter().any(|file| {
            store
                .resolve(&runtime_object(file), Kind::RuntimeFile, c)
                .is_ok_and(|path| path.is_none())
        }) {
            if let Some(source) = &pin.source {
                for asset in &source.assets {
                    c.check()?;
                    let (_, archive) = store.download_object(
                        &asset.url,
                        &crate::models::Asset {
                            file: asset.file.clone(),
                            bytes: asset.bytes,
                            sha256: asset.sha256.clone(),
                        },
                        Kind::Archive,
                        c,
                        log,
                    )?;
                    let stage = tempfile::Builder::new()
                        .prefix("runtime-restore-")
                        .tempdir_in(store.root.join("incoming"))?;
                    crate::setup::extract_runtime(&archive, stage.path(), c)?;
                    for file in &pin.identity.files {
                        let id = runtime_object(file);
                        if store.resolve(&id, Kind::RuntimeFile, c)?.is_some() {
                            continue;
                        }
                        let path = stage.path().join(&file.name);
                        if path.is_file()
                            && crate::model_store::inspect(&path, id.bytes, Kind::RuntimeFile, c)
                                .is_ok_and(|candidate| candidate == id)
                        {
                            store.ingest(&path, &id, Kind::RuntimeFile, c)?;
                            restored_objects += 1;
                        }
                    }
                }
            }
        }
        if pin.identity.files.iter().all(|f| {
            store
                .resolve(&runtime_object(f), Kind::RuntimeFile, c)
                .is_ok_and(|p| p.is_some())
        }) {
            materialize_runtime(&store, pin, c)?;
        }
    }
    let remaining_manual = preview_restore(&plan.lock_path, settings, c)?.missing;
    Ok(RestoreReport {
        restored_objects,
        remaining_manual,
        note: LIMITS.into(),
    })
}
/// Self-service exact-file import for custom/offline model and native dependencies.
pub fn import_missing(
    plan: &RestorePlan,
    token: &str,
    index: usize,
    source: &Path,
    settings: &Settings,
    c: &Control,
) -> Result<RestoreReport> {
    let fresh = preview_restore(&plan.lock_path, settings, c)?;
    anyhow::ensure!(
        token == plan.token
            && token == fresh.token
            && digest(token)
            && fingerprint(plan)? == fingerprint(&fresh)?,
        fault("Dependency import preview is stale")
    );
    let missing = fresh
        .missing
        .get(index)
        .ok_or_else(|| fault("Choose an existing missing object"))?;
    let store = Store::open(&settings.shared_model_store)?;
    store.ingest(source, &missing.object, missing.kind, c)?;
    let lock = read(&plan.lock_path, c)?;
    if let Some(pin) = &lock.runtime {
        if pin.identity.files.iter().all(|f| {
            store
                .resolve(&runtime_object(f), Kind::RuntimeFile, c)
                .is_ok_and(|p| p.is_some())
        }) {
            materialize_runtime(&store, pin, c)?;
        }
    }
    Ok(RestoreReport {
        restored_objects: 1,
        remaining_manual: preview_restore(&plan.lock_path, settings, c)?.missing,
        note: LIMITS.into(),
    })
}
