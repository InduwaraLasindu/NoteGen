//! Bounded, repeatable synthetic probes. No user documents are used for tuning.
use crate::{
    hardware, planning,
    progress::{Phase, Work},
    runtime,
    storage::*,
    types::*,
};
use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

struct TemporaryPreview<'a> {
    log: &'a Activity,
    shown: bool,
}
impl<'a> TemporaryPreview<'a> {
    fn new(log: &'a Activity) -> Self {
        Self { log, shown: false }
    }
    fn show(&mut self, source: crate::telemetry::Preview) {
        self.shown = true;
        self.log.preview(source);
    }
}
impl Drop for TemporaryPreview<'_> {
    fn drop(&mut self) {
        if self.shown {
            self.log.preview(crate::telemetry::Preview::clear());
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trial {
    pub threads: usize,
    pub milliseconds: f64,
    pub correct: bool,
    pub gpu_verified: bool,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recommendation {
    pub cpu_threads: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub hardware: hardware::Hardware,
    pub cpu: Vec<Trial>,
    pub ai: Vec<Trial>,
    pub proposed: Recommendation,
    pub notes: Vec<String>,
    pub measured_ai: bool,
}
pub fn candidates(h: &hardware::Hardware) -> Vec<usize> {
    let max = h.logical_cores.clamp(1, 64);
    let mut values = vec![
        (h.physical_cores / 2).clamp(1, max),
        h.physical_cores.clamp(1, max),
        max.saturating_sub(1).max(1),
    ];
    values.sort_unstable();
    values.dedup();
    values
}
pub fn best(trials: &[Trial]) -> Option<&Trial> {
    trials
        .iter()
        .filter(|t| t.correct && t.milliseconds.is_finite() && t.milliseconds > 0.)
        .min_by(|a, b| a.milliseconds.total_cmp(&b.milliseconds))
}
fn sample() -> Result<image::DynamicImage> {
    let font = [
        "C:/Windows/Fonts/arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf",
    ]
    .iter()
    .find_map(|p| fs::read(p).ok())
    .ok_or_else(|| {
        anyhow::anyhow!("A system TrueType font is needed for the synthetic recognition test")
    })?;
    let font = FontArc::try_from_vec(font)?;
    let scaled = font.as_scaled(PxScale::from(42.));
    let mut im = image::RgbaImage::from_pixel(1000, 200, image::Rgba([255, 255, 255, 255]));
    for (line, text) in ["NoteGen hardware check", "Total 1250"].iter().enumerate() {
        let mut x = 35.;
        for ch in text.chars() {
            let id = font.glyph_id(ch);
            let glyph = id.with_scale_and_position(42., point(x, 65. + line as f32 * 70.));
            if let Some(g) = font.outline_glyph(glyph) {
                let bounds = g.px_bounds();
                g.draw(|gx, gy, coverage| {
                    let px = gx as i32 + bounds.min.x as i32;
                    let py = gy as i32 + bounds.min.y as i32;
                    if px >= 0 && py >= 0 && px < 1000 && py < 200 {
                        let v = (255. * (1. - coverage)) as u8;
                        im.put_pixel(px as u32, py as u32, image::Rgba([v, v, v, 255]));
                    }
                });
            }
            x += scaled.h_advance(id);
        }
    }
    Ok(image::DynamicImage::ImageRgba8(im))
}
pub fn run(s: &Settings, c: &Control, log: &Activity) -> Result<Report> {
    let h = hardware::detect_for(s, c);
    let mut report = Report {
        hardware: h.clone(),
        cpu: vec![],
        ai: vec![],
        proposed: Recommendation {
            cpu_threads: s.cpu_threads,
        },
        notes: vec![],
        measured_ai: false,
    };
    let choices = candidates(&h);
    let data: Vec<u32> = (0..2_000_000u32)
        .map(|n| n.wrapping_mul(2654435761))
        .collect();
    log.work(
        Work::new(Phase::Read, 0, choices.len() + 2, 20000, true),
        "Testing CPU workers with synthetic pixels",
    );
    for (index, &threads) in choices.iter().enumerate() {
        c.check()?;
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()?;
        let mut times = vec![];
        for _ in 0..3 {
            let start = Instant::now();
            let checksum = pool.install(|| {
                data.par_chunks(4096)
                    .map(|chunk| {
                        chunk.iter().fold(0u64, |sum, &p| {
                            let mut v = p;
                            for _ in 0..32 {
                                v = v
                                    .rotate_left(7)
                                    .wrapping_mul(1664525)
                                    .wrapping_add(1013904223);
                            }
                            sum.wrapping_add(v as u64)
                        })
                    })
                    .reduce(|| 0, u64::wrapping_add)
            });
            std::hint::black_box(checksum);
            times.push(start.elapsed().as_secs_f64() * 1000.);
            c.check()?;
        }
        times.sort_by(f64::total_cmp);
        report.cpu.push(Trial {
            threads,
            milliseconds: times[1],
            correct: true,
            gpu_verified: false,
            detail: "Median of 3 synthetic pixel-processing runs".into(),
        });
        log.work(
            Work::new(Phase::Read, index + 1, choices.len() + 2, 20000, false),
            format!("CPU trial: {threads} workers · {:.0} ms", times[1]),
        );
    }
    if let Some(t) = best(&report.cpu) {
        report.proposed.cpu_threads = t.threads;
    }
    let (mut models, _) = runtime::inventory_available(&s.models_dir)?;
    if let Ok(plan) = planning::build(&h, s, &models) {
        report.notes.push(format!("Adaptive plan for the selected quality: {:?} · {} MiB RAM budget · {} MiB VRAM budget · {} DPI · {} input edge · {} context tokens. Readers: {}.",plan.backend,plan.ram_budget_mib,plan.vram_budget_mib,plan.dpi,plan.image_edge,plan.context,plan.models.iter().map(|m|m.name.as_str()).collect::<Vec<_>>().join(", ")));
    }
    models.retain(|m| m.endpoint.is_none());
    models.sort_by_key(|m| (m.role != "ocr", m.weights_mib));
    let image = sample()?;
    let work = tempfile::tempdir()?;
    // Declared after the temporary directory so its preview is cleared first,
    // including cancellation and error exits.
    let mut preview = TemporaryPreview::new(log);
    let source = work.path().join("hardware-check.png");
    image.save(&source)?;
    let ocr = crate::scan::run(
        &s.tesseract,
        &[
            source.to_string_lossy().into_owned(),
            "stdout".into(),
            "--psm".into(),
            "6".into(),
        ],
        Duration::from_secs(20),
        c,
    );
    report.notes.push(
        if ocr.is_ok_and(|(ok, text, _)| ok && text.contains("1250")) {
            "Printed OCR probe read the exact number 1250.".into()
        } else {
            "Printed OCR probe failed; check Tesseract and language data in Advanced.".into()
        },
    );
    if let Some(m) = models.first().filter(|_| s.server.is_file()) {
        let mut probes = vec![report.proposed.cpu_threads, h.physical_cores.clamp(1, 64)];
        probes.sort_unstable();
        probes.dedup();
        for (index, threads) in probes.into_iter().take(2).enumerate() {
            c.check()?;
            let mut settings = s.clone();
            settings.cpu_threads = threads;
            settings.image_edge = 768;
            settings.context = 8192;
            settings.max_tokens = 256;
            settings.request_timeout_secs = 45;
            settings.automatic_models = false;
            settings.model_ids = vec![m.id.clone()];
            settings.preset = Preset::Accuracy;
            settings.image_tokens = 768;
            if s.experience.live_preview {
                preview.show(crate::telemetry::Preview::source(
                    source.clone(),
                    0,
                    &m.name,
                    None,
                ));
            }
            let probe = Control::default();
            let done = Arc::new(AtomicBool::new(false));
            let stop = done.clone();
            let outer = c.clone();
            let pc = probe.clone();
            let watchdog = std::thread::spawn(move || {
                let started = Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    if started.elapsed() > Duration::from_secs(75)
                        || outer.cancelled.load(Ordering::Relaxed)
                    {
                        pc.cancelled.store(true, Ordering::Relaxed);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            });
            let start = Instant::now();
            let result = (|| -> Result<(f64, bool, bool)> {
                let plan = planning::build(&h, &settings, std::slice::from_ref(m))?;
                anyhow::ensure!(
                    !plan.models.is_empty(),
                    "Selected AI probe does not fit available memory"
                );
                let server = runtime::Server::start(m, &plan, &settings, &probe, log, work.path())?;
                let reading_at = Instant::now();
                let answer = server.read(
                    &image,
                    "",
                    "Synthetic hardware test",
                    &settings,
                    &probe,
                    log,
                    0,
                )?;
                Ok((
                    reading_at.elapsed().as_secs_f64() * 1000.,
                    serde_json::to_string(&answer)?.contains("1250"),
                    server.acceleration_verified,
                ))
            })();
            done.store(true, Ordering::Relaxed);
            let _ = watchdog.join();
            c.check()?;
            let trial = match result {
                Ok((ms, correct, gpu)) => Trial {
                    threads,
                    milliseconds: ms,
                    correct,
                    gpu_verified: gpu,
                    detail: format!(
                        "{} · exact-number recognition · {:.1}s including load",
                        m.name,
                        start.elapsed().as_secs_f64()
                    ),
                },
                Err(e) => Trial {
                    threads,
                    milliseconds: start.elapsed().as_secs_f64() * 1000.,
                    correct: false,
                    gpu_verified: false,
                    detail: format!("Probe did not complete correctly: {e:#}"),
                },
            };
            log.emit(
                "tuning",
                format!("{} threads: {}", threads, trial.detail),
                0,
                0,
                0,
                "",
            );
            report.ai.push(trial);
            log.work(
                Work::new(
                    Phase::Read,
                    choices.len() + index + 1,
                    choices.len() + 2,
                    20000,
                    false,
                ),
                "AI hardware trial complete",
            );
        }
        if let Some(t) = best(&report.ai) {
            report.proposed.cpu_threads = t.threads;
            report.measured_ai = true;
        }
    }
    report.notes.push(if report.measured_ai { "Thread recommendation uses the fastest correct AI trial. Image resolution and quality preset are preserved. Other resources remain adaptive to available RAM/VRAM.".into() } else { "AI performance was not measured successfully. The proposed worker count is based on CPU trials only. Install local AI and re-run to complete the AI test.".into() });
    report.notes.push("Synthetic trials are estimates for this PC's current load; real document speed and accuracy vary. No private scans were used.".into());
    log.work(
        Work::new(
            Phase::Read,
            choices.len() + 2,
            choices.len() + 2,
            20000,
            false,
        ),
        "Hardware test complete; review the proposed settings",
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn temporary_preview_clears_on_early_exit_and_leaves_unshown_views_alone() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = events.clone();
        let log = Activity {
            path: None,
            callback: Some(Arc::new(move |e| {
                if let Some(p) = e.preview {
                    seen.lock().unwrap().push(p);
                }
            })),
        };
        {
            let _unused = TemporaryPreview::new(&log);
        }
        assert!(events.lock().unwrap().is_empty());
        let cancelled = (|| -> Result<()> {
            let mut preview = TemporaryPreview::new(&log);
            preview.show(crate::telemetry::Preview::source(
                "synthetic.png".into(),
                0,
                "probe",
                None,
            ));
            let control = Control::default();
            control.cancelled.store(true, Ordering::Relaxed);
            control.check()?;
            Ok(())
        })();
        assert!(cancelled.is_err());
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert!(events[0].image.is_some());
        assert!(events[1].clear);
    }
}
