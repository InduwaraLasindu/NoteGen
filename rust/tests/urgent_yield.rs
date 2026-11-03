use notegen::{
    pipeline,
    storage::{Activity, Control},
    types::*,
};
use std::{
    fs,
    sync::{Arc, Mutex, atomic::Ordering},
};

#[test]
fn real_ocr_yields_after_a_durable_page_and_resumes_without_rewriting_it() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("scans");
    fs::create_dir(&input).unwrap();
    for n in 1..=3 {
        image::RgbImage::from_pixel(160, 120, image::Rgb([255, 255, 255]))
            .save(input.join(format!("{n}.png")))
            .unwrap();
    }
    let settings = Settings {
        preset: Preset::Fast,
        backend: Backend::Cpu,
        models_dir: root.path().join("empty-models"),
        auto_rotate: false,
        deskew: false,
        cpu_threads: 2,
        ..Default::default()
    };
    let output = root.path().join("export.docx");
    let c = Control::default();
    let flag = c.yield_requested.clone();
    let log = Activity {
        callback: Some(Arc::new(move |event| {
            if event.stage == "ocr" && event.message == "Printed OCR checkpoint saved" {
                flag.store(true, Ordering::Release);
            }
        })),
        ..Default::default()
    };
    let error = pipeline::convert(&input, &output, &settings, &c, &log).unwrap_err();
    assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG901");
    assert!(!output.exists());
    assert!(!c.cancelled.load(Ordering::Relaxed));
    let cache = fs::read_dir(root.path().join(".notegen"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.is_dir())
        .unwrap();
    let checkpoint = cache.join("page-00001.json");
    let original = fs::read(&checkpoint).unwrap();
    let raster = fs::read(cache.join("page-00001.png")).unwrap();
    assert!(!cache.join("page-00002.json").exists());
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = events.clone();
    let log = Activity {
        callback: Some(Arc::new(move |event| observed.lock().unwrap().push(event))),
        ..Default::default()
    };
    c.yield_requested.store(false, Ordering::Release);
    let completed = pipeline::convert(&input, &output, &settings, &c, &log).unwrap();
    assert_eq!(completed.pages, 3);
    assert!(output.is_file());
    assert_eq!(fs::read(completed.project.image(1)).unwrap(), raster);
    // Finalization may add review warnings, but the original reader candidate is reused.
    let before: Page = serde_json::from_slice(&original).unwrap();
    let after = completed.project.load(1).unwrap();
    assert_eq!(
        serde_json::to_value(&before.candidates).unwrap(),
        serde_json::to_value(&after.candidates).unwrap()
    );
    assert!(events.lock().unwrap().iter().any(|e| e.stage == "cached"
        && e.page == 1
        && e.message == "Found rendered page checkpoint"));
}
