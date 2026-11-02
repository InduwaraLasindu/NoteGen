use crate::{
    consensus, hardware,
    planning::{self, Plan},
    progress::{Phase, Work},
    runtime::{self, Server},
    scan,
    storage::*,
    types::*,
    word,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
/// Preparation overlap is opt-in because native decoder/OCR allocations are estimates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationMode {
    #[default]
    Sequential,
    BoundedOverlap,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct PreparationAdmission {
    pub requested_mode: PreparationMode,
    pub enabled: bool,
    pub ocr_threads: usize,
    pub required_mib: u64,
    pub available_mib: u64,
    pub reason: String,
}
/// Conservative application admission, not an OS memory quota or native-allocation guarantee.
pub fn preparation_admission(
    mode: PreparationMode,
    pages: usize,
    s: &Settings,
    hw: &hardware::Hardware,
) -> PreparationAdmission {
    let cores = if s.cpu_threads == 0 {
        hw.physical_cores.max(1)
    } else {
        s.cpu_threads
    }
    .min(hw.logical_cores.max(1))
    .min(512);
    // Three source images (rendering, queued, consuming), decode/rotation/deskew,
    // grayscale/OCR/encoder buffers, plus allowance for native OCR and metadata.
    let required_mib = (u64::from(s.max_pixels) * 64).div_ceil(1_048_576) + 512;
    let available_mib = hw
        .free_ram_mib
        .min((hw.ram_mib as f64 * s.ram_fraction) as u64)
        .saturating_sub(s.reserve_ram_mib);
    let reason = if mode == PreparationMode::Sequential {
        "Sequential preparation selected; overlap requires explicit opt-in."
    } else if pages < 2 {
        "Sequential fallback: fewer than two selected pages."
    } else if cores < 2 {
        "Sequential fallback: the configured CPU budget cannot reserve a render core."
    } else if available_mib < required_mib {
        "Sequential fallback: measured free RAM after reservation is below the conservative preparation estimate."
    } else {
        "Bounded preparation admitted: caller-thread rendering, one queued page and one OCR consumer; AI readers start after preparation joins. Memory admission is an estimate, not a hard quota."
    };
    let enabled = mode == PreparationMode::BoundedOverlap
        && pages >= 2
        && cores >= 2
        && available_mib >= required_mib;
    PreparationAdmission {
        requested_mode: mode,
        enabled,
        // Printed OCR/preprocessing does not need hundreds of parked Rayon workers.
        ocr_threads: if enabled { cores - 1 } else { cores }.min(8),
        required_mib,
        available_mib,
        reason: reason.into(),
    }
}

#[derive(Debug)]
struct PreparationStopped;
impl std::fmt::Display for PreparationStopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Preparation consumer stopped")
    }
}
impl std::error::Error for PreparationStopped {}
struct PreparationFinished<'a>(&'a AtomicBool);
impl Drop for PreparationFinished<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
/// The producer always runs on the caller thread. Every helper is scoped and joined.
fn bounded_preparation<T: Send, R>(
    c: &Control,
    producer: impl FnOnce(&Control, &mut dyn FnMut(T) -> Result<()>) -> Result<R>,
    mut consumer: impl FnMut(T, &Control) -> Result<()> + Send,
) -> Result<R> {
    c.check()?;
    let abort = AtomicBool::new(false);
    let finished = AtomicBool::new(false);
    let consumer_done = AtomicBool::new(false);
    let linked = Control {
        cancelled: Arc::new(AtomicBool::new(false)),
        paused: c.paused.clone(),
        clarify: None,
    };
    thread::scope(|scope| {
        let (tx, rx) = mpsc::sync_channel(1);
        let bridge = scope.spawn(|| {
            while !finished.load(Ordering::Acquire) {
                if c.cancelled.load(Ordering::Relaxed)
                    || abort.load(Ordering::Acquire)
                    || consumer_done.load(Ordering::Acquire)
                {
                    linked.cancelled.store(true, Ordering::Relaxed);
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        let worker_control = &linked;
        let worker_done = &consumer_done;
        let worker = scope.spawn(move || -> Result<()> {
            let _finished = PreparationFinished(worker_done);
            loop {
                worker_control.check()?;
                match rx.recv_timeout(Duration::from_millis(10)) {
                    Ok(item) => consumer(item, worker_control)?,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                }
            }
        });
        let produced = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            producer(&linked, &mut |mut item| loop {
                linked.check()?;
                match tx.try_send(item) {
                    Ok(()) => return Ok(()),
                    Err(mpsc::TrySendError::Full(value)) => {
                        item = value;
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => {
                        return Err(PreparationStopped.into());
                    }
                }
            })
        }))
        .unwrap_or_else(|_| {
            Err(Fault::new("NG500", "Source preparation producer stopped unexpectedly").into())
        });
        if produced.is_err() {
            abort.store(true, Ordering::Release);
        }
        drop(tx);
        let consumed = worker.join().unwrap_or_else(|_| {
            Err(Fault::new("NG500", "Source preparation consumer stopped unexpectedly").into())
        });
        finished.store(true, Ordering::Release);
        let _ = bridge.join();
        if c.cancelled.load(Ordering::Relaxed) {
            c.check()?;
        }
        match (produced, consumed) {
            (Err(producer), Err(consumer))
                if producer.is::<PreparationStopped>()
                    || producer
                        .downcast_ref::<Fault>()
                        .is_some_and(|f| f.code == "NG900") =>
            {
                Err(consumer)
            }
            (Err(producer), _) => Err(producer),
            (Ok(_), Err(consumer)) => Err(consumer),
            (Ok(value), Ok(())) => Ok(value),
        }
    })
}
struct PagePreparation<'a> {
    root: &'a Path,
    settings: &'a Settings,
    log: &'a Activity,
    total: usize,
    has_readers: bool,
}
impl PagePreparation<'_> {
    fn run(&self, n: usize, ordinal: usize, im: image::DynamicImage, c: &Control) -> Result<bool> {
        let root = self.root;
        let s = self.settings;
        let log = self.log;
        let pages = self.total;
        let image = root.join(format!("page-{n:05}.png"));
        let checkpoint = root.join(format!("page-{n:05}.json"));
        log.work(
            Work::new(Phase::Render, ordinal - 1, pages, 1500, true),
            "Preparing source pages",
        );
        if image.exists()
            && read_json::<Page>(&checkpoint, 32_000_000)
                .is_ok_and(|p| p.page == n && p.validate().is_ok())
        {
            log.emit(
                "cached",
                "Found rendered page checkpoint",
                n,
                ordinal,
                0,
                "",
            );
            log.work(
                Work::new(Phase::Render, ordinal, pages, 1500, false),
                "Reused rendered checkpoint",
            );
            return Ok(false);
        }
        c.check()?;
        log.emit(
            "render",
            "Preparing source scan and printed OCR",
            n,
            ordinal - 1,
            0,
            "",
        );
        let preflight = crate::preflight::analyze(&im);
        let im = scan::deskew(scan::orient(im, s, c)?, s.deskew, s.max_pixels);
        let content = match scan::printed(&im, s, c) {
            Ok(p) => p,
            Err(e) => {
                if e.downcast_ref::<Fault>().is_some_and(|f| f.code == "NG900") {
                    return Err(e);
                }
                if !self.has_readers {
                    return Err(e);
                }
                log.emit(
                    "ocr",
                    "Printed OCR unavailable; using local AI readers",
                    n,
                    0,
                    0,
                    "NG110",
                );
                PageContent {
                    warnings: vec!["Printed OCR unavailable.".into()],
                    ..Default::default()
                }
            }
        };
        scan::save_scan(&image, &im)?;
        let warning = scan::blur_warning(&im);
        let candidate = Candidate {
            region: None,
            model_id: "tesseract".into(),
            family: "tesseract".into(),
            view: "full".into(),
            reliability: if s.handwriting { 0.25 } else { 0.65 },
            content: content.clone(),
        };
        let mut p = Page {
            page: n,
            image: image.file_name().unwrap().to_string_lossy().into(),
            content,
            candidates: vec![candidate],
            agreement: if s.preset == Preset::Fast { 1. } else { 0. },
            ..Default::default()
        };
        if warning {
            p.content
                .warnings
                .push("Blur/low detail detected. Verify small text against the source.".into());
        }
        for observation in preflight.observations {
            p.content.warnings.push(format!(
                "{} {}",
                observation.message, observation.suggestion
            ));
        }
        p.content
            .warnings
            .extend(crate::preflight::numeric_observations(&p.content));
        p.source_warnings = p.content.warnings.clone();
        atomic_json(&checkpoint, &p)?;
        log.work(
            Work::new(Phase::Render, ordinal, pages, 1500, false),
            "Source page prepared",
        );
        log.emit("ocr", "Printed OCR checkpoint saved", n, ordinal, 0, "");
        Ok(true)
    }
}
#[derive(Clone, Debug)]
pub struct Completed {
    pub project: Project,
    pub output: PathBuf,
    pub pages: usize,
    pub review: usize,
    pub seconds: f64,
    pub plan: Plan,
    pub preparation: PreparationAdmission,
}
pub fn convert(
    source: &Path,
    output: &Path,
    s: &Settings,
    c: &Control,
    activity: &Activity,
) -> Result<Completed> {
    convert_with_preparation(source, output, s, c, activity, PreparationMode::Sequential)
}
pub fn convert_with_preparation(
    source: &Path,
    output: &Path,
    s: &Settings,
    c: &Control,
    activity: &Activity,
    preparation_mode: PreparationMode,
) -> Result<Completed> {
    let begun = Instant::now();
    s.validate()?;
    let language_digest = if s.tessdata_dir.as_os_str().is_empty() {
        String::new()
    } else {
        crate::languages::verify_selection(&s.tessdata_dir, &s.languages, c)?;
        fingerprint(&(s.languages.clone(), include_str!("../languages.json")))?
    };
    let mut effective = s.clone();
    let writer_digest = if s.preset != Preset::Fast && s.writer.use_examples {
        crate::writers::digest(&s.writer.root, s.writer.id.as_deref())?
    } else {
        String::new()
    };
    if s.preset != Preset::Fast
        && s.writer.use_examples
        && let Some(id) = s.writer.id.as_deref()
    {
        effective.writer_reference = crate::writers::reference(&s.writer.root, id)?;
    }
    let s = &effective;
    anyhow::ensure!(
        !output.exists(),
        Fault::new("NG401", "Output already exists")
    );
    crate::inputs::files(source)?;
    let hw = hardware::detect_for(s, c);
    // Printed OCR must not depend on (or scan gigabytes of) unrelated AI assets.
    let installed = if s.preset == Preset::Fast {
        vec![]
    } else {
        let (models, errors) = runtime::inventory_available(&s.models_dir)?;
        for error in errors {
            activity.emit(
                "models",
                format!("Skipped invalid reader: {error}"),
                0,
                0,
                0,
                "NG200",
            );
        }
        models
    };
    let mut plan = planning::build(&hw, s, &installed)?;
    let pages = crate::inputs::selected_page_count(source, &s.pdfium, &s.page_range)?;
    let readers = plan.models.len();
    let read_ms = if plan.backend == Backend::Cpu {
        30000
    } else {
        8000
    };
    let crop_reserved = if s.preset == Preset::Maximum {
        pages * readers * s.crop_retries.min(2)
    } else {
        0
    };
    for (phase, total, ms) in [
        (Phase::Render, pages, 1500),
        (Phase::Load, readers * 2, 5000),
        (Phase::Read, pages * readers, read_ms),
        (Phase::Crop, crop_reserved, read_ms / 2),
        (Phase::Finalize, pages, 100),
        (Phase::Export, 1, 2000),
    ] {
        activity.work(
            Work::new(phase, 0, total, ms, false),
            "Planning document work",
        );
    }
    for (verified, model) in plan.models.iter_mut().enumerate() {
        activity.work(
            Work::new(Phase::Load, verified, readers * 2, 5000, true),
            "Verifying reader files",
        );
        c.check()?;
        if model.endpoint.is_some() {
            continue;
        }
        activity.emit(
            "verify",
            format!("Verifying {} model files", model.name),
            0,
            0,
            0,
            "",
        );
        let digest = fingerprint(&(
            hash_file(&model.weights, c)?,
            hash_file(&model.projector, c)?,
        ))?;
        if !model.sha256.is_empty() {
            anyhow::ensure!(
                model.sha256 == digest,
                Fault::new(
                    "NG200",
                    "Model or projector checksum changed; re-register the files"
                )
            );
        }
        model.sha256 = digest;
        activity.work(
            Work::new(Phase::Load, verified + 1, readers * 2, 5000, false),
            "Reader files verified",
        );
    }
    if s.preset != Preset::Fast && (s.handwriting || s.preset != Preset::Balanced) {
        anyhow::ensure!(
            !plan.models.is_empty(),
            Fault::new(
                "NG200",
                "No installed models fit this PC. Open Advanced → Models to register local GGUFs."
            )
        );
    }
    activity.emit(
        "prepare",
        "Hashing input and planning hardware resources",
        0,
        0,
        0,
        "",
    );
    let input = crate::inputs::hash(source, c).map_err(|e| {
        if e.downcast_ref::<Fault>().is_some() {
            e
        } else {
            Fault::new("NG100", format!("Cannot read input PDF: {e}")).into()
        }
    })?;
    let key = fingerprint(&(
        2,
        recognition_settings(s),
        &writer_digest,
        &language_digest,
        (plan.image_edge, plan.context, plan.dpi, plan.image_tokens),
        &plan
            .models
            .iter()
            .map(|m| {
                (
                    &m.id,
                    &m.weights,
                    &m.projector,
                    fs::metadata(&m.weights).ok().map(|v| v.len()),
                    &m.sha256,
                    &m.endpoint,
                    &m.alias,
                    &m.revision,
                )
            })
            .collect::<Vec<_>>(),
    ))?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let base_root = parent.join(".notegen").join(format!(
        "{}-{}-{}",
        source.file_stem().unwrap_or_default().to_string_lossy(),
        &input[..10],
        &key[..10]
    ));
    let mut root = base_root.clone();
    let mut attempt = 0;
    while root.join("input-invalidated.json").exists() {
        attempt += 1;
        anyhow::ensure!(
            attempt <= 100,
            Fault::new(
                "NG500",
                "Too many invalidated checkpoints; choose a new output folder"
            )
        );
        root = base_root.with_file_name(format!(
            "{}-retry-{attempt}",
            base_root.file_name().unwrap().to_string_lossy()
        ));
    }
    fs::create_dir_all(&root)?;
    let log = Activity {
        path: Some(root.join("activity.jsonl")),
        callback: activity.callback.clone(),
    };
    let admission = preparation_admission(preparation_mode, pages, s, &hardware::detect());
    atomic_json(&root.join("preparation-report.json"), &admission)?;
    log.emit(
        "prepare",
        format!("{} OCR worker budget: {} threads. Estimated buffers/native allowance: {} MiB; measured admission budget after user reserve: {} MiB.", admission.reason, admission.ocr_threads, admission.required_mib, admission.available_mib),
        0, 0, pages, "",
    );
    let mut preparation_settings = s.clone();
    preparation_settings.cpu_threads = admission.ocr_threads;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(admission.ocr_threads)
        .build()?;
    let preparation = PagePreparation {
        root: &root,
        settings: &preparation_settings,
        log: &log,
        total: pages,
        has_readers: !plan.models.is_empty(),
    };
    let mut rendered = vec![];
    let mut prepared = 0;
    let mut consume = |(n, im), control: &Control| {
        prepared += 1;
        let changed = pool.install(|| preparation.run(n, prepared, im, control))?;
        if changed {
            rendered.push(n);
        }
        Ok(())
    };
    // PDFium remains on this caller thread; only one page's OCR/preprocessing runs at a time.
    let selected = if admission.enabled {
        bounded_preparation(
            c,
            |control, send| {
                crate::inputs::render(
                    source,
                    &s.pdfium,
                    &s.page_range,
                    plan.dpi,
                    s.max_pixels,
                    control,
                    |n, im| send((n, im)),
                )
            },
            consume,
        )?
    } else {
        crate::inputs::render(
            source,
            &s.pdfium,
            &s.page_range,
            plan.dpi,
            s.max_pixels,
            c,
            |n, im| consume((n, im), c),
        )?
    };
    drop(pool);
    let manifest = Manifest {
        schema: 2,
        input_sha256: input,
        fingerprint: key,
        pages: selected.clone(),
        models: plan.models.iter().map(|m| m.id.clone()).collect(),
        settings: s.clone(),
    };
    let project = Project::open(root.clone(), manifest)?;
    let mut metadata = crate::projects::metadata(&project)?;
    if metadata.source.is_none() {
        metadata.source = Some(source.canonicalize()?);
        crate::projects::save_metadata(&project, &metadata)?;
    }
    let guidance = crate::assistance::load_guidance(&project)?;
    let context_notes = guidance.notes.join("\n");
    let total = selected.len() * (plan.models.len() + 1) + 1;
    let mut done = selected.len();
    let mut read_done = 0;
    let mut crop_done = 0;
    for (model_index, model) in plan.models.clone().iter().enumerate() {
        log.work(
            Work::new(Phase::Load, readers + model_index, readers * 2, 5000, true),
            "Preparing local reader",
        );
        c.check()?;
        let tasks = selected
            .iter()
            .copied()
            .filter(|n| {
                project
                    .load(*n)
                    .ok()
                    .is_none_or(|p| should_read(&model.id, &model.role, &p, s))
            })
            .collect::<Vec<_>>();
        if tasks.is_empty() {
            done += selected.len();
            read_done += selected.len();
            log.work(
                Work::new(Phase::Read, read_done, pages * readers, read_ms, false),
                "Reused reader checkpoints",
            );
            log.work(
                Work::new(
                    Phase::Load,
                    readers + model_index + 1,
                    readers * 2,
                    5000,
                    false,
                ),
                "Reader already complete",
            );
            continue;
        }
        // Re-admit against current free memory before allocating model weights.
        let current = hardware::detect_for(s, c);
        let fresh = planning::build(&current, s, std::slice::from_ref(model))?;
        if fresh.models.is_empty() {
            read_done += selected.len();
            log.work(
                Work::new(Phase::Read, read_done, pages * readers, read_ms, false),
                "Reader skipped by memory admission",
            );
            log.work(
                Work::new(
                    Phase::Load,
                    readers + model_index + 1,
                    readers * 2,
                    5000,
                    false,
                ),
                "Memory admission checked",
            );
            log.emit(
                "resource",
                "Model no longer fits free memory; preserving current readers",
                0,
                done,
                total,
                "NG300",
            );
            continue;
        }
        let mut active_settings = s.clone();
        if active_settings.image_edge == 0 {
            active_settings.image_edge = if model.prompt == "lighton" {
                if model.id == "lighton3-1b" {
                    1540
                } else {
                    2048
                }
            } else {
                plan.image_edge
            };
        }
        active_settings.context = plan.context;
        active_settings.image_tokens = plan.image_tokens;
        active_settings.max_tokens = s.max_tokens.min(
            plan.context
                .saturating_sub(plan.image_tokens + 6000)
                .max(512),
        );
        let mut server = Server::start(model, &plan, &active_settings, c, &log, &root);
        if let Err(e) = &server
            && e.downcast_ref::<Fault>().is_some_and(|f| f.code == "NG900")
        {
            return Err(server.err().unwrap());
        }
        if server.is_err() {
            log.emit(
                "recovery",
                "Retrying runtime with smaller working buffers and CPU projector",
                0,
                done,
                total,
                "NG202",
            );
            let recovery = recovery_plan(&plan);
            active_settings.projector_on_cpu = true;
            active_settings.context = recovery.context;
            active_settings.image_tokens = recovery.image_tokens;
            active_settings.max_tokens = active_settings.max_tokens.min(
                recovery
                    .context
                    .saturating_sub(recovery.image_tokens + 6000)
                    .max(512),
            );
            server = Server::start(model, &recovery, &active_settings, c, &log, &root);
        }
        let server = match server {
            Ok(v) => v,
            Err(e) => {
                if e.downcast_ref::<Fault>().is_some_and(|f| f.code == "NG900") {
                    return Err(e);
                }
                read_done += selected.len();
                log.work(
                    Work::new(Phase::Read, read_done, pages * readers, read_ms, false),
                    "Unavailable reader skipped; review required",
                );
                log.work(
                    Work::new(
                        Phase::Load,
                        readers + model_index + 1,
                        readers * 2,
                        5000,
                        false,
                    ),
                    "Reader startup finished",
                );
                log.emit(
                    "model",
                    format!(
                        "{} could not run: {e}; remaining evidence is preserved",
                        model.name
                    ),
                    0,
                    done,
                    total,
                    "NG200",
                );
                continue;
            }
        };
        log.work(
            Work::new(
                Phase::Load,
                readers + model_index + 1,
                readers * 2,
                5000,
                false,
            ),
            "Local reader ready",
        );
        for n in &selected {
            c.check()?;
            let mut page = project.load(*n)?;
            if !tasks.contains(n) {
                done += 1;
                read_done += 1;
                log.work(
                    Work::new(Phase::Read, read_done, pages * readers, read_ms, false),
                    "Reader checkpoint reused",
                );
                continue;
            }
            let started = Instant::now();
            log.work(
                Work::new(Phase::Read, read_done, pages * readers, read_ms, true),
                "Reading source page",
            );
            log.emit(
                "inference",
                format!(
                    "{} · {} of {} readers",
                    model.name,
                    model_index + 1,
                    plan.models.len()
                ),
                *n,
                done,
                total,
                "",
            );
            let im = image::open(project.image(*n))?;
            if s.experience.live_preview {
                log.preview(crate::telemetry::Preview::source(
                    project.image(*n),
                    *n,
                    &model.name,
                    None,
                ));
            }
            let previous = selected
                .iter()
                .position(|p| p == n)
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| project.load(selected[i]).ok())
                .map(|p| p.content.summary)
                .unwrap_or_default();
            let context = format!(
                "{}\n{}\nUser clarification context: {}\nPrevious selected page: {}",
                s.subject,
                s.assistance.context(),
                context_notes.chars().take(8000).collect::<String>(),
                previous
            );
            // First-pass readers are blind to one another to avoid artificial agreement.
            let evidence = String::new();
            let mut failure = None;
            let mut reading = server.read(&im, &evidence, &context, &active_settings, c, &log, *n);
            if reading
                .as_ref()
                .is_err_and(|e| e.downcast_ref::<Fault>().is_some_and(|f| f.code == "NG202"))
            {
                let mut reduced = active_settings.clone();
                reduced.image_edge = 1024;
                log.emit(
                    "recovery",
                    "GPU allocation failed; retrying page once at smaller image size",
                    *n,
                    done,
                    total,
                    "NG202",
                );
                reading = server.read(&im, &evidence, &context, &reduced, c, &log, *n);
            }
            match reading {
                Ok(content) => {
                    page.candidates.push(Candidate {
                        region: None,
                        model_id: model.id.clone(),
                        family: model.family.clone(),
                        view: "full".into(),
                        content,
                        reliability: 0.9,
                    });
                }
                Err(e) => {
                    if e.downcast_ref::<Fault>().is_some_and(|f| f.code == "NG900") {
                        return Err(e);
                    }
                    failure = Some(format!(
                        "{} failed: {e}. Original scan and remaining readings preserved.",
                        model.name
                    ));
                    log.emit(
                        "recovery",
                        "Reader failed; preserving other candidates",
                        *n,
                        done,
                        total,
                        "NG201",
                    );
                }
            }
            read_done += 1;
            log.work(
                Work::new(Phase::Read, read_done, pages * readers, read_ms, false),
                "Full-page reading finished",
            );
            let (mut content, agreement) =
                consensus::select(&page.candidates, s.consensus_min, s.agreement_threshold);
            // Low-confidence source crops provide high effective DPI without sending a huge whole-page image.
            if s.preset == Preset::Maximum && (agreement < s.agreement_threshold || s.handwriting) {
                let regions = scan::crop_regions(&content, &im, s);
                for (i, (bbox, crop)) in regions.into_iter().enumerate() {
                    c.check()?;
                    log.work(
                        Work::new(
                            Phase::Crop,
                            crop_done,
                            crop_reserved.max(crop_done + 1),
                            read_ms / 2,
                            true,
                        ),
                        "Reading a difficult crop",
                    );
                    log.emit(
                        "crop",
                        format!("Re-reading difficult region {} with {}", i + 1, model.name),
                        *n,
                        done,
                        total,
                        "",
                    );
                    let table_crop = content.blocks.iter().any(|b| {
                        b.kind == "table"
                            && b.bbox
                                .as_ref()
                                .is_some_and(|b| consensus::overlap(b, &bbox) > 0.5)
                    });
                    if s.experience.live_preview {
                        log.preview(crate::telemetry::Preview::source(
                            project.image(*n),
                            *n,
                            &model.name,
                            Some(bbox.clone()),
                        ));
                    }
                    let reading = if table_crop {
                        server.read_table(&crop, "", &context, &active_settings, c, &log, *n)
                    } else {
                        server.read(&crop, "", &context, &active_settings, c, &log, *n)
                    };
                    crop_done += 1;
                    log.work(
                        Work::new(
                            Phase::Crop,
                            crop_done,
                            crop_reserved.max(crop_done),
                            read_ms / 2,
                            false,
                        ),
                        "Crop reading finished",
                    );
                    if let Err(error) = &reading {
                        if error
                            .downcast_ref::<Fault>()
                            .is_some_and(|f| f.code == "NG900")
                        {
                            return Err(reading.unwrap_err());
                        }
                        log.emit(
                            "crop",
                            format!("Crop reader failed: {error}; full-page evidence retained"),
                            *n,
                            done,
                            total,
                            "NG201",
                        );
                    }
                    if let Ok(mut candidate) = reading {
                        for block in &mut candidate.blocks {
                            if let Some(b) = &mut block.bbox {
                                b.left = bbox.left + b.left * (bbox.right - bbox.left);
                                b.right = bbox.left + b.right * (bbox.right - bbox.left);
                                b.top = bbox.top + b.top * (bbox.bottom - bbox.top);
                                b.bottom = bbox.top + b.bottom * (bbox.bottom - bbox.top);
                            }
                        }
                        page.candidates.push(Candidate {
                            region: Some(bbox.clone()),
                            model_id: model.id.clone(),
                            family: model.family.clone(),
                            view: format!("crop-{i}"),
                            content: candidate,
                            reliability: 0.8,
                        });
                    }
                }
                content.warnings.push("Difficult-region crop readings are preserved in evidence. Review conflicting regions.".into());
            }
            // Prefer the explicit contextual diagram reader while preserving all of its figures.
            if let Some(v) = page.candidates.iter().rev().find(|v| {
                v.view == "full"
                    && v.content
                        .blocks
                        .iter()
                        .any(|b| b.kind == "figure" && !b.interpretation.is_empty())
            }) {
                merge_contextual_figures(&mut content, &v.content);
            }
            if let Some(warning) = failure {
                content.warnings.push(warning);
            }
            retain_source_warnings(&page.source_warnings, &mut content);
            page.content = content;
            page.agreement = agreement;
            page.seconds += started.elapsed().as_secs_f64();
            project.save(&page)?;
            done += 1;
            log.emit(
                "checkpoint",
                "Candidate readings and consensus saved",
                *n,
                done,
                total,
                "",
            );
        }
        drop(server);
    }
    log.work(
        Work::new(Phase::Crop, crop_done, crop_done, read_ms / 2, false),
        "Difficult-region passes finished",
    );
    atomic_json(&root.join("hardware.json"), &hw)?;
    atomic_json(&root.join("plan.json"), &plan)?;
    let mut review = 0;
    for (index, n) in selected.iter().enumerate() {
        log.work(
            Work::new(Phase::Finalize, index, pages, 100, true),
            "Reconciling page evidence",
        );
        let mut p = project.load(*n)?;
        if !p.edited && !p.reviewed {
            consensus::refine_regions(&mut p, s);
        }
        if p.content.blocks.is_empty() {
            p.content.blocks.push(Block {
                kind: "figure".into(),
                bbox: Some(BoxRect::full()),
                description: "Original page preserved; no reliable transcription was available."
                    .into(),
                uncertain: true,
                ..Default::default()
            });
        }
        if s.preset != Preset::Fast && !p.candidates.iter().any(|v| v.family != "tesseract") {
            p.content
                .warnings
                .push("No AI reader completed this page. OCR fallback needs full review.".into());
        }
        if p.needs_review(s.review_threshold) {
            review += 1;
        }
        if !p.edited && !p.reviewed {
            project.save(&p)?;
        }
    }
    log.work(
        Work::new(Phase::Finalize, pages, pages, 100, false),
        "Page evidence ready for review",
    );
    if s.assistance.ask_questions && s.assistance.max_questions > 0 {
        let questions =
            crate::assistance::questions(&project, s.assistance.max_questions, s.review_threshold)?;
        crate::assistance::save_questions(&project, &questions)?;
        if !questions.is_empty() {
            if let Some(clarify) = &c.clarify {
                log.emit(
                    "clarify",
                    format!(
                        "Waiting for you: {} uncertain regions. Answer, add context, or skip.",
                        questions.len()
                    ),
                    0,
                    done,
                    total,
                    "",
                );
                let answers = clarify(crate::assistance::Batch {
                    project: root.clone(),
                    questions: questions.clone(),
                })?;
                c.check()?;
                let changed = crate::assistance::apply(&project, &questions, &answers)?;
                log.emit(
                    "clarify",
                    format!("Applied {changed} confirmed corrections; reader evidence preserved."),
                    0,
                    done,
                    total,
                    "",
                );
                let contextual = answers
                    .iter()
                    .filter(|a| !a.skip && !a.context.trim().is_empty())
                    .filter_map(|a| questions.iter().find(|q| q.id == a.id).map(|q| q.page))
                    .collect::<Vec<_>>();
                if !contextual.is_empty()
                    && let Err(error) = reconsider(&project, &contextual, s, c, &log)
                {
                    if error
                        .downcast_ref::<Fault>()
                        .is_some_and(|f| f.code == "NG900")
                    {
                        return Err(error);
                    }
                    log.emit("context",format!("Context saved; automatic re-reading could not complete: {error:#}. Retry from Review."),0,done,total,"NG201");
                }
                crate::assistance::clear_pending(&project)?;
            } else {
                log.emit("clarify","Clarification questions saved. Open Review to answer; unattended conversion continues.",0,done,total,"");
            }
        }
        review = selected
            .iter()
            .filter(|n| {
                project
                    .load(**n)
                    .is_ok_and(|p| p.needs_review(s.review_threshold))
            })
            .count();
    }
    c.check()?;
    log.emit(
        "export",
        "Writing editable Word document",
        0,
        total - 1,
        total,
        "",
    );
    log.work(
        Work::new(Phase::Export, 0, 1, 2000, true),
        "Writing Word document",
    );
    let verified_input = crate::inputs::hash(source, c);
    if !verified_input
        .as_ref()
        .is_ok_and(|digest| digest == &project.manifest.input_sha256)
    {
        if c.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            c.check()?;
        }
        atomic_json(
            &root.join("input-invalidated.json"),
            &serde_json::json!({"expected":project.manifest.input_sha256,"observed":verified_input.ok(),"reason":"Source changed or became unavailable during conversion; checkpoints must not be reused."}),
        )?;
        return Err(Fault::new("NG100","Source changed or became unavailable during conversion. Export was withheld. Retry uses fresh checkpoints; previous evidence is preserved for inspection.").into());
    }
    word::export(&project, output, c)?;
    log.work(
        Work::new(Phase::Export, 1, 1, 2000, false),
        "Word document saved",
    );
    log.emit(
        "complete",
        format!("Word file saved · {review} pages to review"),
        0,
        total,
        total,
        "",
    );
    plan.notes.push(format!(
        "{} page render checkpoints reused",
        selected.len() - rendered.len()
    ));
    plan.notes.push(admission.reason.clone());
    Ok(Completed {
        project,
        output: output.into(),
        pages: selected.len(),
        review,
        seconds: begun.elapsed().as_secs_f64(),
        plan,
        preparation: admission,
    })
}

