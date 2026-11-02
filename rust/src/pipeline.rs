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
    time::Instant,
};
#[derive(Clone, Debug)]
pub struct Completed {
    pub project: Project,
    pub output: PathBuf,
    pub pages: usize,
    pub review: usize,
    pub seconds: f64,
    pub plan: Plan,
}
pub fn convert(
    source: &Path,
    output: &Path,
    s: &Settings,
    c: &Control,
    activity: &Activity,
) -> Result<Completed> {
    let begun = Instant::now();
    s.validate()?;
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
    let root = parent.join(".notegen").join(format!(
        "{}-{}-{}",
        source.file_stem().unwrap_or_default().to_string_lossy(),
        &input[..10],
        &key[..10]
    ));
    fs::create_dir_all(&root)?;
    let log = Activity {
        path: Some(root.join("activity.jsonl")),
        callback: activity.callback.clone(),
    };
    let mut selected = vec![];
    let mut rendered = vec![];
    // PDFium stays in this one worker. Stage results live on disk instead of a document-sized RAM graph.
    crate::inputs::render(
        source,
        &s.pdfium,
        &s.page_range,
        plan.dpi,
        s.max_pixels,
        c,
        |n, im| {
            let image = root.join(format!("page-{n:05}.png"));
            let checkpoint = root.join(format!("page-{n:05}.json"));
            selected.push(n);
            log.work(
                Work::new(Phase::Render, selected.len() - 1, pages, 1500, true),
                "Preparing source pages",
            );
            if image.exists()
                && fs::File::open(&checkpoint)
                    .ok()
                    .and_then(|f| serde_json::from_reader::<_, Page>(f).ok())
                    .is_some_and(|p| {
                        p.page == n
                            && p.image == format!("page-{n:05}.png")
                            && p.content.validate().is_ok()
                    })
            {
                log.emit(
                    "cached",
                    "Found rendered page checkpoint",
                    n,
                    selected.len(),
                    0,
                    "",
                );
                log.work(
                    Work::new(Phase::Render, selected.len(), pages, 1500, false),
                    "Reused rendered checkpoint",
                );
                return Ok(());
            }
            c.check()?;
            log.emit(
                "render",
                "Preparing source scan and printed OCR",
                n,
                selected.len() - 1,
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
                    if plan.models.is_empty() {
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
            rendered.push(n);
            log.work(
                Work::new(Phase::Render, selected.len(), pages, 1500, false),
                "Source page prepared",
            );
            log.emit(
                "ocr",
                "Printed OCR checkpoint saved",
                n,
                selected.len(),
                0,
                "",
            );
            Ok(())
        },
    )?;
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
    Ok(Completed {
        project,
        output: output.into(),
        pages: selected.len(),
        review,
        seconds: begun.elapsed().as_secs_f64(),
        plan,
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
        let mut page = project.load(n)?;
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
        attach_context_reading(&mut page, &content, &model);
        project.save(&page)?;
        count += 1;
    }
    Ok(count)
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
