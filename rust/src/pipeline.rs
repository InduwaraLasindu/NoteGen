use crate::{
    consensus, hardware,
    planning::{self, Plan},
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
    anyhow::ensure!(
        !output.exists(),
        Fault::new("NG401", "Output already exists")
    );
    anyhow::ensure!(
        source
            .extension()
            .is_some_and(|v| v.eq_ignore_ascii_case("pdf")),
        Fault::new("NG100", "Choose a PDF input")
    );
    let hw = hardware::detect_for(s, c);
    let mut installed = runtime::inventory(&s.models_dir)?;
    for model in &mut installed {
        c.check()?;
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
    }
    let mut plan = planning::build(&hw, s, &installed)?;
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
    let input = hash_file(source, c).map_err(|e| if e.downcast_ref::<Fault>().is_some() {e} else {Fault::new("NG100",format!("Cannot read input PDF: {e}")).into()})?;
    let key = fingerprint(&(
        2,
        recognition_settings(s),
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
    scan::render_pdf(
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
            p.source_warnings=p.content.warnings.clone();
            atomic_json(&checkpoint, &p)?;
            rendered.push(n);
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
    let total = selected.len() * (plan.models.len() + 1) + 1;
    let mut done = selected.len();
    for (model_index, model) in plan.models.clone().iter().enumerate() {
        c.check()?;
        let tasks = selected
            .iter()
            .copied()
            .filter(|n| project.load(*n).ok().is_none_or(|p| should_read(&model.id,&model.role,&p,s)))
            .collect::<Vec<_>>();
        if tasks.is_empty() {
            done += selected.len();
            continue;
        }
        // Re-admit against current free memory before allocating model weights.
        let current = hardware::detect_for(s, c);
        let fresh = planning::build(&current, s, std::slice::from_ref(model))?;
        if fresh.models.is_empty() {
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
            let mut recovery = plan.clone();
            recovery.context = 12288;
            recovery.image_tokens = 1536;
            recovery.micro_batch = 32;
            recovery.gpu_layers = if plan.backend == Backend::Cpu { 0 } else { 16 };
            active_settings.projector_on_cpu = true;
            active_settings.context=recovery.context;
            active_settings.image_tokens=recovery.image_tokens;
            active_settings.max_tokens=active_settings.max_tokens.min(recovery.context.saturating_sub(recovery.image_tokens+6000).max(512));
            server = Server::start(model, &recovery, &active_settings, c, &log, &root);
        }
        let server = match server {
            Ok(v) => v,
            Err(e) => {
                if e.downcast_ref::<Fault>().is_some_and(|f| f.code == "NG900") {
                    return Err(e);
                }
                log.emit(
                    "model",
                    format!(
                        "{} could not run; candidate absence will require review",
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
        for n in &selected {
            c.check()?;
            let mut page = project.load(*n)?;
            if !tasks.contains(n) {
                done += 1;
                continue;
            }
            let started = Instant::now();
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
            let previous = selected
                .iter()
                .position(|p| p == n)
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| project.load(selected[i]).ok())
                .map(|p| p.content.summary)
                .unwrap_or_default();
            let context = format!("{}\nPrevious selected page: {}", s.subject, previous);
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
            let (mut content, agreement) =
                consensus::select(&page.candidates, s.consensus_min, s.agreement_threshold);
            // Low-confidence source crops provide high effective DPI without sending a huge whole-page image.
            if s.preset == Preset::Maximum && (agreement < s.agreement_threshold || s.handwriting) {
                let regions = scan::crop_regions(&content, &im, s);
                for (i, (bbox, crop)) in regions.into_iter().enumerate() {
                    c.check()?;
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
                    let reading = if table_crop {
                        server.read_table(&crop, "", &context, &active_settings, c, &log, *n)
                    } else {
                        server.read(&crop, "", &context, &active_settings, c, &log, *n)
                    };
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
    atomic_json(&root.join("hardware.json"), &hw)?;
    atomic_json(&root.join("plan.json"), &plan)?;
    let mut review = 0;
    for n in &selected {
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
        if !p.edited && !p.reviewed {project.save(&p)?;}
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
    word::export(&project, output, c)?;
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
    value.font = Settings::default().font;
    value.font_size = 11.;
    value.keep_page_breaks = true;
    value.preserve_figures = true;
    value
}

/// Balanced skips a vision pass only after independent full-page agreement.
pub fn should_read(id:&str, role:&str, page:&Page,s:&Settings)->bool {
    if page.edited || page.reviewed || page.candidates.iter().any(|v|v.model_id==id && v.view=="full") {return false;}
    !(s.preset==Preset::Balanced && role=="vision" && !s.handwriting && page.agreement>=s.agreement_threshold
        && !page.content.blocks.iter().any(|b|b.kind=="figure" || b.uncertain || b.plain().contains("[illegible]")))
}

pub fn retain_source_warnings(source:&[String],content:&mut PageContent) {
    for warning in source {if !content.warnings.contains(warning){content.warnings.push(warning.clone());}}
}

pub fn merge_contextual_figures(target:&mut PageContent,incoming:&PageContent) {
    for figure in incoming.blocks.iter().filter(|b|b.kind=="figure") {
        target.blocks.retain(|old|old.kind!="figure" || old.bbox.as_ref().zip(figure.bbox.as_ref())
            .is_none_or(|(a,b)|consensus::overlap(a,b)<0.35));
        target.blocks.push(figure.clone());
    }
}
