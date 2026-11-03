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
        .min((hw.ram_mib as f64 * f64::from(s.ram_fraction)) as u64)
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
    let mut linked = c.clone();
    linked.cancelled = Arc::new(AtomicBool::new(false));
    linked.clarify = None;
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
    prepare_only: bool,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingRecognition {
    schema: u32,
    source: crate::redactions::SourceIdentity,
    phase: u8,
    page_fingerprint: String,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedInput {
    schema: u32,
    source: PathBuf,
    input_sha256: String,
    fingerprint: String,
}
fn pending_recognition(root: &Path, page: usize) -> PathBuf {
    root.join(format!("page-{page:05}.pending-recognition.json"))
}
fn update_pending(project: &Project, page: &Page) -> Result<()> {
    let path = pending_recognition(&project.root, page.page);
    crate::watch::no_links(&path)?;
    if path.exists() {
        let mut pending: PendingRecognition = read_json(&path, 100_000)?;
        pending.phase = 1;
        pending.page_fingerprint = fingerprint(page)?;
        atomic_json(&path, &pending)?;
    }
    Ok(())
}
fn pending_reader(project: &Project, page: &Page, id: &str) -> Result<bool> {
    let path = pending_recognition(&project.root, page.page);
    crate::watch::no_links(&path)?;
    if !path.exists() {
        return Ok(false);
    }
    let pending: PendingRecognition = read_json(&path, 100_000)?;
    anyhow::ensure!(
        pending.schema == 1 && pending.phase == 1 && pending.page_fingerprint == fingerprint(page)?,
        Fault::new(
            "NG500",
            "Prepared recognition checkpoint changed; preserve edits and use explicit Review rereading"
        )
    );
    Ok(!page.reviewed
        && !page
            .candidates
            .iter()
            .any(|candidate| candidate.model_id == id && candidate.view == "full"))
}
fn sanitize_masked_readings(
    page: &mut Page,
    identity: &crate::redactions::SourceIdentity,
    c: &Control,
) -> Result<()> {
    if page.confirmed_redactions.is_empty() {
        return Ok(());
    }
    page.content = crate::redactions::enforce_view(
        &page.content,
        identity,
        &page.confirmed_redactions,
        None,
        c,
    )?
    .0;
    for candidate in &mut page.candidates {
        // Crop candidates have already been mapped back to full-page boxes.
        candidate.content = crate::redactions::enforce_view(
            &candidate.content,
            identity,
            &page.confirmed_redactions,
            None,
            c,
        )?
        .0;
    }
    page.source_warnings.clear();
    page.manual_layers.clear();
    Ok(())
}
fn verify_reader_source(
    project: &Project,
    page: &Page,
    expected: &crate::redactions::SourceIdentity,
    c: &Control,
) -> Result<()> {
    anyhow::ensure!(
        crate::redactions::source_identity(project, page, c)? == *expected,
        Fault::new(
            "NG500",
            "Source changed during recognition; response was withheld"
        )
    );
    Ok(())
}
fn pixel_view(region: &BoxRect, image: &image::DynamicImage) -> BoxRect {
    let (left, top, right, bottom) = region.pixels(image.width(), image.height());
    BoxRect {
        left: left as f32 / image.width() as f32,
        top: top as f32 / image.height() as f32,
        right: right as f32 / image.width() as f32,
        bottom: bottom as f32 / image.height() as f32,
    }
}
impl PagePreparation<'_> {
    fn run(&self, n: usize, ordinal: usize, im: image::DynamicImage, c: &Control) -> Result<bool> {
        let root = self.root;
        let s = self.settings;
        let log = self.log;
        let pages = self.total;
        let image = root.join(format!("page-{n:05}.png"));
        let checkpoint = root.join(format!("page-{n:05}.json"));
        let overlay = root.join(format!("page-{n:05}.edits.json"));
        let pending_path = pending_recognition(root, n);
        let saved_path = if fs::symlink_metadata(&overlay).is_ok() {
            &overlay
        } else {
            &checkpoint
        };
        if fs::symlink_metadata(saved_path).is_ok() {
            crate::watch::no_links(saved_path)?;
        }
        let saved_page = if saved_path.exists() {
            crate::watch::no_links(saved_path)?;
            match read_json::<Page>(saved_path, 32_000_000) {
                Ok(saved) => Some(saved),
                Err(error) if pending_path.exists() || saved_path == &overlay => return Err(error),
                Err(_) => None,
            }
        } else {
            None
        };
        if let Some(mut saved) = saved_page {
            if !saved.confirmed_redactions.is_empty() || pending_path.exists() {
                anyhow::ensure!(
                    saved.page == n && saved.image == format!("page-{n:05}.png"),
                    Fault::new("NG500", "Prepared page identity changed")
                );
                let project = Project {
                    root: root.into(),
                    manifest: Manifest {
                        schema: 2,
                        input_sha256: String::new(),
                        fingerprint: String::new(),
                        pages: vec![n],
                        models: vec![],
                        settings: s.clone(),
                    },
                };
                let (identity, masked) = crate::redactions::recognition_view(&project, &saved, c)?;
                if !pending_path.exists() {
                    return Ok(false);
                }
                crate::watch::no_links(&pending_path)?;
                let mut pending: PendingRecognition = read_json(&pending_path, 100_000)?;
                anyhow::ensure!(
                    pending.schema == 1 && pending.source == identity && pending.phase <= 1,
                    Fault::new(
                        "NG500",
                        "Prepared source changed; prepare and confirm masks again"
                    )
                );
                if pending.phase == 1 {
                    anyhow::ensure!(
                        !self.prepare_only && pending.page_fingerprint == fingerprint(&saved)?,
                        Fault::new(
                            "NG500",
                            "Prepared page was recognized or manually edited; use Review without overwriting it"
                        )
                    );
                    return Ok(false);
                }
                if self.prepare_only {
                    return Ok(false);
                }
                let before_recognition = saved.clone();
                anyhow::ensure!(
                    saved.candidates.is_empty()
                        && saved.content.summary.is_empty()
                        && saved.content.blocks.iter().all(|block| block.kind == "text"
                            && block.text == "[redacted]"
                            && block.bbox.as_ref().is_some_and(|bounds| saved
                                .confirmed_redactions
                                .iter()
                                .any(|mask| mask.region == *bounds))),
                    Fault::new(
                        "NG500",
                        "Prepared page contains manual text; preserve it and use explicit Review rereading"
                    )
                );
                let safe = crate::redactions::recognition_settings(s, &saved.confirmed_redactions)?;
                let mut content = match scan::printed(&masked, &safe, c) {
                    Ok(content) => content,
                    Err(error)
                        if !self.has_readers
                            || error
                                .downcast_ref::<Fault>()
                                .is_some_and(|fault| fault.code == "NG900") =>
                    {
                        return Err(error);
                    }
                    Err(_) => PageContent {
                        warnings: vec![
                            "Printed OCR unavailable; prepared source retained for AI recognition."
                                .into(),
                        ],
                        ..Default::default()
                    },
                };
                crate::geometry::bind_content(&mut content, &identity)?;
                let content = crate::redactions::enforce_view(
                    &content,
                    &identity,
                    &saved.confirmed_redactions,
                    None,
                    c,
                )?
                .0;
                saved.script_detection = Some(crate::language_routing::bind_detection(
                    &identity,
                    scan::detect_script(&masked, &safe, c)?,
                )?);
                saved.content = content.clone();
                saved.source_warnings = content.warnings.clone();
                saved.candidates = vec![Candidate {
                    region: None,
                    model_id: "tesseract".into(),
                    family: "tesseract".into(),
                    view: "full".into(),
                    reliability: if s.handwriting { 0.25 } else { 0.65 },
                    content,
                }];
                saved.reviewed = false;
                saved.edited = true;
                saved.agreement = if s.preset == Preset::Fast { 1. } else { 0. };
                verify_reader_source(&project, &saved, &identity, c)?;
                let canonical_project = Project::read(root.into())?;
                crate::review::commit(
                    &canonical_project,
                    &before_recognition,
                    &saved,
                    "Recognize prepared source",
                )?;
                pending.phase = 1;
                pending.page_fingerprint = fingerprint(&saved)?;
                atomic_json(&pending_path, &pending)?;
                return Ok(true);
            }
            anyhow::ensure!(
                !self.prepare_only,
                Fault::new(
                    "NG500",
                    "This source already has recognition; confirm masks in Review or choose a new preparation folder"
                )
            );
        }
        anyhow::ensure!(
            !pending_path.exists(),
            Fault::new(
                "NG500",
                "Prepared source checkpoint is missing; recover it before recognition"
            )
        );
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
        if self.prepare_only {
            // Pixel-only deskew is allowed; OSD/orientation would itself run OCR.
            let im = scan::deskew(im, s.deskew, s.max_pixels);
            scan::save_scan(&image, &im)?;
            let page = Page { page: n, image: format!("page-{n:05}.png"), content: PageContent { warnings: vec!["Source prepared without OCR, orientation detection or AI. Confirm masks, then explicitly recognize this prepared source.".into()], ..Default::default() }, ..Default::default() };
            let project = Project {
                root: root.into(),
                manifest: Manifest {
                    schema: 2,
                    input_sha256: String::new(),
                    fingerprint: String::new(),
                    pages: vec![n],
                    models: vec![],
                    settings: s.clone(),
                },
            };
            let pending = PendingRecognition {
                schema: 1,
                source: crate::redactions::source_identity(&project, &page, c)?,
                phase: 0,
                page_fingerprint: fingerprint(&page)?,
            };
            atomic_json(&pending_path, &pending)?;
            atomic_json(&checkpoint, &page)?;
            log.emit(
                "prepare",
                "Source page saved without recognition; mask confirmation is available",
                n,
                ordinal,
                pages,
                "",
            );
            return Ok(true);
        }
        log.emit(
            "render",
            "Preparing source scan and printed OCR",
            n,
            ordinal - 1,
            0,
            "",
        );
        let preflight = crate::preflight::analyze(&im);
        let oriented = scan::orient_with_detection(im, s, c)?;
        let im = scan::deskew(oriented.image, s.deskew, s.max_pixels);
        let mut content = match scan::printed(&im, s, c) {
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
        let identity = crate::geometry::source_identity(&image, n, c)?;
        crate::geometry::bind_content(&mut content, &identity)?;
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
            script_detection: Some(crate::language_routing::bind_detection(
                &identity,
                oriented.detection,
            )?),
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
    convert_internal(
        source,
        output,
        s,
        c,
        activity,
        preparation_mode,
        None,
        ConversionIntent::Recognize,
    )
}
/// Explicit normal-workspace opt-in. Retention sessions and hardware trials keep
/// using the cold conversion API and never supply a pool.
pub fn convert_with_runtime(
    source: &Path,
    output: &Path,
    s: &Settings,
    c: &Control,
    activity: &Activity,
    preparation_mode: PreparationMode,
    pool: &crate::warm_runtime::WarmPool,
) -> Result<Completed> {
    convert_internal(
        source,
        output,
        s,
        c,
        activity,
        preparation_mode,
        Some(pool),
        ConversionIntent::Recognize,
    )
}
#[derive(Clone, Copy)]
enum ConversionIntent<'a> {
    Recognize,
    PrepareOnly,
    Prepared(&'a Project),
}
/// Save immutable source rasters and pending checkpoints without OCR, OSD or AI.
/// `output_hint` selects the ordinary private project folder; no Word is written.
pub fn prepare_for_review(
    source: &Path,
    output_hint: &Path,
    settings: &Settings,
    c: &Control,
    activity: &Activity,
) -> Result<Project> {
    Ok(convert_internal(
        source,
        output_hint,
        settings,
        c,
        activity,
        PreparationMode::Sequential,
        None,
        ConversionIntent::PrepareOnly,
    )?
    .project)
}
/// Recognize an explicitly prepared project, rejecting changed input/settings
/// before any reader. Output may be written elsewhere without moving the project.
pub fn recognize_prepared(
    project: &Project,
    output: &Path,
    settings: &Settings,
    c: &Control,
    activity: &Activity,
) -> Result<Completed> {
    crate::watch::no_links(&project.root.join("prepared-input.json"))?;
    let prepared: PreparedInput = read_json(&project.root.join("prepared-input.json"), 100_000)?;
    anyhow::ensure!(
        prepared.schema == 1
            && prepared.input_sha256 == project.manifest.input_sha256
            && prepared.fingerprint == project.manifest.fingerprint
            && crate::inputs::hash(&prepared.source, c)? == prepared.input_sha256,
        Fault::new(
            "NG500",
            "Prepared input changed; prepare the new source and explicitly confirm its masks"
        )
    );
    convert_internal(
        &prepared.source,
        output,
        settings,
        c,
        activity,
        PreparationMode::Sequential,
        None,
        ConversionIntent::Prepared(project),
    )
}
#[allow(clippy::too_many_arguments)]
fn convert_internal(
    source: &Path,
    output: &Path,
    s: &Settings,
    c: &Control,
    activity: &Activity,
    preparation_mode: PreparationMode,
    pool: Option<&crate::warm_runtime::WarmPool>,
    intent: ConversionIntent<'_>,
) -> Result<Completed> {
    let prepare_only = matches!(intent, ConversionIntent::PrepareOnly);
    let mut warm_job = pool.map(|pool| pool.begin(c)).transpose()?;
    let begun = Instant::now();
    s.validate()?;
    let resolved = match intent {
        ConversionIntent::Prepared(project) => {
            crate::project_lock::resolve_for_project(project, s, c)?
        }
        _ => crate::project_lock::resolve_settings(s, c)?,
    };
    let s = resolved.as_ref().map(|value| &value.settings).unwrap_or(s);
    let language_digest = if s.tessdata_dir.as_os_str().is_empty() {
        String::new()
    } else {
        if !prepare_only {
            crate::languages::verify_selection(&s.tessdata_dir, &s.languages, c)?;
        }
        fingerprint(&(s.languages.clone(), include_str!("../languages.json")))?
    };
    let mut effective = s.clone();
    let writer_digest = if s.preset != Preset::Fast && s.writer.use_examples {
        crate::writers::digest(&s.writer.root, s.writer.id.as_deref())?
    } else {
        String::new()
    };
    if !prepare_only
        && s.preset != Preset::Fast
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
    let installed = if let Some(locked) = &resolved {
        locked.models.clone()
    } else if s.preset == Preset::Fast {
        vec![]
    } else {
        crate::activation::recover_all(&s.models_dir, c)?;
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
    if !prepare_only && s.preset != Preset::Fast && (s.handwriting || s.preset != Preset::Balanced)
    {
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
    if let Some(locked) = &resolved {
        anyhow::ensure!(
            locked.input_sha256 == input,
            Fault::new(
                "NG500",
                "Pinned reader lock belongs to different source bytes"
            )
        );
    }
    let key = cache_identity(s, &plan, &writer_digest, &language_digest, c)?;
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
    if let ConversionIntent::Prepared(expected) = intent {
        anyhow::ensure!(
            expected.manifest.input_sha256 == input && expected.manifest.fingerprint == key,
            Fault::new(
                "NG500",
                "Prepared recognition settings or reader versions changed; prepare and confirm masks again"
            )
        );
        root = expected.root.canonicalize()?;
        anyhow::ensure!(
            !root.join("input-invalidated.json").exists(),
            Fault::new(
                "NG500",
                "Prepared source was invalidated; prepare and confirm a fresh source"
            )
        );
    }
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
        prepare_only,
    };
    let mut rendered = vec![];
    let mut prepared = 0;
    let mut consume = |(n, im), control: &Control| {
        prepared += 1;
        let changed = pool.install(|| preparation.run(n, prepared, im, control))?;
        if changed {
            rendered.push(n);
        }
        control.checkpoint()?;
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
    let new_project = !root.join("project.json").exists();
    let mut project = Project::open(root.clone(), manifest)?;
    if root.join("prepared-input.json").exists() {
        // Review history is bound to the persisted manifest, including its
        // original presentation settings, rather than a caller's UI clone.
        project = Project::read(root.clone())?;
    }
    if new_project {
        crate::project_lock::ensure_conversion_lock(&project, &plan.models, s, c)?;
        crate::project_layout::initialize_new(&project, source, c)?;
    }
    let mut metadata = crate::projects::metadata(&project)?;
    if metadata.source.is_none() {
        metadata.source = Some(source.canonicalize()?);
        crate::projects::save_metadata(&project, &metadata)?;
    }
    if prepare_only {
        atomic_json(
            &root.join("prepared-input.json"),
            &PreparedInput {
                schema: 1,
                source: source.canonicalize()?,
                input_sha256: project.manifest.input_sha256.clone(),
                fingerprint: project.manifest.fingerprint.clone(),
            },
        )?;
        log.emit("prepare", "Source pages prepared without recognition. Confirm masks, then recognize this prepared project.", 0, selected.len(), selected.len(), "");
        return Ok(Completed {
            project,
            output: output.into(),
            pages: selected.len(),
            review: selected.len(),
            seconds: begun.elapsed().as_secs_f64(),
            plan,
            preparation: admission,
        });
    }
    let private_project = crate::redactions::has_masks(&project, c)?;
    if private_project {
        drop(warm_job.take());
        anyhow::ensure!(
            !s.external_server && plan.models.iter().all(|model| model.endpoint.is_none()),
            Fault::new(
                "NG200",
                "Confirmed masks require fresh internally owned readers; external endpoints are withheld"
            )
        );
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
        let mut tasks = vec![];
        for n in &selected {
            let page = project.load(*n)?;
            if pending_reader(&project, &page, &model.id)?
                || should_read(&model.id, &model.role, &page, s)
            {
                tasks.push(*n);
            }
        }
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
        let mut active_settings = if private_project {
            crate::redactions::private_recognition_settings(s)
        } else {
            s.clone()
        };
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
        let mut server = if let Some(job) = warm_job.as_mut() {
            job.reader(model, &plan, &active_settings, c, &log, &root)
        } else {
            Server::start(model, &plan, &active_settings, c, &log, &root)
                .map(crate::warm_runtime::ReaderLease::cold)
        };
        if server.is_err()
            && let Some(job) = warm_job.as_mut()
        {
            job.taint();
        }
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
            server = if let Some(job) = warm_job.as_mut() {
                job.reader(model, &recovery, &active_settings, c, &log, &root)
            } else {
                Server::start(model, &recovery, &active_settings, c, &log, &root)
                    .map(crate::warm_runtime::ReaderLease::cold)
            };
            if server.is_err()
                && let Some(job) = warm_job.as_mut()
            {
                job.taint();
            }
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
            let before_reading = page.clone();
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
            let (identity, im) = crate::redactions::recognition_view(&project, &page, c)?;
            sanitize_masked_readings(&mut page, &identity, c)?;
            if !private_project && s.experience.live_preview {
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
            let read = |settings: &Settings| {
                if private_project {
                    server.read_masked(
                        &im,
                        &identity,
                        &page.confirmed_redactions,
                        None,
                        settings,
                        c,
                        &log,
                        *n,
                    )
                } else {
                    server.read(&im, &evidence, &context, settings, c, &log, *n)
                }
            };
            let mut reading = read(&active_settings);
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
                reading = read(&reduced);
            }
            verify_reader_source(&project, &page, &identity, c)?;
            match reading {
                Ok(mut content) => {
                    crate::geometry::bind_reader_content(&mut content, &identity)?;
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
                    let bbox = pixel_view(&bbox, &im);
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
                    if !private_project && s.experience.live_preview {
                        log.preview(crate::telemetry::Preview::source(
                            project.image(*n),
                            *n,
                            &model.name,
                            Some(bbox.clone()),
                        ));
                    }
                    let reading = if private_project && table_crop {
                        server.read_table_masked(
                            &crop,
                            &identity,
                            &page.confirmed_redactions,
                            Some(&bbox),
                            &active_settings,
                            c,
                            &log,
                            *n,
                        )
                    } else if private_project {
                        server.read_masked(
                            &crop,
                            &identity,
                            &page.confirmed_redactions,
                            Some(&bbox),
                            &active_settings,
                            c,
                            &log,
                            *n,
                        )
                    } else if table_crop {
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
                        crate::geometry::bind_reader_content(&mut candidate, &identity)?;
                        crate::geometry::map_content(&mut candidate, &bbox, &identity)?;
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
            verify_reader_source(&project, &page, &identity, c)?;
            sanitize_masked_readings(&mut page, &identity, c)?;
            page.agreement = agreement;
            page.seconds += started.elapsed().as_secs_f64();
            if pending_recognition(&project.root, *n).exists() {
                page.edited = true;
                crate::review::commit(
                    &project,
                    &before_reading,
                    &page,
                    "Prepared reader evidence",
                )?;
            } else {
                project.save(&page)?;
            }
            update_pending(&project, &page)?;
            done += 1;
            log.emit(
                "checkpoint",
                "Candidate readings and consensus saved",
                *n,
                done,
                total,
                "",
            );
            c.checkpoint()?;
        }
        if let Some(job) = warm_job.as_mut() {
            job.finish_reader(server, c, &log);
        } else {
            drop(server);
        }
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
        let before_finalize = p.clone();
        let is_pending = pending_recognition(&project.root, *n).exists();
        if (!p.edited || is_pending) && !p.reviewed {
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
        if !p.confirmed_redactions.is_empty() {
            let identity = crate::redactions::source_identity(&project, &p, c)?;
            sanitize_masked_readings(&mut p, &identity, c)?;
        }
        if p.needs_review(s.review_threshold) {
            review += 1;
        }
        if (!p.edited || is_pending || !p.confirmed_redactions.is_empty()) && !p.reviewed {
            if is_pending {
                p.edited = true;
                if fingerprint(&p)? != fingerprint(&before_finalize)? {
                    crate::review::commit(
                        &project,
                        &before_finalize,
                        &p,
                        "Finalize prepared recognition",
                    )?;
                }
            } else {
                project.save(&p)?;
            }
            update_pending(&project, &p)?;
        }
        c.checkpoint()?;
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
    c.checkpoint()?;
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
    for number in &selected {
        let pending = pending_recognition(&project.root, *number);
        if pending.exists() {
            fs::remove_file(pending)?;
        }
    }
    if let Some(job) = warm_job
        && let Err(error) = job.commit()
    {
        log.emit(
            "model",
            format!("Completed output retained; warm reader released: {error}"),
            0,
            0,
            0,
            "NG200",
        );
    }
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
fn ocr_executable(s: &Settings) -> Option<PathBuf> {
    if s.tesseract.is_file() {
        return s.tesseract.canonicalize().ok();
    }
    if s.tesseract.components().count() != 1 {
        return None;
    }
    let paths = std::env::var_os("PATH")?;
    for folder in std::env::split_paths(&paths) {
        let path = folder.join(&s.tesseract);
        if path.is_file() {
            return path.canonicalize().ok();
        }
        if cfg!(windows) {
            let exe = path.with_extension("exe");
            if exe.is_file() {
                return exe.canonicalize().ok();
            }
        }
    }
    None
}
fn cache_identity(
    s: &Settings,
    plan: &Plan,
    writer_digest: &str,
    language_digest: &str,
    c: &Control,
) -> Result<String> {
    let mut settings = recognition_settings(s);
    settings.server = PathBuf::new();
    settings.models_dir = PathBuf::new();
    settings.shared_model_store = PathBuf::new();
    settings.project_lock = None;
    settings.automatic_models = false;
    settings.backend = plan.backend;
    settings.model_ids = plan.models.iter().map(|m| m.id.clone()).collect();
    let runtime = if !s.external_server && plan.models.iter().any(|m| m.endpoint.is_none()) {
        Some(crate::canary::runtime_identity(&s.server, c)?.fingerprint)
    } else {
        None
    };
    let ocr = ocr_executable(s)
        .map(|path| hash_file(&path, c))
        .transpose()?;
    fingerprint(&(
        5,
        settings,
        runtime,
        ocr,
        writer_digest,
        language_digest,
        (plan.image_edge, plan.context, plan.dpi, plan.image_tokens),
        plan.models
            .iter()
            .map(|m| {
                (
                    &m.id,
                    fs::metadata(&m.weights).ok().map(|v| v.len()),
                    &m.sha256,
                    &m.endpoint,
                    &m.alias,
                    &m.revision,
                )
            })
            .collect::<Vec<_>>(),
    ))
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct PlannedIdentity {
    pub input_sha256: String,
    pub fingerprint: String,
    pub model_ids: Vec<String>,
    pub external_readers: bool,
    pub ocr_executable_verified: bool,
    pub pages: Vec<usize>,
}
/// Read-only duplicate admission uses the exact same key as conversion. Local
/// executable/model bytes are checked; an HTTP alias cannot prove a version.
pub fn planned_identity(
    source: &Path,
    settings: &Settings,
    c: &Control,
) -> Result<PlannedIdentity> {
    settings.validate()?;
    let resolved = crate::project_lock::resolve_settings(settings, c)?;
    let s = resolved.as_ref().map_or(settings, |r| &r.settings);
    let language = if s.tessdata_dir.as_os_str().is_empty() {
        String::new()
    } else {
        crate::languages::verify_selection(&s.tessdata_dir, &s.languages, c)?;
        fingerprint(&(s.languages.clone(), include_str!("../languages.json")))?
    };
    let writer = if s.preset != Preset::Fast && s.writer.use_examples {
        crate::writers::digest(&s.writer.root, s.writer.id.as_deref())?
    } else {
        String::new()
    };
    let installed = if let Some(lock) = &resolved {
        lock.models.clone()
    } else if s.preset == Preset::Fast {
        vec![]
    } else {
        crate::activation::recover_all(&s.models_dir, c)?;
        runtime::inventory_available(&s.models_dir)?.0
    };
    let mut plan = planning::build(&hardware::detect_for(s, c), s, &installed)?;
    for model in &mut plan.models {
        if model.endpoint.is_none() {
            let hash = fingerprint(&(
                hash_file(&model.weights, c)?,
                hash_file(&model.projector, c)?,
            ))?;
            anyhow::ensure!(
                model.sha256.is_empty() || model.sha256 == hash,
                Fault::new("NG200", "Model checksum changed; duplicate reuse withheld")
            );
            model.sha256 = hash;
        }
    }
    let input = crate::inputs::hash(source, c)?;
    let total = crate::inputs::selected_page_count(source, &s.pdfium, "")?;
    let pages = pages_range(&s.page_range, total)?;
    if let Some(lock) = &resolved {
        anyhow::ensure!(
            lock.input_sha256 == input,
            Fault::new(
                "NG500",
                "Selected model lock belongs to different source bytes"
            )
        );
    }
    Ok(PlannedIdentity {
        input_sha256: input,
        fingerprint: cache_identity(s, &plan, &writer, &language, c)?,
        model_ids: plan.models.iter().map(|m| m.id.clone()).collect(),
        external_readers: s.external_server || plan.models.iter().any(|m| m.endpoint.is_some()),
        ocr_executable_verified: ocr_executable(s).is_some(),
        pages,
    })
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
    let mut requested = settings.clone();
    if requested.preset == Preset::Fast {
        requested.preset = Preset::Accuracy;
    }
    let resolved = crate::project_lock::resolve_for_project(project, &requested, c)?;
    let private = crate::redactions::has_masks(project, c)?;
    let mut s = resolved
        .as_ref()
        .map(|value| value.settings.clone())
        .unwrap_or_else(|| requested.clone());
    if private {
        s = crate::redactions::private_recognition_settings(&s);
    }
    s.validate()?;
    if s.writer.use_examples
        && s.writer_reference.is_none()
        && let Some(id) = s.writer.id.as_deref()
    {
        s.writer_reference = crate::writers::reference(&s.writer.root, id)?;
    }
    let installed = if let Some(locked) = &resolved {
        locked.models.clone()
    } else {
        crate::activation::recover_all(&s.models_dir, c)?;
        runtime::inventory(&s.models_dir)?
    };
    let mut plan = planning::build(&hardware::detect_for(&s, c), &s, &installed)?;
    let model=plan.models.iter().find(|m|m.role=="vision" && m.prompt=="structured").cloned().ok_or_else(||Fault::new("NG200","Context re-reading requires a fitting structured vision reader. Register a Qwen or compatible third-party vision model."))?;
    anyhow::ensure!(
        !private || !s.external_server && model.endpoint.is_none(),
        Fault::new(
            "NG200",
            "Confirmed masks require a fresh internally owned reader"
        )
    );
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
        let (identity, image) = crate::redactions::recognition_view(project, &page, c)?;
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
        let mut content = if private {
            server.read_masked(
                &image,
                &identity,
                &page.confirmed_redactions,
                None,
                &s,
                c,
                log,
                n,
            )?
        } else {
            server.read(&image, "", &subject, &s, c, log, n)?
        };
        crate::geometry::bind_reader_content(&mut content, &identity)?;
        verify_reader_source(project, &page, &identity, c)?;
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
    if !before.confirmed_redactions.is_empty() {
        crate::redactions::source_identity(project, before, &Control::default())?;
    }
    attach_context_reading(&mut after, content, model)?;
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
pub fn attach_context_reading(
    page: &mut Page,
    content: &PageContent,
    model: &planning::Model,
) -> Result<()> {
    let safe_content;
    let identity = page
        .confirmed_redactions
        .first()
        .map(|mask| mask.source.clone());
    let content = if let Some(identity) = &identity {
        safe_content = crate::redactions::enforce_view(
            content,
            identity,
            &page.confirmed_redactions,
            None,
            &Control::default(),
        )?
        .0;
        sanitize_masked_readings(page, identity, &Control::default())?;
        &safe_content
    } else {
        content
    };
    content.validate()?;
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
    if let Some(identity) = identity {
        sanitize_masked_readings(page, &identity, &Control::default())?;
    }
    Ok(())
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
    let safe_content;
    let identity = page
        .confirmed_redactions
        .first()
        .map(|mask| mask.source.clone());
    let content = if let Some(identity) = &identity {
        safe_content = crate::redactions::enforce_view(
            content,
            identity,
            &page.confirmed_redactions,
            Some(region),
            &Control::default(),
        )?
        .0;
        sanitize_masked_readings(page, identity, &Control::default())?;
        &safe_content
    } else {
        content
    };
    anyhow::ensure!(
        !content.blocks.is_empty(),
        Fault::new("NG201", "Region reader returned no usable source content")
    );
    let mut mapped = content.clone();
    let width = region.right - region.left;
    let height = region.bottom - region.top;
    let source = identity
        .clone()
        .or_else(|| page.script_detection.as_ref().map(|s| s.source.clone()))
        .or_else(|| {
            page.content
                .blocks
                .iter()
                .find_map(|b| b.geometry.as_ref().and_then(|g| g.source.clone()))
        })
        .or_else(|| {
            content
                .blocks
                .iter()
                .find_map(|b| b.geometry.as_ref().and_then(|g| g.source.clone()))
        });
    if let Some(source) = source {
        for block in &mut mapped.blocks {
            if block.bbox.is_none() {
                block.bbox = Some(BoxRect::full());
            }
        }
        crate::geometry::bind_reader_content(&mut mapped, &source)?;
        crate::geometry::map_content(&mut mapped, region, &source)?;
    } else {
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
    if let Some(identity) = identity {
        sanitize_masked_readings(page, &identity, &Control::default())?;
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
    reread_view(project, request, None, settings, c, log)
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineReadRequest {
    pub page: usize,
    pub block: usize,
    pub ordinal: usize,
    pub reader_id: String,
    pub task: String,
    pub image_edge: u32,
    pub context: String,
}
/// Consume the explicitly repaired quadrilateral, including its whitened
/// outside pixels. The response is evidence only and remains undoable.
pub fn reread_line(
    project: &Project,
    request: &LineReadRequest,
    settings: &Settings,
    c: &Control,
    log: &Activity,
) -> Result<u64> {
    RegionRequest {
        page: request.page,
        region: BoxRect::full(),
        reader: request.reader_id.clone(),
        image_edge: request.image_edge,
        context: request.context.clone(),
        task: request.task.clone(),
    }
    .validate(project)?;
    let before = project.load(request.page)?;
    let view =
        crate::geometry_repair::line_view(project, &before, request.block, request.ordinal, c)?;
    let region_request = RegionRequest {
        page: request.page,
        region: view.region.clone(),
        reader: request.reader_id.clone(),
        image_edge: request.image_edge,
        context: request.context.clone(),
        task: request.task.clone(),
    };
    reread_view(
        project,
        &region_request,
        Some((before, view)),
        settings,
        c,
        log,
    )
}
fn reread_view(
    project: &Project,
    request: &RegionRequest,
    line: Option<(Page, crate::geometry_repair::LineView)>,
    settings: &Settings,
    c: &Control,
    log: &Activity,
) -> Result<u64> {
    request.validate(project)?;
    c.check()?;
    let is_line = line.is_some();
    let before = if let Some((before, _)) = &line {
        anyhow::ensure!(
            fingerprint(&project.load(request.page)?)? == fingerprint(before)?,
            Fault::new("NG500", "Page changed before line recognition")
        );
        before.clone()
    } else {
        project.load(request.page)?
    };
    anyhow::ensure!(
        before.candidates.len() < 128,
        Fault::new(
            "NG201",
            "Reader-evidence capacity reached; preserve a project copy before further re-reading"
        )
    );
    let mut requested = settings.clone();
    requested.preset = Preset::Accuracy;
    requested.automatic_models = false;
    requested.model_ids = vec![request.reader.clone()];
    let resolved = crate::project_lock::resolve_for_project(project, &requested, c)?;
    let private = crate::redactions::has_masks(project, c)?;
    let mut s = resolved
        .as_ref()
        .map(|value| value.settings.clone())
        .unwrap_or_else(|| requested.clone());
    if private {
        s = crate::redactions::private_recognition_settings(&s);
    }
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
    let installed = if let Some(locked) = &resolved {
        locked.models.clone()
    } else {
        crate::activation::recover_all(&s.models_dir, c)?;
        runtime::inventory(&s.models_dir)?
    };
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
    anyhow::ensure!(
        !private || !s.external_server && model.endpoint.is_none(),
        Fault::new(
            "NG200",
            "Confirmed masks require a fresh internally owned reader"
        )
    );
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
    let (identity, crop, actual_region) = if let Some((_, view)) = line {
        verify_reader_source(project, &before, &view.source, c)?;
        (view.source, view.image, view.region)
    } else {
        let (identity, image) = crate::redactions::recognition_view(project, &before, c)?;
        let (x, y, r, b) = request.region.pixels(image.width(), image.height());
        let crop = image.crop_imm(x, y, r - x, b - y);
        (identity, crop, pixel_view(&request.region, &image))
    };
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
    if !is_line && s.experience.live_preview {
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
    let mut content = if private && request.task == "table" {
        server.read_table_masked(
            &crop,
            &identity,
            &before.confirmed_redactions,
            Some(&actual_region),
            &s,
            c,
            log,
            request.page,
        )?
    } else if private {
        server.read_masked(
            &crop,
            &identity,
            &before.confirmed_redactions,
            Some(&actual_region),
            &s,
            c,
            log,
            request.page,
        )?
    } else if request.task == "table" {
        server.read_table(&crop, &evidence, &context, &s, c, log, request.page)?
    } else {
        server.read(&crop, &evidence, &context, &s, c, log, request.page)?
    };
    crate::geometry::bind_reader_content(&mut content, &identity)?;
    c.check()?;
    verify_reader_source(project, &before, &identity, c)?;
    let mut after = before.clone();
    attach_region_reading(&mut after, &actual_region, &content, &model)?;
    crate::review::commit(
        project,
        &before,
        &after,
        &format!(
            "{} re-read: {}",
            if is_line { "Repaired line" } else { "Region" },
            model.id
        ),
    )
}

#[cfg(test)]
mod preparation_tests {
    use super::*;
    #[test]
    fn overlapping_preparation_yields_after_publication_and_joins_both_stages() {
        let c = Control::default();
        let root = tempfile::tempdir().unwrap();
        let saved = root.path().join("durable.json");
        let error = bounded_preparation(
            &c,
            |_, send| {
                for n in 0..100 {
                    send(n)?;
                }
                Ok(())
            },
            |n: usize, control| {
                atomic_json(&saved, &n)?;
                c.yield_requested.store(true, Ordering::Release);
                control.checkpoint()
            },
        )
        .unwrap_err();
        assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG901");
        assert_eq!(read_json::<usize>(&saved, 100).unwrap(), 0);
        assert!(!c.cancelled.load(Ordering::Relaxed));
    }
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