/// Appearance and Word typography do not invalidate costly recognition checkpoints.
pub fn recognition_settings(s: &Settings) -> Settings {
    let mut value = s.clone();
    value.ui = Appearance::default();
    value.experience = ExperienceOptions::default();
    value.writer_reference = None;
    value.assistance.ask_questions = true;
    value.assistance.max_questions = 8;
    value.assistance.web_lookup = false;
    value.font = Settings::default().font;
    value.font_size = 11.;
    value.keep_page_breaks = true;
    value.preserve_figures = true;
    value
}

/// Balanced skips a vision pass only after independent full-page agreement.
pub fn should_read(id: &str, role: &str, page: &Page, s: &Settings) -> bool {
    if page.edited
        || page.reviewed
        || page
            .candidates
            .iter()
            .any(|v| v.model_id == id && v.view == "full")
    {
        return false;
    }
    !(s.preset == Preset::Balanced
        && role == "vision"
        && !s.handwriting
        && page.agreement >= s.agreement_threshold
        && !page
            .content
            .blocks
            .iter()
            .any(|b| b.kind == "figure" || b.uncertain || b.plain().contains("[illegible]")))
}

pub fn retain_source_warnings(source: &[String], content: &mut PageContent) {
    for warning in source {
        if !content.warnings.contains(warning) {
            content.warnings.push(warning.clone());
        }
    }
}

