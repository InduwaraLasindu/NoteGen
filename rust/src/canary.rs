//! A bounded known-fixture compatibility gate, not a model accuracy benchmark.
use crate::{
    hardware,
    planning::{self, Model},
    runtime,
    storage::{Activity, Control, fingerprint},
    types::*,
    watch::no_links,
};
use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
use image::{DynamicImage, Rgb, RgbImage};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Cursor,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub const LIMITS: &str = "A small synthetic printed-number and 2x2 editable-table compatibility check runs before activation. Passing does not establish general recognition accuracy, handwriting/language support, numerical reliability on other scans or GPU performance. Parser/source uncertainty remains reviewable. Runtime identity covers the executable and adjacent native libraries, not operating-system libraries, drivers or hardware.";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeIdentity {
    pub server: String,
    pub files: Vec<RuntimeFile>,
    pub fingerprint: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Criteria {
    pub printed_number: bool,
    pub exact_table: bool,
    pub table_uncertain: bool,
}
impl Criteria {
    pub fn passed(&self) -> bool {
        self.printed_number && self.exact_table
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub schema: u32,
    pub model_id: String,
    pub model_fingerprint: String,
    pub runtime: RuntimeIdentity,
    pub printed_fixture_sha256: String,
    pub table_fixture_sha256: String,
    pub fixture_font_sha256: String,
    pub criteria: Criteria,
    pub seconds: f64,
    pub completed: String,
    pub note: String,
    pub evidence_directory: std::path::PathBuf,
}
/// Cannot be deserialized or constructed outside this module. Saved reports are
/// inspectable evidence; they are never accepted as permission to activate.
pub struct Passed {
    report: Report,
}
impl Passed {
    pub fn report(&self) -> &Report {
        &self.report
    }
    pub fn verify_current(&self, model: &Model, server: &Path, c: &Control) -> Result<()> {
        anyhow::ensure!(
            self.report.criteria.passed()
                && self.report.model_fingerprint == model_identity(model, c)?
                && self.report.runtime == runtime_identity(server, c)?,
            fault("Canary candidate changed before activation")
        );
        Ok(())
    }
}
fn fault(message: &str) -> anyhow::Error {
    Fault::new("NG660", message).into()
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 240
        && name != "."
        && name != ".."
        && !name
            .chars()
            .any(|ch| ch.is_control() || ch == '/' || ch == '\\' || ch == ':')
}
pub fn validate_runtime_identity(identity: &RuntimeIdentity) -> Result<()> {
    anyhow::ensure!(
        valid_name(&identity.server) && !identity.files.is_empty() && identity.files.len() <= 256,
        fault("Invalid pinned runtime inventory")
    );
    let mut names = std::collections::BTreeSet::new();
    let mut total = 0u64;
    for file in &identity.files {
        total = total
            .checked_add(file.bytes)
            .ok_or_else(|| fault("Runtime inventory size overflow"))?;
        anyhow::ensure!(
            valid_name(&file.name)
                && names.insert(file.name.to_ascii_lowercase())
                && (1..=2_000_000_000).contains(&file.bytes)
                && file.sha256.len() == 64
                && file
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            fault("Invalid pinned runtime file identity")
        );
    }
    anyhow::ensure!(
        total <= 4_000_000_000,
        fault("Runtime inventory exceeds the 4 GB budget")
    );
    anyhow::ensure!(
        identity
            .files
            .iter()
            .any(|file| file.name == identity.server)
            && fingerprint(&(&identity.server, &identity.files))? == identity.fingerprint,
        fault("Runtime inventory fingerprint is inconsistent")
    );
    Ok(())
}
pub fn runtime_identity(server: &Path, c: &Control) -> Result<RuntimeIdentity> {
    c.check()?;
    no_links(server)?;
    anyhow::ensure!(
        server.is_file(),
        fault("Candidate runtime executable is missing")
    );
    let folder = server
        .parent()
        .ok_or_else(|| fault("Select a runtime in a native runtime folder"))?;
    let name = server
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| fault("Invalid runtime executable filename"))?
        .to_string();
    anyhow::ensure!(
        valid_name(&name),
        fault("Invalid runtime executable filename")
    );
    let mut files = vec![];
    let mut visited = 0;
    let mut total = 0u64;
    for entry in fs::read_dir(folder)? {
        c.check()?;
        visited += 1;
        anyhow::ensure!(
            visited <= 10000,
            fault("Runtime folder inventory exceeds its bound")
        );
        let entry = entry?;
        let path = entry.path();
        let filename = entry.file_name().to_string_lossy().to_string();
        let lower = filename.to_ascii_lowercase();
        if filename == name
            || lower.ends_with(".dll")
            || lower.ends_with(".so")
            || lower.contains(".so.")
            || lower.ends_with(".dylib")
        {
            no_links(&path)?;
            let metadata = fs::metadata(&path)?;
            total = total
                .checked_add(metadata.len())
                .ok_or_else(|| fault("Runtime inventory size overflow"))?;
            anyhow::ensure!(
                metadata.is_file()
                    && metadata.len() > 0
                    && metadata.len() <= 2_000_000_000
                    && files.len() < 256,
                fault("Runtime file is not a bounded regular native asset")
            );
            anyhow::ensure!(
                total <= 4_000_000_000,
                fault("Runtime inventory exceeds the 4 GB budget")
            );
            let sha256 = crate::model_store::inspect(
                &path,
                2_000_000_000,
                crate::model_store::Kind::RuntimeFile,
                c,
            )?
            .sha256;
            anyhow::ensure!(
                fs::metadata(&path)?.len() == metadata.len(),
                fault("Runtime changed during inventory")
            );
            files.push(RuntimeFile {
                name: filename,
                bytes: metadata.len(),
                sha256,
            });
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    let result = RuntimeIdentity {
        fingerprint: fingerprint(&(&name, &files))?,
        server: name,
        files,
    };
    validate_runtime_identity(&result)?;
    Ok(result)
}
pub fn model_identity(model: &Model, c: &Control) -> Result<String> {
    crate::models::validate_model(model)?;
    anyhow::ensure!(
        model.endpoint.is_none(),
        fault("External HTTP readers cannot satisfy a local activation canary")
    );
    runtime::verify_gguf(&model.weights)?;
    runtime::verify_gguf(&model.projector)?;
    no_links(&model.weights)?;
    no_links(&model.projector)?;
    let weights = crate::model_store::inspect(
        &model.weights,
        100_000_000_000,
        crate::model_store::Kind::Gguf,
        c,
    )?
    .sha256;
    let projector = crate::model_store::inspect(
        &model.projector,
        100_000_000_000,
        crate::model_store::Kind::Gguf,
        c,
    )?
    .sha256;
    anyhow::ensure!(
        model.sha256 == fingerprint(&(&weights, &projector))?,
        fault("Candidate reader object pair failed SHA256 verification")
    );
    fingerprint(&(model, weights, projector))
}
pub fn evaluate(printed: &PageContent, table: &PageContent) -> Criteria {
    if printed.validate().is_err() || table.validate().is_err() {
        return Criteria::default();
    }
    let number = printed
        .plain()
        .split(|ch: char| !ch.is_alphanumeric() && ch != '.')
        .any(|word| word == "1250");
    let expected = [vec!["Item", "Amount"], vec!["Sales", "1250"]];
    let matched = table.blocks.iter().find(|block| {
        block.kind == "table"
            && block.rows.len() == 2
            && block.rows.iter().zip(&expected).all(|(actual, expected)| {
                actual.len() == 2
                    && actual
                        .iter()
                        .zip(expected)
                        .all(|(a, b)| crate::consensus::normalize(a) == *b)
            })
    });
    Criteria {
        printed_number: number,
        exact_table: matched.is_some_and(|block| block.merges.is_empty()),
        table_uncertain: matched.is_some_and(|block| block.uncertain),
    }
}
struct Fixtures {
    printed: DynamicImage,
    table: DynamicImage,
    font_sha256: String,
}
fn fixtures() -> Result<Fixtures> {
    let font_bytes = [
        "C:/Windows/Fonts/arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf",
    ]
    .iter()
    .find_map(|path| fs::read(path).ok())
    .ok_or_else(|| {
        fault("A system TrueType font is required to render the known activation canary")
    })?;
    use sha2::Digest;
    let font_sha256 = hex::encode(sha2::Sha256::digest(&font_bytes));
    let font = FontArc::try_from_vec(font_bytes)?;
    fn text(image: &mut RgbImage, font: &FontArc, x: f32, y: f32, text: &str) {
        let scaled = font.as_scaled(PxScale::from(42.));
        let mut x = x;
        for ch in text.chars() {
            let id = font.glyph_id(ch);
            if let Some(glyph) = font.outline_glyph(id.with_scale_and_position(42., point(x, y))) {
                let bounds = glyph.px_bounds();
                glyph.draw(|gx, gy, coverage| {
                    let px = gx as i32 + bounds.min.x as i32;
                    let py = gy as i32 + bounds.min.y as i32;
                    if (0..image.width() as i32).contains(&px)
                        && (0..image.height() as i32).contains(&py)
                    {
                        let value = (255. * (1. - coverage)) as u8;
                        image.put_pixel(px as u32, py as u32, Rgb([value; 3]));
                    }
                });
            }
            x += scaled.h_advance(id);
        }
    }
    let mut printed = RgbImage::from_pixel(900, 180, Rgb([255; 3]));
    text(&mut printed, &font, 25., 60., "NoteGen canary");
    text(
        &mut printed,
        &font,
        25.,
        125.,
        "Revenue is 1250 in October.",
    );
    let mut table = RgbImage::from_pixel(900, 270, Rgb([255; 3]));
    for y in [15, 135, 255] {
        for dy in 0..3 {
            for x in 15..885 {
                table[(x, y + dy)] = Rgb([0; 3]);
            }
        }
    }
    for x in [15, 450, 885] {
        for dx in 0..3 {
            for y in 15..258 {
                table[(x + dx, y)] = Rgb([0; 3]);
            }
        }
    }
    for (x, y, value) in [
        (40., 90., "Item"),
        (475., 90., "Amount"),
        (40., 210., "Sales"),
        (475., 210., "1250"),
    ] {
        text(&mut table, &font, x, y, value);
    }
    Ok(Fixtures {
        printed: DynamicImage::ImageRgb8(printed),
        table: DynamicImage::ImageRgb8(table),
        font_sha256,
    })
}
fn image_hash(image: &DynamicImage) -> Result<String> {
    use sha2::Digest;
    let mut bytes = Cursor::new(vec![]);
    image.write_to(&mut bytes, image::ImageFormat::Png)?;
    Ok(hex::encode(sha2::Sha256::digest(bytes.into_inner())))
}
struct Watchdog {
    done: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Watchdog {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
/// Cold, real inference against only synthetic pixels. No user context, writer
/// memory, external server, lookup, clarifiers or previous transcript is used.
pub fn run(
    model: &Model,
    settings: &Settings,
    evidence: &Path,
    c: &Control,
    log: &Activity,
) -> Result<Passed> {
    c.check()?;
    let model_fingerprint = model_identity(model, c)?;
    let runtime = runtime_identity(&settings.server, c)?;
    let fixtures = fixtures()?;
    let printed_fixture_sha256 = image_hash(&fixtures.printed)?;
    let table_fixture_sha256 = image_hash(&fixtures.table)?;
    no_links(evidence)?;
    fs::create_dir_all(evidence)?;
    let evidence = tempfile::Builder::new()
        .prefix("canary-")
        .tempdir_in(evidence)?
        .keep();
    let evidence = evidence.as_path();
    let mut safe = settings.clone();
    safe.external_server = false;
    safe.subject.clear();
    safe.assistance = Default::default();
    safe.assistance.ask_questions = false;
    safe.assistance.web_lookup = false;
    safe.writer.use_examples = false;
    safe.writer_reference = None;
    safe.experience.warm_reader = false;
    safe.experience.live_preview = false;
    safe.experience.check_updates = false;
    safe.automatic_models = false;
    safe.model_ids = vec![model.id.clone()];
    safe.image_edge = 768;
    safe.image_tokens = 1024;
    safe.context = 8192;
    safe.max_tokens = 2048;
    safe.request_timeout_secs = 120;
    safe.preset = Preset::Accuracy;
    let probe = Control {
        cancelled: Arc::new(AtomicBool::new(false)),
        clarify: None,
        ..c.clone()
    };
    let done = Arc::new(AtomicBool::new(false));
    let stop = done.clone();
    let outer = c.clone();
    let child = probe.clone();
    let watchdog = thread::spawn(move || {
        let start = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            if start.elapsed() > Duration::from_secs(300) || outer.check_cancelled().is_err() {
                child.cancelled.store(true, Ordering::Relaxed);
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
    });
    let _guard = Watchdog {
        done,
        thread: Some(watchdog),
    };
    let plan = planning::build(
        &hardware::detect_for(&safe, &probe),
        &safe,
        std::slice::from_ref(model),
    )?;
    anyhow::ensure!(
        plan.models.len() == 1,
        fault("Candidate does not fit available resources for the activation canary")
    );
    let started = Instant::now();
    let activity = Activity {
        path: Some(evidence.join("activity.jsonl")),
        callback: log.callback.clone(),
    };
    let server = runtime::Server::start(model, &plan, &safe, &probe, &activity, evidence)?;
    let printed = server.read(
        &fixtures.printed,
        "",
        "Synthetic compatibility fixture; transcribe visible content faithfully.",
        &safe,
        &probe,
        &activity,
        1,
    )?;
    let table = server.read_table(
        &fixtures.table,
        "",
        "Synthetic table compatibility fixture; transcribe visible content faithfully.",
        &safe,
        &probe,
        &activity,
        2,
    )?;
    drop(server);
    probe.check()?;
    c.check()?;
    anyhow::ensure!(
        fs::read_dir(evidence.join("responses"))?.take(3).count() >= 2,
        fault("Canary raw response evidence was not fully retained")
    );
    let criteria = evaluate(&printed, &table);
    let report = Report {
        schema: 1,
        model_id: model.id.clone(),
        model_fingerprint,
        runtime,
        printed_fixture_sha256,
        table_fixture_sha256,
        fixture_font_sha256: fixtures.font_sha256,
        criteria,
        seconds: started.elapsed().as_secs_f64(),
        completed: chrono::Utc::now().to_rfc3339(),
        note: LIMITS.into(),
        evidence_directory: evidence.to_path_buf(),
    };
    crate::storage::atomic_json(&evidence.join("canary-report.json"), &report)?;
    crate::storage::atomic_json(&evidence.join("printed.json"), &printed)?;
    crate::storage::atomic_json(&evidence.join("table.json"), &table)?;
    anyhow::ensure!(
        report.criteria.passed(),
        fault(
            "Candidate failed the known printed-number/table canary; working installation was not activated"
        )
    );
    let passed = Passed { report };
    passed.verify_current(model, &settings.server, c)?;
    Ok(passed)
}
