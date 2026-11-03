//! In-app local AI installation. Only pinned files are executed; active versions survive failure.
use crate::{
    hardware::{self, Hardware},
    models::{self, Asset, Update},
    planning::{self, Model},
    progress::{Phase, Work},
    storage::{self, Activity, Control, fingerprint},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Download {
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
    pub url: String,
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
    pub output_preset: Option<Preset>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeRecord {
    pub server: PathBuf,
    pub backend: Backend,
    pub build: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeCandidateRecord {
    schema: u32,
    id: String,
    root: PathBuf,
    record: RuntimeRecord,
    identity: crate::canary::RuntimeIdentity,
    source: Option<crate::project_lock::RuntimeSource>,
    active: Option<crate::activation::Snapshot>,
    previous: Option<crate::activation::Snapshot>,
    reused: bool,
}
pub struct PreparedRuntime {
    candidate: RuntimeCandidateRecord,
    path: Option<PathBuf>,
}
impl PreparedRuntime {
    pub fn record(&self) -> &RuntimeRecord {
        &self.candidate.record
    }
    pub fn pending_record(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    fn changes(&self) -> Result<Vec<crate::activation::Change>> {
        if self.candidate.reused {
            return Ok(vec![]);
        }
        let mut changes = vec![];
        if let Some(active) = &self.candidate.active {
            changes.push(crate::activation::Change {
                scope: crate::activation::Scope::Runtime,
                file: "previous-ai-runtime.json".into(),
                expected: self.candidate.previous.clone(),
                content: active.content()?,
            });
        }
        changes.push(crate::activation::Change {
            scope: crate::activation::Scope::Runtime,
            file: "active-ai-runtime.json".into(),
            expected: self.candidate.active.clone(),
            content: serde_json::to_vec_pretty(&self.candidate.record)?,
        });
        Ok(changes)
    }
    fn verify(&self, c: &Control) -> Result<()> {
        anyhow::ensure!(
            self.candidate.schema == 1
                && self.candidate.id
                    == fingerprint(&(
                        1,
                        &self.candidate.root,
                        &self.candidate.record,
                        &self.candidate.identity,
                        &self.candidate.source,
                        &self.candidate.active,
                        &self.candidate.previous,
                        self.candidate.reused
                    ))?
                && crate::canary::runtime_identity(&self.candidate.record.server, c)?
                    == self.candidate.identity,
            Fault::new("NG660", "Prepared runtime changed before canary/activation")
        );
        Ok(())
    }
}
fn runtime_candidate(
    root: PathBuf,
    record: RuntimeRecord,
    source: Option<crate::project_lock::RuntimeSource>,
    active: Option<crate::activation::Snapshot>,
    previous: Option<crate::activation::Snapshot>,
    reused: bool,
    c: &Control,
) -> Result<PreparedRuntime> {
    let identity = crate::canary::runtime_identity(&record.server, c)?;
    let mut candidate = RuntimeCandidateRecord {
        schema: 1,
        id: String::new(),
        root,
        record,
        identity,
        source,
        active,
        previous,
        reused,
    };
    candidate.id = fingerprint(&(
        1,
        &candidate.root,
        &candidate.record,
        &candidate.identity,
        &candidate.source,
        &candidate.active,
        &candidate.previous,
        candidate.reused,
    ))?;
    anyhow::ensure!(
        serde_json::to_vec_pretty(&candidate)?.len() <= 2_000_000,
        Fault::new("NG660", "Prepared runtime metadata exceeds its bound")
    );
    let path = if reused {
        None
    } else {
        let folder = candidate.root.join("candidates");
        crate::watch::no_links(&folder)?;
        fs::create_dir_all(&folder)?;
        let path = folder.join(format!("{}.runtime.json", candidate.id));
        if !path.exists() {
            storage::protected_json(&path, &candidate)?;
        }
        Some(path)
    };
    Ok(PreparedRuntime { candidate, path })
}
pub fn read_prepared_runtime(path: &Path, s: &Settings, c: &Control) -> Result<PreparedRuntime> {
    c.check()?;
    crate::watch::no_links(path)?;
    let candidate: RuntimeCandidateRecord = storage::read_json(path, 2_000_000)?;
    anyhow::ensure!(
        candidate.root == runtime_root(s).canonicalize()?
            && path.canonicalize()?
                == candidate
                    .root
                    .join("candidates")
                    .join(format!("{}.runtime.json", candidate.id)),
        Fault::new("NG660", "Prepared runtime belongs to another installation")
    );
    let prepared = PreparedRuntime {
        candidate,
        path: Some(path.canonicalize()?),
    };
    prepared.verify(c)?;
    Ok(prepared)
}
/// Find the exact retained candidate selected by these settings. This does not
/// activate it or accept saved reports as proof; subsequent setup runs a canary.
pub fn pending_runtime(s: &Settings, c: &Control) -> Result<Option<PreparedRuntime>> {
    c.check()?;
    let root = runtime_root(s);
    crate::watch::no_links(&root)?;
    if !root.is_dir() || !s.server.is_file() {
        return Ok(None);
    }
    let server = s.server.canonicalize()?;
    if let Some(active) = saved_runtime(&root)
        && active
            .server
            .canonicalize()
            .is_ok_and(|path| path == server)
    {
        return Ok(None);
    }
    let folder = root.join("candidates");
    crate::watch::no_links(&folder)?;
    if !folder.exists() {
        return Ok(None);
    }
    let mut found = None;
    for (index, entry) in fs::read_dir(folder)?.enumerate() {
        c.check()?;
        anyhow::ensure!(
            index < 1000,
            Fault::new("NG660", "Too many pending runtime records")
        );
        let path = entry?.path();
        if !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".runtime.json"))
        {
            continue;
        }
        crate::watch::no_links(&path)?;
        let candidate: RuntimeCandidateRecord = storage::read_json(&path, 2_000_000)?;
        if candidate.record.server == s.server && candidate.record.backend == s.backend {
            anyhow::ensure!(
                found.is_none(),
                Fault::new(
                    "NG660",
                    "Multiple pending runtime records match; select one candidate explicitly"
                )
            );
            found = Some(read_prepared_runtime(&path, s, c)?);
        }
    }
    Ok(found)
}
pub fn saved_runtime(root: &Path) -> Option<RuntimeRecord> {
    let record: RuntimeRecord =
        storage::read_json(&root.join("active-ai-runtime.json"), 100_000).ok()?;
    record.server.is_file().then_some(record)
}
fn initial_work(hw: &Hardware, s: &Settings, options: &Options) -> Result<Vec<Work>> {
    if options.runtime_only {
        return Ok(vec![]);
    }
    // Reserve the model work before the runtime starts. Otherwise a quick runtime
    // install could make the master bar appear nearly finished before multi-GB downloads.
    let mut preview = s.clone();
    preview.require_acceleration = false;
    let models = if options.model_ids.is_empty() {
        recommend(hw, &preview)?.models
    } else {
        options
            .model_ids
            .iter()
            .map(|id| pinned(id).map(|v| v.0))
            .collect::<Result<Vec<_>>>()?
    };
    let mut bytes = 0u64;
    for m in &models {
        let (_, u) = pinned(&m.id)?;
        bytes += u.weights.bytes + u.projector.bytes;
    }
    Ok(vec![
        Work::new(Phase::Download, 0, bytes.div_ceil(1024) as usize, 2, false),
        Work::new(Phase::Activate, 0, models.len(), 1500, false),
    ])
}

/// Stage and validate before changing metadata. Production callers should use
/// install_to so the returned settings participate in the same recovery journal.
pub fn install(s: &Settings, options: &Options, c: &Control, log: &Activity) -> Result<Settings> {
    install_inner(s, options, None, c, log)
}
pub fn install_to(
    s: &Settings,
    options: &Options,
    settings_output: &Path,
    c: &Control,
    log: &Activity,
) -> Result<Settings> {
    install_inner(s, options, Some(settings_output), c, log)
}
fn registry_dependency(
    s: &Settings,
    model: &Model,
    c: &Control,
) -> Result<crate::activation::Change> {
    let file = format!("{}.notegen.json", model.id);
    let path = s.models_dir.join(&file);
    let before = crate::activation::snapshot(&path, c)?.ok_or_else(|| {
        Fault::new(
            "NG660",
            "Registered reader disappeared during runtime preparation",
        )
    })?;
    let current: Model = serde_json::from_slice(&before.content()?)?;
    // Match inventory's path resolution before trusting this newer snapshot.
    // A claimed pair hash alone cannot bind a descriptor to the files probed.
    let resolved = |path: &Path| {
        if path.is_relative() {
            s.models_dir.join(path)
        } else {
            path.to_path_buf()
        }
    };
    anyhow::ensure!(
        current.id == model.id
            && current.sha256 == model.sha256
            && resolved(&current.weights) == model.weights
            && resolved(&current.projector) == model.projector
            && current.prompt == model.prompt
            && current.family == model.family
            && current.role == model.role
            && current.endpoint.is_none()
            && (current.revision.is_empty() || current.revision == model.revision),
        Fault::new(
            "NG660",
            "Registered reader changed during runtime preparation"
        )
    );
    Ok(crate::activation::Change {
        scope: crate::activation::Scope::Models,
        file,
        content: before.content()?,
        expected: Some(before),
    })
}
fn installed_local(s: &Settings) -> Result<Vec<Model>> {
    let (models, _) = crate::runtime::inventory_available(&s.models_dir)?;
    Ok(models
        .into_iter()
        .filter(|model| model.endpoint.is_none())
        .collect())
}
pub(crate) fn validate_pairs(
    models: &[Model],
    s: &Settings,
    evidence: &Path,
    c: &Control,
    log: &Activity,
) -> Result<Vec<crate::canary::Passed>> {
    anyhow::ensure!(
        models.len() <= 64,
        Fault::new(
            "NG660",
            "Too many local readers for a bounded activation; reduce the active registry before updating the runtime"
        )
    );
    let mut passed = vec![];
    for (index, model) in models.iter().enumerate() {
        c.check()?;
        log.work(
            Work::new(Phase::Activate, index, models.len(), 60000, true),
            format!("Canary validation: {}", model.name),
        );
        passed.push(crate::canary::run(
            model,
            s,
            &evidence.join(&model.id),
            c,
            log,
        )?);
    }
    for (model, proof) in models.iter().zip(&passed) {
        proof.verify_current(model, &s.server, c)?;
    }
    Ok(passed)
}
pub(crate) fn activate_with_pending_runtime(
    prepared: &models::PreparedModel,
    s: &Settings,
    c: &Control,
    log: &Activity,
) -> Result<Option<Model>> {
    let Some(runtime) = pending_runtime(s, c)? else {
        return Ok(None);
    };
    let mut probes = vec![prepared.model().clone()];
    let mut dependencies = vec![];
    for model in installed_local(s)? {
        if model.id != prepared.model().id {
            dependencies.push(registry_dependency(s, &model, c)?);
            probes.push(model);
        }
    }
    let proofs = validate_pairs(
        &probes,
        s,
        &s.models_dir.join("activation-canaries"),
        c,
        log,
    )?;
    runtime.verify(c)?;
    for (model, proof) in probes.iter().zip(&proofs) {
        proof.verify_current(model, &s.server, c)?;
    }
    let mut changes = dependencies;
    changes.extend(prepared.changes(c)?);
    changes.extend(runtime.changes()?);
    crate::activation::commit(&s.models_dir, &runtime.candidate.root, None, changes, c)?;
    log.emit("models", "Reader and pending runtime activated after the known print/table canary; real documents still require review", 0, 1, 1, "");
    Ok(Some(prepared.model().clone()))
}
fn install_inner(
    s: &Settings,
    options: &Options,
    settings_output: Option<&Path>,
    c: &Control,
    log: &Activity,
) -> Result<Settings> {
    anyhow::ensure!(
        !(options.models_only && options.runtime_only),
        Fault::new("NG650", "Choose runtime-only or models-only")
    );
    s.validate()?;
    c.check()?;
    crate::activation::recover_all(&s.models_dir, c)?;
    let output_path = settings_output.map(std::path::absolute).transpose()?;
    if let Some(path) = &output_path {
        crate::watch::no_links(path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
    }
    let settings_before = output_path
        .as_ref()
        .map(|path| crate::activation::snapshot(path, c))
        .transpose()?
        .flatten();
    for work in initial_work(&hardware::detect(), s, options)? {
        log.work(
            work,
            "Planning staged installation and real compatibility validation",
        );
    }
    let existing = installed_local(s)?;
    let runtime = if options.models_only {
        pending_runtime(s, c)?
    } else {
        let assets: Vec<Download> =
            serde_json::from_str(include_str!("../runtime-downloads.json"))?;
        Some(prepare_runtime_release(s, "b12032", &assets, c, log)?)
    };
    let mut next = s.clone();
    if let Some(runtime) = &runtime {
        runtime.verify(c)?;
        next.server = runtime.record().server.clone();
        next.backend = runtime.record().backend;
        next.external_server = false;
    }
    if let Some(preset) = options.output_preset {
        next.preset = preset;
    }
    let runtime_changed = runtime.as_ref().is_some_and(|runtime| {
        !runtime.candidate.reused
            || runtime.record().server != s.server
            || runtime.record().backend != s.backend
    });
    let mut prepared = vec![];
    if !options.runtime_only {
        let chosen = if options.model_ids.is_empty() {
            recommend(&hardware::detect_for(&next, c), &next)?.models
        } else {
            options
                .model_ids
                .iter()
                .map(|id| pinned(id).map(|pair| pair.0))
                .collect::<Result<Vec<_>>>()?
        };
        anyhow::ensure!(
            !chosen.is_empty()
                && chosen.len() <= 64
                && chosen
                    .iter()
                    .map(|model| &model.id)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == chosen.len(),
            Fault::new("NG650", "Select between one and 64 distinct local readers")
        );
        for model in chosen {
            c.check()?;
            let healthy =
                if let Some(current) = existing.iter().find(|current| current.id == model.id) {
                    !current.sha256.is_empty()
                        && storage::fingerprint(&(
                            storage::hash_file(&current.weights, c)?,
                            storage::hash_file(&current.projector, c)?,
                        ))? == current.sha256
                } else {
                    false
                };
            if healthy {
                log.emit(
                    "models",
                    format!(
                        "{} retained without downgrade; its registered bytes are unchanged",
                        model.name
                    ),
                    0,
                    0,
                    0,
                    "",
                );
            } else {
                let (model, update) = pinned(&model.id)?;
                prepared.push(models::prepare(&next, &model, &update, c, log)?);
            }
        }
    }
    let mut probes = prepared
        .iter()
        .map(|candidate| candidate.model().clone())
        .collect::<Vec<_>>();
    let mut dependencies = vec![];
    if runtime_changed {
        for model in &existing {
            if !probes.iter().any(|candidate| candidate.id == model.id) {
                dependencies.push(registry_dependency(s, model, c)?);
                probes.push(model.clone());
            }
        }
        if probes.is_empty()
            && options.runtime_only
            && runtime.as_ref().is_some_and(|runtime| {
                runtime.candidate.active.is_none() && !runtime.candidate.reused
            })
        {
            // Explicit first-install staging supports runtime-only -> models-only
            // without executing an untested model or publishing a working runtime.
            if let Some(path) = &output_path {
                crate::activation::commit(
                    &s.models_dir,
                    &runtime.as_ref().unwrap().candidate.root,
                    Some(path),
                    vec![crate::activation::Change {
                        scope: crate::activation::Scope::Settings,
                        file: path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .ok_or_else(|| {
                                Fault::new("NG660", "Choose a Unicode settings filename")
                            })?
                            .into(),
                        expected: settings_before,
                        content: serde_json::to_vec_pretty(&next)?,
                    }],
                    c,
                )?;
            }
            log.work(
                Work::new(Phase::Activate, 1, 1, 1500, false),
                "Runtime staged only; complete model setup and the real canary before readiness",
            );
            return Ok(next);
        }
        anyhow::ensure!(
            !probes.is_empty(),
            Fault::new(
                "NG660",
                format!(
                    "Runtime is staged but pending a local model canary. Complete AI setup with a reader before activation. Candidate: {}",
                    runtime
                        .as_ref()
                        .and_then(|r| r.pending_record())
                        .map_or_else(
                            || "user-selected runtime".into(),
                            |p| p.display().to_string()
                        )
                )
            )
        );
    }
    let evidence = s.models_dir.join("activation-canaries");
    let proofs = validate_pairs(&probes, &next, &evidence, c, log)?;
    if let Some(runtime) = &runtime {
        runtime.verify(c)?;
    }
    for (model, proof) in probes.iter().zip(&proofs) {
        proof.verify_current(model, &next.server, c)?;
    }
    let mut changes = dependencies;
    if let Some(runtime) = &runtime {
        changes.extend(runtime.changes()?);
    }
    for candidate in &prepared {
        changes.extend(candidate.changes(c)?);
    }
    if let Some(path) = &output_path {
        changes.push(crate::activation::Change {
            scope: crate::activation::Scope::Settings,
            file: path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| Fault::new("NG660", "Choose a Unicode settings output filename"))?
                .into(),
            expected: settings_before,
            content: serde_json::to_vec_pretty(&next)?,
        });
    }
    if !changes.is_empty() {
        crate::activation::commit(
            &s.models_dir,
            &runtime
                .as_ref()
                .map_or_else(|| runtime_root(s), |r| r.candidate.root.clone()),
            output_path.as_deref(),
            changes,
            c,
        )?;
    }
    log.work(
        Work::new(Phase::Activate, 1, 1, 1500, false),
        "Validated installation metadata committed; real documents still require review",
    );
    Ok(next)
}
fn runtime_downloads(backend: &str, assets: &[Download]) -> Vec<Download> {
    let marker = match backend {
        "cuda13" => "cuda-13.4",
        "cuda12" => "cuda-12.4",
        "vulkan" => "vulkan",
        _ => "cpu",
    };
    assets
        .iter()
        .filter(|a| a.file.contains(&format!("win-{marker}-x64")))
        .cloned()
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
pub fn install_runtime_release(
    s: &Settings,
    build: &str,
    assets: &[Download],
    c: &Control,
    log: &Activity,
) -> Result<RuntimeRecord> {
    install_runtime_release_inner(s, build, assets, None, c, log)
}
pub fn install_runtime_release_to(
    s: &Settings,
    build: &str,
    assets: &[Download],
    settings_output: &Path,
    c: &Control,
    log: &Activity,
) -> Result<RuntimeRecord> {
    install_runtime_release_inner(s, build, assets, Some(settings_output), c, log)
}
fn install_runtime_release_inner(
    s: &Settings,
    build: &str,
    assets: &[Download],
    settings_output: Option<&Path>,
    c: &Control,
    log: &Activity,
) -> Result<RuntimeRecord> {
    crate::activation::recover_all(&s.models_dir, c)?;
    let output = settings_output.map(std::path::absolute).transpose()?;
    let settings_before = output
        .as_ref()
        .map(|path| crate::activation::snapshot(path, c))
        .transpose()?
        .flatten();
    let models = installed_local(s)?;
    let dependencies = models
        .iter()
        .map(|model| registry_dependency(s, model, c))
        .collect::<Result<Vec<_>>>()?;
    let prepared = prepare_runtime_release(s, build, assets, c, log)?;
    if prepared.candidate.reused
        && prepared.record().server == s.server
        && prepared.record().backend == s.backend
        && !s.external_server
    {
        return Ok(prepared.record().clone());
    }
    anyhow::ensure!(
        !models.is_empty(),
        Fault::new(
            "NG660",
            format!(
                "Runtime is staged but pending a real local model canary; complete AI setup with a reader. Candidate: {}",
                prepared.pending_record().map_or_else(
                    || "user-selected runtime".into(),
                    |path| path.display().to_string()
                )
            )
        )
    );
    let mut next = s.clone();
    next.server = prepared.record().server.clone();
    next.backend = prepared.record().backend;
    next.external_server = false;
    let proofs = validate_pairs(
        &models,
        &next,
        &s.models_dir.join("activation-canaries"),
        c,
        log,
    )?;
    prepared.verify(c)?;
    for (model, proof) in models.iter().zip(&proofs) {
        proof.verify_current(model, &next.server, c)?;
    }
    let mut changes = dependencies;
    changes.extend(prepared.changes()?);
    if let Some(output) = &output {
        changes.push(crate::activation::Change {
            scope: crate::activation::Scope::Settings,
            file: output
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| Fault::new("NG660", "Choose a Unicode settings output filename"))?
                .into(),
            expected: settings_before,
            content: serde_json::to_vec_pretty(&next)?,
        });
    }
    if !changes.is_empty() {
        crate::activation::commit(
            &s.models_dir,
            &prepared.candidate.root,
            output.as_deref(),
            changes,
            c,
        )?;
    }
    Ok(prepared.record().clone())
}
/// Validate a previously installed native runtime against every active local
/// reader before restoring its pointer and the selected settings file together.
pub fn rollback_runtime_to(
    s: &Settings,
    settings_output: &Path,
    c: &Control,
    log: &Activity,
) -> Result<Settings> {
    c.check()?;
    crate::activation::recover_all(&s.models_dir, c)?;
    let root = runtime_root(s);
    crate::watch::no_links(&root)?;
    let root = root.canonicalize()?;
    let old_active = crate::activation::snapshot(&root.join("active-ai-runtime.json"), c)?;
    let old_previous = crate::activation::snapshot(&root.join("previous-ai-runtime.json"), c)?
        .ok_or_else(|| Fault::new("NG660", "No previous managed runtime is available"))?;
    let previous: RuntimeRecord = serde_json::from_slice(&old_previous.content()?)?;
    crate::watch::no_links(&previous.server)?;
    let server = previous.server.canonicalize()?;
    anyhow::ensure!(
        server.starts_with(root.join("ai-backends"))
            && server
                .file_name()
                .is_some_and(|name| name == "llama-server.exe"),
        Fault::new("NG660", "Previous runtime is outside the managed folder")
    );
    let output = std::path::absolute(settings_output)?;
    let settings_before = crate::activation::snapshot(&output, c)?;
    let models = installed_local(s)?;
    anyhow::ensure!(
        !models.is_empty(),
        Fault::new(
            "NG660",
            "Previous runtime remains pending until a local reader canary is available"
        )
    );
    let dependencies = models
        .iter()
        .map(|model| registry_dependency(s, model, c))
        .collect::<Result<Vec<_>>>()?;
    let prepared = runtime_candidate(
        root.clone(),
        previous,
        None,
        old_active,
        Some(old_previous),
        false,
        c,
    )?;
    let mut next = s.clone();
    next.server = server;
    next.backend = prepared.record().backend;
    next.external_server = false;
    let proofs = validate_pairs(
        &models,
        &next,
        &s.models_dir.join("activation-canaries"),
        c,
        log,
    )?;
    prepared.verify(c)?;
    for (model, proof) in models.iter().zip(&proofs) {
        proof.verify_current(model, &next.server, c)?;
    }
    let mut changes = dependencies;
    changes.extend(prepared.changes()?);
    changes.push(crate::activation::Change {
        scope: crate::activation::Scope::Settings,
        file: output
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Fault::new("NG660", "Choose a Unicode settings output filename"))?
            .into(),
        expected: settings_before,
        content: serde_json::to_vec_pretty(&next)?,
    });
    crate::activation::commit(&s.models_dir, &root, Some(&output), changes, c)?;
    Ok(next)
}
pub fn runtime_root(s: &Settings) -> PathBuf {
    let parent = s
        .server
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("runtime"));
    parent
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == "ai-backends"))
        .and_then(Path::parent)
        .unwrap_or(parent)
        .to_path_buf()
}
pub fn prepare_runtime_release(
    s: &Settings,
    build: &str,
    assets: &[Download],
    c: &Control,
    log: &Activity,
) -> Result<PreparedRuntime> {
    crate::maintenance::validate_runtime_release(build, assets)?;
    let root = runtime_root(s);
    crate::watch::no_links(&root)?;
    fs::create_dir_all(&root)?;
    let root = root.canonicalize()?;
    let active_snapshot = crate::activation::snapshot(&root.join("active-ai-runtime.json"), c)?;
    let previous_snapshot = crate::activation::snapshot(&root.join("previous-ai-runtime.json"), c)?;
    if !cfg!(windows) {
        anyhow::ensure!(
            s.server.is_file(),
            Fault::new(
                "NG650",
                "Automatic runtime installation supports Windows x64. Select a native llama-server or use models-only."
            )
        );
        return runtime_candidate(
            root,
            RuntimeRecord {
                server: s.server.clone(),
                backend: s.backend,
                build: "user-selected".into(),
            },
            None,
            active_snapshot,
            previous_snapshot,
            true,
            c,
        );
    }
    if let Some(active) = saved_runtime(&root)
        && active
            .build
            .trim_start_matches('b')
            .parse::<u64>()
            .unwrap_or(0)
            >= build.trim_start_matches('b').parse::<u64>().unwrap_or(0)
        && (!s.require_acceleration || active.backend != Backend::Cpu)
        && (s.backend == Backend::Auto || active.backend == s.backend)
        && crate::scan::run(
            &active.server,
            &["--list-devices".into()],
            Duration::from_secs(30),
            c,
        )
        .is_ok_and(|r| r.0)
    {
        log.emit(
            "runtime",
            format!("{build} runtime already installed and starts correctly"),
            0,
            0,
            0,
            "",
        );
        return runtime_candidate(
            root,
            active,
            None,
            active_snapshot,
            previous_snapshot,
            true,
            c,
        );
    }
    let candidates = runtime_choices(
        s.backend,
        !hardware::detect().gpus.is_empty(),
        s.require_acceleration,
    );
    let mut failures = vec![];
    for backend in candidates {
        c.check()?;
        let result = (|| -> Result<PreparedRuntime> {
            let cache = root.join("downloads");
            fs::create_dir_all(&cache)?;
            let backends = root.join("ai-backends");
            fs::create_dir_all(&backends)?;
            let staging = tempfile::Builder::new()
                .prefix(&format!("{backend}-{build}-"))
                .tempdir_in(&backends)?;
            log.emit(
                "runtime",
                format!("Installing and verifying {backend} runtime"),
                0,
                0,
                0,
                "",
            );
            let assets = runtime_downloads(backend, assets);
            anyhow::ensure!(
                !assets.is_empty(),
                Fault::new("NG650", "Runtime package metadata missing")
            );
            for a in &assets {
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
                build: build.into(),
            };
            let source = crate::project_lock::RuntimeSource {
                build: build.into(),
                backend: selected,
                assets,
            };
            storage::protected_json(&folder.join("notegen-runtime-source.json"), &source)?;
            runtime_candidate(
                root.clone(),
                record,
                Some(source),
                active_snapshot.clone(),
                previous_snapshot.clone(),
                false,
                c,
            )
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

pub(crate) fn extract_runtime(archive: &Path, dir: &Path, c: &Control) -> Result<()> {
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
    fn runtime_dependency_rejects_changed_paths_even_when_bytes_and_claimed_hash_match() {
        for field in ["weights", "projector"] {
            let root = tempfile::tempdir().unwrap();
            let registry = root.path().join("models");
            fs::create_dir(&registry).unwrap();
            let (mut model, _) = pinned("glm-ocr").unwrap();
            model.weights = registry.join("weights.gguf");
            model.projector = registry.join("projector.gguf");
            fs::write(&model.weights, b"GGUForiginal weights").unwrap();
            fs::write(&model.projector, b"GGUForiginal projector").unwrap();
            models::save_custom(&registry, model, &Control::default()).unwrap();
            let descriptor = registry.join("glm-ocr.notegen.json");
            let mut value: serde_json::Value = storage::read_json(&descriptor, 100_000).unwrap();
            // Both unchanged relative paths must resolve exactly as inventory does.
            value["weights"] = "weights.gguf".into();
            value["projector"] = "projector.gguf".into();
            fs::write(&descriptor, serde_json::to_vec(&value).unwrap()).unwrap();
            let s = Settings {
                models_dir: registry.clone(),
                ..Default::default()
            };
            let captured = installed_local(&s).unwrap().remove(0);
            assert!(registry_dependency(&s, &captured, &Control::default()).is_ok());
            let original_hash = value["sha256"].clone();
            let replacement = registry.join(format!("replacement-{field}.gguf"));
            let original = if field == "weights" {
                &captured.weights
            } else {
                &captured.projector
            };
            fs::copy(original, &replacement).unwrap();
            // Exercise relative and absolute replacements without changing any
            // other descriptor field or any model bytes.
            value[field] = if field == "weights" {
                replacement.file_name().unwrap().to_str().unwrap().into()
            } else {
                replacement.to_str().unwrap().into()
            };
            assert_eq!(value["sha256"], original_hash);
            let changed = serde_json::to_vec(&value).unwrap();
            fs::write(&descriptor, &changed).unwrap();
            let error = registry_dependency(&s, &captured, &Control::default())
                .err()
                .expect("changed model paths must invalidate activation dependencies");
            assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG660");
            assert_eq!(fs::read(&descriptor).unwrap(), changed);
            assert!(!registry.join("glm-ocr.previous.json").exists());
            assert!(!registry.join(".notegen-activation").exists());
            assert_eq!(fs::read(original).unwrap(), fs::read(replacement).unwrap());
        }
    }
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
    fn first_install_runtime_candidate_is_discovered_without_an_active_pointer_or_saved_proof() {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("runtime");
        let server = runtime.join("ai-backends/cpu-pending/llama-server.exe");
        fs::create_dir_all(server.parent().unwrap()).unwrap();
        fs::write(&server, b"MZsynthetic-runtime-not-executed").unwrap();
        let s = Settings {
            server: server.clone(),
            backend: Backend::Cpu,
            models_dir: root.path().join("models"),
            shared_model_store: root.path().join("store"),
            ..Default::default()
        };
        let staged = runtime_candidate(
            runtime.canonicalize().unwrap(),
            RuntimeRecord {
                server,
                backend: Backend::Cpu,
                build: "b12032".into(),
            },
            None,
            None,
            None,
            false,
            &Control::default(),
        )
        .unwrap();
        let found = pending_runtime(&s, &Control::default()).unwrap().unwrap();
        assert_eq!(found.pending_record(), staged.pending_record());
        assert!(!runtime.join("active-ai-runtime.json").exists());
        assert!(!s.models_dir.join("glm-ocr.notegen.json").exists());
        let path = staged.pending_record().unwrap();
        let mut altered: serde_json::Value = storage::read_json(path, 2_000_000).unwrap();
        altered["record"]["build"] = "tampered".into();
        fs::write(path, serde_json::to_vec(&altered).unwrap()).unwrap();
        assert!(pending_runtime(&s, &Control::default()).is_err());
        assert!(!runtime.join("active-ai-runtime.json").exists());
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
    fn runtime_completion_does_not_hide_the_remaining_model_downloads() {
        let hw = Hardware {
            ram_mib: 16000,
            free_ram_mib: 14000,
            logical_cores: 8,
            ..Default::default()
        };
        let mut estimate = crate::progress::Estimate::new(0.);
        for w in initial_work(&hw, &Settings::default(), &Options::default()).unwrap() {
            estimate.update(&w, 0.);
        }
        estimate.update(&Work::new(Phase::Runtime, 0, 1, 15000, true), 0.);
        estimate.update(&Work::new(Phase::Runtime, 1, 1, 15000, false), 15.);
        assert!(estimate.snapshot(15.).fraction < 0.05);
        assert!(estimate.snapshot(15.).remaining.unwrap() > 100.);
    }
    #[test]
    fn runtime_fallback_respects_explicit_backend_and_acceleration() {
        assert_eq!(
            runtime_choices(Backend::Cuda, true, true),
            vec!["cuda13", "cuda12"]
        );
        assert_eq!(runtime_choices(Backend::Cpu, true, false), vec!["cpu"]);
        assert_eq!(
            runtime_downloads(
                "cuda13",
                &serde_json::from_str::<Vec<Download>>(include_str!("../runtime-downloads.json"))
                    .unwrap()
            )
            .len(),
            2
        );
        assert_eq!(
            runtime_downloads(
                "cpu",
                &serde_json::from_str::<Vec<Download>>(include_str!("../runtime-downloads.json"))
                    .unwrap()
            )
            .len(),
            1
        );
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