pub fn merge_contextual_figures(target: &mut PageContent, incoming: &PageContent) {
    for figure in incoming.blocks.iter().filter(|b| b.kind == "figure") {
        target.blocks.retain(|old| {
            old.kind != "figure"
                || old
                    .bbox
                    .as_ref()
                    .zip(figure.bbox.as_ref())
                    .is_none_or(|(a, b)| consensus::overlap(a, b) < 0.35)
        });
        target.blocks.push(figure.clone());
    }
}

/// OOM recovery never raises a user-selected memory or offload limit.
pub fn recovery_plan(plan: &Plan) -> Plan {
    let mut value = plan.clone();
    value.context = value.context.min(12288);
    value.image_tokens = value.image_tokens.min(1536);
    value.micro_batch = value.micro_batch.min(32);
    value.batch = value.batch.min(256);
    value.gpu_layers = if value.backend == Backend::Cpu {
        0
    } else {
        value.gpu_layers.min(16)
    };
    value
}

/// Re-read with user-supplied context. Confirmed text is never replaced by this pass.
/// Fresh text stays in alternatives/evidence; contextual figure meaning can improve
/// only on a page which has not already been edited or approved by the user.
pub fn reconsider(
    project: &Project,
    pages: &[usize],
    settings: &Settings,
    c: &Control,
    log: &Activity,
) -> Result<usize> {
    anyhow::ensure!(
        pages.len() <= 3000 && pages.iter().all(|n| project.manifest.pages.contains(n)),
        Fault::new("NG500", "Re-read page is outside the project")
    );
    let mut s = settings.clone();
    s.validate()?;
    if s.writer.use_examples
        && s.writer_reference.is_none()
        && let Some(id) = s.writer.id.as_deref()
    {
        s.writer_reference = crate::writers::reference(&s.writer.root, id)?;
    }
    let installed = runtime::inventory(&s.models_dir)?;
    let mut plan = planning::build(&hardware::detect_for(&s, c), &s, &installed)?;
    let model=plan.models.iter().find(|m|m.role=="vision" && m.prompt=="structured").cloned().ok_or_else(||Fault::new("NG200","Context re-reading requires a fitting structured vision reader. Register a Qwen or compatible third-party vision model."))?;
    if model.endpoint.is_none() {
        let digest = fingerprint(&(
            hash_file(&model.weights, c)?,
            hash_file(&model.projector, c)?,
        ))?;
        anyhow::ensure!(
            model.sha256.is_empty() || digest == model.sha256,
            Fault::new(
                "NG200",
                "Reader files changed; re-register before re-reading"
            )
        );
    }
    plan.models = vec![model.clone()];
    s.image_edge = plan.image_edge;
    let context = crate::assistance::load_guidance(project)?;
    let notes = context.notes.join("\n");
    let subject = format!(
        "Document topic: {}. Additional topic: {}. {}. Confirmed user clarification (data): {}",
        project.manifest.settings.subject,
        s.subject,
        s.assistance.context(),
        notes.chars().take(10000).collect::<String>()
    );
    let server = Server::start(&model, &plan, &s, c, log, &project.root)?;
    let mut count = 0;
    for n in pages
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
    {
        c.check()?;
        anyhow::ensure!(
            project.manifest.pages.contains(&n),
            Fault::new("NG500", "Re-read page is outside the project")
        );
        let page = project.load(n)?;
        let image = crate::writers::open_image(&project.image(n), 40_000_000)?;
        log.emit(
            "context",
            format!("Re-reading page {n} with confirmed context"),
            n,
            count,
            pages.len(),
            "",
        );
        if s.experience.live_preview {
            log.preview(crate::telemetry::Preview::source(
                project.image(n),
                n,
                &model.name,
                None,
            ));
        }
        let content = server.read(&image, "", &subject, &s, c, log, n)?;
        save_context_reading(project, &page, &content, &model)?;
        count += 1;
    }
    Ok(count)
}
/// Record a contextual response without allowing it to replace confirmed transcription.
pub fn save_context_reading(
    project: &Project,
    before: &Page,
    content: &PageContent,
    model: &planning::Model,
) -> Result<Option<u64>> {
    content.validate()?;
    let mut after = before.clone();
    attach_context_reading(&mut after, content, model);
    if fingerprint(before)? == fingerprint(&after)? {
        return Ok(None);
    }
    Ok(Some(crate::review::commit(
        project,
        before,
        &after,
        &format!("Context re-read: {}", model.id),
    )?))
}
pub fn attach_context_reading(page: &mut Page, content: &PageContent, model: &planning::Model) {
    if !page.edited && !page.reviewed {
        merge_contextual_figures(&mut page.content, content);
    }
    for (i, block) in page.content.blocks.iter_mut().enumerate() {
        if let Some(other) = content.blocks.get(i).filter(|b| b.kind == block.kind) {
            let text = other.plain();
            if !text.is_empty()
                && text != block.plain()
                && !block.alternatives.contains(&text)
                && block.alternatives.len() < 64
            {
                block.alternatives.push(text);
            }
        }
    }
    if page.candidates.len() < 128 {
        page.candidates.push(Candidate {
            region: None,
            model_id: model.id.clone(),
            family: model.family.clone(),
            view: format!("clarification-{}", page.candidates.len()),
            reliability: 0.8,
            content: content.clone(),
        });
    }
    if !page
        .content
        .warnings
        .iter()
        .any(|w| w.starts_with("Context-assisted re-reading"))
    {
        page.content.warnings.push("Context-assisted re-reading is preserved as evidence. Confirmed text is protected; check alternatives against the source.".into());
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionRequest {
    pub page: usize,
    pub region: BoxRect,
    pub reader: String,
    pub image_edge: u32,
    pub context: String,
    pub task: String,
}
impl RegionRequest {
    pub fn validate(&self, project: &Project) -> Result<()> {
        anyhow::ensure!(
            project.manifest.pages.contains(&self.page)
                && self.region.valid()
                && crate::writers::valid_id(&self.reader)
                && (self.image_edge == 0 || (512..=8192).contains(&self.image_edge))
                && self.context.len() <= 8000
                && ["text", "handwriting", "table", "equation", "figure", "code"]
                    .contains(&self.task.as_str()),
            Fault::new(
                "NG102",
                "Choose a valid source region, installed reader, resolution and region task"
            )
        );
        Ok(())
    }
}
/// Convert crop-relative boxes to stable page coordinates and preserve the full response.
/// Existing text, table values and confirmed explanations are never overwritten.
pub fn attach_region_reading(
    page: &mut Page,
    region: &BoxRect,
    content: &PageContent,
    model: &planning::Model,
) -> Result<()> {
    anyhow::ensure!(
        region.valid() && page.candidates.len() < 128,
        Fault::new(
            "NG201",
            "Invalid region or reader-evidence capacity reached"
        )
    );
    content.validate()?;
    anyhow::ensure!(
        !content.blocks.is_empty(),
        Fault::new("NG201", "Region reader returned no usable source content")
    );
    let mut mapped = content.clone();
    let width = region.right - region.left;
    let height = region.bottom - region.top;
    for block in &mut mapped.blocks {
        block.bbox = Some(if let Some(b) = &block.bbox {
            BoxRect {
                left: region.left + b.left * width,
                top: region.top + b.top * height,
                right: (region.left + b.right * width).min(1.0),
                bottom: (region.top + b.bottom * height).min(1.0),
            }
        } else {
            region.clone()
        });
    }
    mapped.validate()?;
    let mut disagreement = false;
    for block in &mut page.content.blocks {
        if !block
            .bbox
            .as_ref()
            .is_some_and(|b| consensus::overlap(b, region) > 0.05)
        {
            continue;
        }
        let alternatives = mapped.blocks.iter().filter(|b| {
            b.kind == block.kind
                && b.bbox
                    .as_ref()
                    .zip(block.bbox.as_ref())
                    .is_some_and(|(a, b)| consensus::overlap(a, b) > 0.05)
        });
        for reading in alternatives {
            let text = if reading.kind == "figure" {
                format!(
                    "Observed: {}. Interpretation: {}",
                    reading.description, reading.interpretation
                )
            } else {
                reading.plain()
            };
            if !text.is_empty()
                && text != block.plain()
                && !block.alternatives.contains(&text)
                && block.alternatives.len() < 64
            {
                block.alternatives.push(text);
                block.uncertain = true;
                disagreement = true;
            }
        }
    }
    if disagreement {
        page.reviewed = false;
    }
    page.candidates.push(Candidate {
        region: Some(region.clone()),
        model_id: model.id.clone(),
        family: model.family.clone(),
        view: format!("manual-region-{}", page.candidates.len()),
        content: mapped,
        reliability: 0.8,
    });
    let warning = "Targeted region re-reading is retained as source-linked evidence. Compare alternatives before accepting a correction.";
    if !page.content.warnings.iter().any(|w| w == warning) {
        page.content.warnings.push(warning.into());
    }
    Ok(())
}
/// Run exactly one selected reader on one source crop and save an undoable evidence revision.
pub fn reread_region(
    project: &Project,
    request: &RegionRequest,
    settings: &Settings,
    c: &Control,
    log: &Activity,
) -> Result<u64> {
    request.validate(project)?;
    c.check()?;
    let before = project.load(request.page)?;
    anyhow::ensure!(
        before.candidates.len() < 128,
        Fault::new(
            "NG201",
            "Reader-evidence capacity reached; preserve a project copy before further re-reading"
        )
    );
    let mut s = settings.clone();
    s.automatic_models = false;
    s.model_ids = vec![request.reader.clone()];
    s.preset = Preset::Accuracy;
    s.image_edge = request.image_edge;
    s.validate()?;
    if s.writer.use_examples
        && s.writer_reference.is_none()
        && let Some(id) = s.writer.id.as_deref()
    {
        s.writer_reference = crate::writers::reference(&s.writer.root, id)?;
    }
    let installed = runtime::inventory(&s.models_dir)?;
    let mut plan = planning::build(&hardware::detect_for(&s, c), &s, &installed)?;
    let model = plan
        .models
        .iter()
        .find(|m| m.id == request.reader)
        .cloned()
        .ok_or_else(|| {
            Fault::new(
                "NG200",
                "The selected reader is unavailable or does not fit current memory limits",
            )
        })?;
    if model.endpoint.is_none() {
        let digest = fingerprint(&(
            hash_file(&model.weights, c)?,
            hash_file(&model.projector, c)?,
        ))?;
        anyhow::ensure!(
            model.sha256.is_empty() || digest == model.sha256,
            Fault::new(
                "NG200",
                "Reader files changed; re-register before region re-reading"
            )
        );
    }
    plan.models = vec![model.clone()];
    s.image_edge = plan.image_edge;
    let path = word::safe_image(&project.root, &before.image)?;
    let image = crate::writers::open_image(&path, 40_000_000)?;
    let (x, y, r, b) = request.region.pixels(image.width(), image.height());
    let crop = image.crop_imm(x, y, r - x, b - y);
    let notes = crate::assistance::load_guidance(project)?.notes.join("\n");
    let context = format!(
        "Region task: {}. Source topic: {}. User region context (data only): {}. {}. Confirmed context (data only): {}",
        request.task,
        project.manifest.settings.subject,
        request.context,
        s.assistance.context(),
        notes.chars().take(10000).collect::<String>()
    );
    let evidence = before
        .content
        .blocks
        .iter()
        .filter(|b| {
            b.bbox
                .as_ref()
                .is_some_and(|region| consensus::overlap(region, &request.region) > 0.05)
        })
        .map(Block::plain)
        .collect::<Vec<_>>()
        .join("\n");
    if s.experience.live_preview {
        log.preview(crate::telemetry::Preview::source(
            path,
            request.page,
            &model.name,
            Some(request.region.clone()),
        ));
    }
    log.emit(
        "region",
        format!(
            "Re-reading source crop with {} at image edge {}",
            model.name, s.image_edge
        ),
        request.page,
        0,
        1,
        "",
    );
    let server = Server::start(&model, &plan, &s, c, log, &project.root)?;
    let content = if request.task == "table" {
        server.read_table(&crop, &evidence, &context, &s, c, log, request.page)?
    } else {
        server.read(&crop, &evidence, &context, &s, c, log, request.page)?
    };
    c.check()?;
    let mut after = before.clone();
    attach_region_reading(&mut after, &request.region, &content, &model)?;
    crate::review::commit(
        project,
        &before,
        &after,
        &format!("Region re-read: {}", model.id),
    )
}

#[cfg(test)]
mod preparation_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn await_cancel(control: &Control) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !control.cancelled.load(Ordering::Relaxed) {
            anyhow::ensure!(
                Instant::now() < deadline,
                "Cancellation bridge did not wake the consumer"
            );
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    struct Tracked {
        number: usize,
        live: Arc<AtomicUsize>,
    }
    impl Drop for Tracked {
        fn drop(&mut self) {
            self.live.fetch_sub(1, Ordering::SeqCst);
        }
    }
    #[test]
    fn bounded_preparation_retains_at_most_three_items_and_preserves_order() {
        let live = Arc::new(AtomicUsize::new(0));
        let high = Arc::new(AtomicUsize::new(0));
        let created = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut seen = vec![];
        thread::scope(|scope| {
            let counted = created.clone();
            let release = scope.spawn(move || {
                started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                let limit = Instant::now() + Duration::from_secs(3);
                while counted.load(Ordering::SeqCst) < 3 {
                    assert!(
                        Instant::now() < limit,
                        "Producer never filled the bounded queue"
                    );
                    thread::sleep(Duration::from_millis(1));
                }
                assert_eq!(counted.load(Ordering::SeqCst), 3);
                release_tx.send(()).unwrap();
            });
            let seen = &mut seen;
            bounded_preparation(
                &Control::default(),
                |_, send| {
                    for number in 0..20 {
                        let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                        high.fetch_max(now, Ordering::SeqCst);
                        created.fetch_add(1, Ordering::SeqCst);
                        send(Tracked {
                            number,
                            live: live.clone(),
                        })?;
                    }
                    Ok(())
                },
                move |item: Tracked, _| {
                    if item.number == 0 {
                        started_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                    }
                    seen.push(item.number);
                    Ok(())
                },
            )
            .unwrap();
            release.join().unwrap();
        });
        assert_eq!(seen, (0..20).collect::<Vec<_>>());
        assert_eq!(high.load(Ordering::SeqCst), 3);
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn consumer_failure_unblocks_a_paused_producer_without_cancelling_parent() {
        let control = Control::default();
        let error = bounded_preparation(
            &control,
            |linked, send| {
                for number in 0..100 {
                    linked.check()?;
                    send(number)?;
                }
                Ok(())
            },
            |_: usize, _| {
                control.paused.store(true, Ordering::Relaxed);
                Err(Fault::new("NG110", "Synthetic OCR failure").into())
            },
        )
        .unwrap_err();
        assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG110");
        assert!(!control.cancelled.load(Ordering::Relaxed));
        assert!(control.paused.load(Ordering::Relaxed));
        control.paused.store(false, Ordering::Relaxed);
    }
    #[test]
    fn producer_failure_interrupts_current_consumer_and_keeps_original_error() {
        let control = Control::default();
        let (started_tx, started_rx) = mpsc::channel();
        let mut ended = false;
        let error = bounded_preparation(
            &control,
            |_, send| -> Result<()> {
                send(1usize)?;
                started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                Err(Fault::new("NG100", "Synthetic renderer failure").into())
            },
            |_, linked| {
                started_tx.send(()).unwrap();
                await_cancel(linked)?;
                ended = true;
                linked.check()
            },
        )
        .unwrap_err();
        assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG100");
        assert!(ended);
        assert!(!control.cancelled.load(Ordering::Relaxed));
    }
    #[test]
    fn parent_cancellation_reaches_both_stages_and_joins_before_return() {
        let control = Control::default();
        let mut consumed = 0;
        let mut consumer_observed_cancel = false;
        let error = bounded_preparation(
            &control,
            |linked, send| {
                for number in 0..100 {
                    linked.check()?;
                    send(number)?;
                }
                Ok(())
            },
            |_: usize, linked| {
                consumed += 1;
                control.cancelled.store(true, Ordering::Relaxed);
                await_cancel(linked)?;
                consumer_observed_cancel = true;
                linked.check()
            },
        )
        .unwrap_err();
        assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG900");
        assert_eq!(consumed, 1);
        assert!(consumer_observed_cancel);
    }
    #[test]
    fn either_stage_panic_is_reported_and_no_worker_is_detached() {
        for producer_panics in [false, true] {
            let control = Control::default();
            let (started_tx, started_rx) = mpsc::channel();
            let mut consumer_ended = false;
            let error = bounded_preparation(
                &control,
                |_, send| -> Result<()> {
                    send(0usize)?;
                    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                    if producer_panics {
                        panic!("Synthetic producer panic");
                    }
                    for number in 1..100 {
                        send(number)?;
                    }
                    Ok(())
                },
                |_, linked| {
                    started_tx.send(()).unwrap();
                    if !producer_panics {
                        panic!("Synthetic consumer panic");
                    }
                    await_cancel(linked)?;
                    consumer_ended = true;
                    linked.check()
                },
            )
            .unwrap_err();
            assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG500");
            assert!(!control.cancelled.load(Ordering::Relaxed));
            assert_eq!(consumer_ended, producer_panics);
        }
    }
}
