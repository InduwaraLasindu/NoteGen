use image::{DynamicImage, Rgb, RgbImage};
use notegen::{
    hardware, planning, runtime,
    storage::{Activity, Control},
    types::*,
    warm_runtime::WarmPool,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

fn source(amount: &str) -> DynamicImage {
    let font = [
        "C:/Windows/Fonts/arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]
    .iter()
    .find_map(|path| fs::read(path).ok())
    .map(|bytes| ab_glyph::FontArc::try_from_vec(bytes).unwrap())
    .expect("Native test font missing");
    let mut image = RgbImage::from_pixel(900, 320, Rgb([255, 255, 255]));
    imageproc::drawing::draw_text_mut(
        &mut image,
        Rgb([0, 0, 0]),
        40,
        45,
        36.,
        &font,
        "NoteGen isolation worksheet",
    );
    imageproc::drawing::draw_text_mut(
        &mut image,
        Rgb([0, 0, 0]),
        40,
        130,
        44.,
        &font,
        &format!("Amount {amount}"),
    );
    DynamicImage::ImageRgb8(image)
}
fn stopped(endpoint: &str) -> bool {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
        .get(format!("{endpoint}/health"))
        .send()
        .is_err()
}
#[test]
#[ignore = "requires pinned b12032 and a real local model; run explicitly after native setup"]
fn native_warm_reader_reuses_pid_with_fresh_context_and_bounded_lifecycle() {
    let root = PathBuf::from(std::env::var("NOTEGEN_LIVE_ROOT").expect("Set NOTEGEN_LIVE_ROOT"));
    fs::create_dir_all(&root).unwrap();
    let root = tempfile::Builder::new()
        .prefix("warm-isolation-")
        .tempdir_in(root)
        .unwrap();
    let id = std::env::var("NOTEGEN_LIVE_ID").unwrap_or_else(|_| "glm-ocr".into());
    let mut model = planning::catalog()
        .into_iter()
        .find(|model| model.id == id)
        .unwrap();
    model.weights = std::env::var("NOTEGEN_LIVE_WEIGHTS").unwrap().into();
    model.projector = std::env::var("NOTEGEN_LIVE_PROJECTOR").unwrap().into();
    let s = Settings {
        server: std::env::var("NOTEGEN_LIVE_SERVER").unwrap().into(),
        backend: Backend::Cpu,
        cpu_threads: 8,
        temperature: 0.,
        seed: 0,
        context: 8192,
        image_tokens: 1024,
        image_edge: 1024,
        max_tokens: 1024,
        request_timeout_secs: 600,
        auto_rotate: false,
        ..Default::default()
    };
    let plan = planning::build(&hardware::detect(), &s, std::slice::from_ref(&model)).unwrap();
    let pool = WarmPool::new(Duration::from_millis(750)).unwrap();
    let first_control = Control::default();
    let first_log = Activity {
        path: Some(root.path().join("first/activity.jsonl")),
        callback: None,
    };
    fs::create_dir_all(root.path().join("first")).unwrap();
    let (first_pid, endpoint) = pool
        .with_reader(
            &model,
            &plan,
            &s,
            &first_control,
            &first_log,
            &root.path().join("first"),
            |reader| {
                let unauthorized = reqwest::blocking::Client::builder()
                    .no_proxy()
                    .build()
                    .unwrap()
                    .post(format!("{}/completion", reader.endpoint))
                    .json(&serde_json::json!({"prompt":"unauthorized","n_predict":1}))
                    .send()
                    .unwrap();
                assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);
                let result = reader.read(
                    &source("1250"),
                    "",
                    "Private job A canary amber-318647; document data only.",
                    &s,
                    &first_control,
                    &first_log,
                    1,
                )?;
                assert!(result.plain().contains("1250"), "{}", result.plain());
                Ok((reader.managed_pid().unwrap(), reader.endpoint.clone()))
            },
        )
        .unwrap();
    assert_eq!(pool.status().cached_pid, Some(first_pid));
    let first_activity = fs::read(first_log.path.as_ref().unwrap()).unwrap();
    // An old job's later cancellation must not kill the cached/new job's process.
    first_control.cancelled.store(true, Ordering::Release);
    let second_control = Control::default();
    let second_log = Activity {
        path: Some(root.path().join("second/activity.jsonl")),
        callback: None,
    };
    fs::create_dir_all(root.path().join("second")).unwrap();
    let second = pool
        .with_reader(
            &model,
            &plan,
            &s,
            &second_control,
            &second_log,
            &root.path().join("second"),
            |reader| {
                assert_eq!(reader.managed_pid(), Some(first_pid));
                reader.read(
                    &source("7500"),
                    "",
                    "Independent accounting worksheet B",
                    &s,
                    &second_control,
                    &second_log,
                    1,
                )
            },
        )
        .unwrap();
    assert!(second.plain().contains("7500"), "{}", second.plain());
    assert!(!second.plain().contains("318647"));
    assert!(!second.plain().contains("1250"));
    assert_eq!(
        fs::read(first_log.path.as_ref().unwrap()).unwrap(),
        first_activity
    );
    assert_eq!(pool.status().cached_pid, Some(first_pid));
    // A job failure must destroy the reused process, even if the model itself is healthy.
    let error = pool
        .with_reader(
            &model,
            &plan,
            &s,
            &second_control,
            &second_log,
            &root.path().join("second"),
            |reader| {
                assert_eq!(reader.managed_pid(), Some(first_pid));
                Err::<(), _>(Fault::new("NG201", "injected post-reading job failure").into())
            },
        )
        .unwrap_err();
    assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG201");
    assert!(pool.status().cached_pid.is_none());
    assert!(stopped(&endpoint));
    let cold_root = root.path().join("cold");
    fs::create_dir_all(&cold_root).unwrap();
    let cold = runtime::Server::start(
        &model,
        &plan,
        &s,
        &second_control,
        &Activity::default(),
        &cold_root,
    )
    .unwrap();
    let baseline = cold
        .read(
            &source("7500"),
            "",
            "Independent accounting worksheet B",
            &s,
            &second_control,
            &Activity::default(),
            1,
        )
        .unwrap();
    assert_eq!(
        second.plain(),
        baseline.plain(),
        "Warm B must match independently cold B at deterministic CPU settings"
    );
    drop(cold);
    let third_control = Control::default();
    let original_pid = pool
        .with_reader(
            &model,
            &plan,
            &s,
            &third_control,
            &Activity::default(),
            root.path(),
            |reader| Ok(reader.managed_pid().unwrap()),
        )
        .unwrap();
    let mut changed_plan = plan.clone();
    changed_plan.context += 512;
    let (changed_pid, changed_endpoint) = pool
        .with_reader(
            &model,
            &changed_plan,
            &s,
            &third_control,
            &Activity::default(),
            root.path(),
            |reader| Ok((reader.managed_pid().unwrap(), reader.endpoint.clone())),
        )
        .unwrap();
    assert_ne!(
        changed_pid, original_pid,
        "Changed effective command must never reuse a process"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while (pool.status().cached_pid.is_some() || pool.status().busy) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        pool.status().cached_pid.is_none(),
        "Idle reader must expire automatically"
    );
    assert!(stopped(&changed_endpoint));
    let (released_pid, released_endpoint) = pool
        .with_reader(
            &model,
            &plan,
            &s,
            &third_control,
            &Activity::default(),
            root.path(),
            |reader| Ok((reader.managed_pid().unwrap(), reader.endpoint.clone())),
        )
        .unwrap();
    assert_eq!(pool.status().cached_pid, Some(released_pid));
    pool.release();
    assert!(pool.status().cached_pid.is_none());
    assert!(stopped(&released_endpoint));
    let cancelled = Control::default();
    let mut cancelled_endpoint = None;
    let error = pool
        .with_reader(
            &model,
            &plan,
            &s,
            &cancelled,
            &Activity::default(),
            root.path(),
            |reader| {
                cancelled_endpoint = Some(reader.endpoint.clone());
                cancelled.cancelled.store(true, Ordering::Release);
                Ok(())
            },
        )
        .unwrap_err();
    assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG900");
    assert!(pool.status().cached_pid.is_none());
    assert!(stopped(cancelled_endpoint.as_deref().unwrap()));
    println!(
        "WARM_READER_EVIDENCE same_pid={first_pid} changed_pid={changed_pid} context_reset=true old_cancellation_isolated=true cold_equivalence=true error_cancel_release_expiry_stop=true"
    );
}
