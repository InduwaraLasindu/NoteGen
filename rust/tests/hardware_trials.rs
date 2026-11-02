use notegen::{
    hardware::{Gpu, Hardware},
    hardware_trials::{
        self as trials, Device, DeviceRequest, MemorySample, MemorySummary, Options, Trial,
    },
    storage::{Activity, Control, Manifest, Project, fingerprint},
    types::*,
};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
fn hardware() -> Hardware {
    Hardware {
        cpu: "Fixture CPU".into(),
        logical_cores: 8,
        physical_cores: 4,
        ram_mib: 32000,
        free_ram_mib: 20000,
        gpus: vec![
            Gpu {
                index: 0,
                name: "Observed CUDA A".into(),
                uuid: "CUDA0".into(),
                backend: "cuda".into(),
                total_mib: 12000,
                free_mib: 10000,
            },
            Gpu {
                index: 2,
                name: "Observed Vulkan B".into(),
                uuid: "Vulkan2".into(),
                backend: "vulkan".into(),
                total_mib: 8000,
                free_mib: 7000,
            },
        ],
        ..Default::default()
    }
}
#[test]
fn selections_require_exact_detected_backend_and_device() {
    let h = hardware();
    let devices = trials::select_devices(
        &h,
        &[
            DeviceRequest::cpu(),
            DeviceRequest {
                backend: Backend::Cuda,
                gpu_index: Some(0),
            },
            DeviceRequest {
                backend: Backend::Vulkan,
                gpu_index: Some(2),
            },
        ],
    )
    .unwrap();
    assert_eq!(devices.len(), 3);
    assert_eq!(devices[1].runtime_id, "CUDA0");
    assert_eq!(devices[2].name, "Observed Vulkan B");
    for request in [
        DeviceRequest {
            backend: Backend::Cuda,
            gpu_index: Some(2),
        },
        DeviceRequest {
            backend: Backend::Vulkan,
            gpu_index: Some(0),
        },
        DeviceRequest {
            backend: Backend::Auto,
            gpu_index: None,
        },
        DeviceRequest {
            backend: Backend::Cpu,
            gpu_index: Some(0),
        },
        DeviceRequest {
            backend: Backend::Cuda,
            gpu_index: None,
        },
    ] {
        assert!(trials::select_devices(&h, &[request]).is_err());
    }
    assert!(trials::select_devices(&h, &[DeviceRequest::cpu(), DeviceRequest::cpu()]).is_err());
}
#[test]
fn absent_gpus_are_not_invented_from_expected_or_previous_indices() {
    let h = Hardware {
        gpus: vec![],
        ..hardware()
    };
    assert!(
        trials::select_devices(
            &h,
            &[DeviceRequest {
                backend: Backend::Cuda,
                gpu_index: Some(0)
            }]
        )
        .is_err()
    );
    assert_eq!(
        trials::select_devices(&h, &[DeviceRequest::cpu()])
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn nvidia_process_metrics_preserve_unknowns_and_sum_only_the_owned_pid() {
    let rows =
        trials::parse_nvidia_processes("42, GPU-aaa, 512\n42,GPU-bbb,256\n99,GPU-ccc,9000\n")
            .unwrap();
    let (memory, uuids) = trials::process_vram(&rows, 42).unwrap();
    assert_eq!(memory, Some(768));
    assert_eq!(uuids, vec!["GPU-aaa", "GPU-bbb"]);
    assert_eq!(trials::process_vram(&rows, 123).unwrap().0, None);
    let unknown = trials::parse_nvidia_processes("42,GPU-aaa,N/A\n42,GPU-bbb,256").unwrap();
    assert_eq!(trials::process_vram(&unknown, 42).unwrap().0, None);
    assert_eq!(
        trials::parse_nvidia_processes("42,GPU-aaa,0").unwrap()[0].used_mib,
        Some(0)
    );
    assert!(trials::parse_nvidia_processes("").unwrap().is_empty());
}
#[test]
fn nvidia_parser_rejects_corrupt_duplicate_oversized_and_unit_bearing_rows() {
    for value in [
        "0,GPU-a,1",
        "abc,GPU-a,1",
        "42,GPU-a,-10",
        "42,GPU-a,123 MiB",
        "42,GPU-a,1,extra",
        "42,,1",
        "42,GPU-a,1\n42,GPU-a,2",
    ] {
        assert!(
            trials::parse_nvidia_processes(value).is_err(),
            "accepted {value:?}"
        );
    }
    assert!(trials::parse_nvidia_processes(&"x".repeat(1_000_001)).is_err());
    let rows = trials::parse_nvidia_processes("42,GPU-a,[Not Supported]").unwrap();
    assert_eq!(rows[0].used_mib, None);
}
#[test]
fn sampled_highwater_keeps_missing_readings_unknown_and_retains_actual_observations() {
    let samples = vec![
        MemorySample {
            elapsed_ms: 0,
            app_rss_bytes: Some(100),
            runtime_rss_bytes: Some(1000),
            runtime_vram_mib: Some(400),
            ..Default::default()
        },
        MemorySample {
            elapsed_ms: 500,
            app_rss_bytes: Some(200),
            runtime_rss_bytes: None,
            runtime_vram_mib: None,
            ..Default::default()
        },
        MemorySample {
            elapsed_ms: 1000,
            app_rss_bytes: Some(150),
            runtime_rss_bytes: Some(1200),
            runtime_vram_mib: Some(300),
            ..Default::default()
        },
    ];
    let summary = trials::summarize(samples, 500, false);
    assert_eq!(summary.app_rss_highwater_bytes, Some(200));
    assert_eq!(summary.runtime_rss_highwater_bytes, Some(1200));
    assert_eq!(summary.runtime_vram_highwater_mib, Some(400));
    assert!(summary.notes.iter().any(|n| n.contains("lower bounds")));
    let missing = trials::summarize(vec![MemorySample::default()], 500, false);
    assert_eq!(missing.runtime_rss_highwater_bytes, None);
    assert_eq!(missing.runtime_vram_highwater_mib, None);
}
#[test]
fn admission_suggestions_never_reduce_original_estimates_or_fabricate_unknown_metrics() {
    let memory = MemorySummary {
        runtime_rss_highwater_bytes: Some(2 * 1048576),
        runtime_vram_highwater_mib: Some(1024),
        ..Default::default()
    };
    let suggested = trials::conservative_budget(9000, 7000, &memory);
    assert_eq!(suggested.runtime_ram_mib, Some(9000));
    assert_eq!(suggested.vram_mib, Some(7000));
    let unknown = trials::conservative_budget(9000, 7000, &MemorySummary::default());
    assert_eq!(unknown.runtime_ram_mib, None);
    assert_eq!(unknown.vram_mib, None);
    let measured = MemorySummary {
        runtime_rss_highwater_bytes: Some(8192 * 1048576),
        runtime_vram_highwater_mib: Some(8192),
        ..Default::default()
    };
    let suggested = trials::conservative_budget(0, 0, &measured);
    assert_eq!(suggested.runtime_ram_mib, Some(10752));
    assert_eq!(suggested.vram_mib, Some(10496));
}
#[test]
fn live_sampler_is_bounded_stops_without_waiting_the_full_interval_and_does_not_guess_gpu_memory() {
    let sampler = trials::MemorySampler::start(None, false, 5000).unwrap();
    let started = Instant::now();
    let memory = sampler.finish();
    assert!(started.elapsed() < Duration::from_secs(4));
    assert!(!memory.samples.is_empty());
    assert_eq!(memory.requested_interval_ms, 5000);
    assert_eq!(memory.runtime_rss_highwater_bytes, None);
    assert_eq!(memory.runtime_vram_highwater_mib, None);
    assert!(
        memory
            .samples
            .iter()
            .all(|sample| !sample.vram_query_performed)
    );
    assert!(trials::MemorySampler::start(None, false, 1).is_err());
}
fn device() -> Device {
    Device {
        request: DeviceRequest::cpu(),
        name: "CPU".into(),
        runtime_id: "CPU".into(),
    }
}
fn trial(seconds: f64, success: bool, text: &str) -> Trial {
    Trial {
        device: device(),
        repetition: 1,
        elapsed_seconds: seconds,
        startup_seconds: None,
        read_seconds: Some(seconds),
        success,
        acceleration_verified: false,
        managed_pid: None,
        memory: MemorySummary::default(),
        review_regions: 0,
        content: success.then(|| PageContent {
            blocks: vec![Block::text(text)],
            ..Default::default()
        }),
        scorecard: None,
        quality_error: None,
        error: (!success).then(|| "Recorded failure".into()),
    }
}
#[test]
fn comparisons_use_only_completed_measurements_and_show_output_variation() {
    let observed = vec![
        trial(3.0, true, "A"),
        trial(1.0, true, "A"),
        trial(8.0, false, ""),
        trial(2.0, true, "B"),
    ];
    let summary = trials::device_summaries(&observed, &[device()]).unwrap();
    assert_eq!(summary[0].successes, 3);
    assert_eq!(summary[0].failures, 1);
    assert_eq!(summary[0].median_seconds, Some(2.0));
    assert!(summary[0].output_varied);
    let failed = trials::device_summaries(
        &[trial(f64::NAN, true, "A"), trial(0.0, false, "")],
        &[device()],
    )
    .unwrap();
    assert_eq!(failed[0].median_seconds, None);
    assert_eq!(failed[0].successes, 0);
}
struct Fixture {
    _temp: tempfile::TempDir,
    project: Project,
    settings: Settings,
}
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let settings = Settings {
        server: temp.path().join("missing-server.exe"),
        tesseract: temp.path().join("missing-tesseract.exe"),
        preset: Preset::Fast,
        ..Default::default()
    };
    let project = Project::open(
        temp.path().join("project"),
        Manifest {
            schema: 2,
            input_sha256: "source".into(),
            fingerprint: "settings".into(),
            pages: vec![1],
            models: vec![],
            settings: settings.clone(),
        },
    )
    .unwrap();
    image::RgbImage::from_pixel(800, 240, image::Rgb([255, 255, 255]))
        .save(project.image(1))
        .unwrap();
    project
        .save(&Page {
            page: 1,
            image: "page-00001.png".into(),
            content: PageContent {
                blocks: vec![Block::text("Prior confirmed source")],
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
    Fixture {
        _temp: temp,
        project,
        settings,
    }
}
#[test]
fn invalid_options_and_cancellation_do_not_change_project_data() {
    let f = fixture();
    let p = &f.project;
    let before = fs::read(p.page_path(1)).unwrap();
    let mut options = Options {
        page: 99,
        ..Default::default()
    };
    assert!(options.validate(p).is_err());
    options.page = 1;
    options.devices = vec![DeviceRequest {
        backend: Backend::Cuda,
        gpu_index: Some(0),
    }];
    assert!(options.validate(p).is_err());
    let control = Control::default();
    control
        .cancelled
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        trials::run(
            p,
            &Options::default(),
            &f.settings,
            &control,
            &Activity::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(p.page_path(1)).unwrap(), before);
    assert!(!p.root.join("review-history.json").exists());
}
#[test]
fn failed_native_trials_report_failures_without_inventing_quality_or_measurements() {
    let f = fixture();
    let original = fingerprint(&f.project.load(1).unwrap()).unwrap();
    let mut settings = f.settings.clone();
    settings.experience.live_preview = false;
    let report = trials::run(
        &f.project,
        &Options {
            repetitions: 1,
            ..Default::default()
        },
        &settings,
        &Control::default(),
        &Activity::default(),
    )
    .unwrap();
    assert_eq!(report.trials.len(), 1);
    let measured = &report.trials[0];
    assert!(!measured.success);
    assert!(measured.error.is_some());
    assert!(measured.scorecard.is_none());
    assert!(measured.elapsed_seconds > 0.0);
    assert_eq!(measured.memory.runtime_vram_highwater_mib, None);
    assert!(!report.reference_supplied);
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("Accuracy is unmeasured"))
    );
    assert_eq!(fingerprint(&f.project.load(1).unwrap()).unwrap(), original);
    assert!(report.recommended.is_none());
}
fn printed_image() -> image::RgbImage {
    use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
    let path = notegen::searchable_pdf::default_font().unwrap();
    let font = FontArc::try_from_vec(fs::read(path).unwrap()).unwrap();
    let scaled = font.as_scaled(PxScale::from(42.0));
    let mut image = image::RgbImage::from_pixel(800, 240, image::Rgb([255, 255, 255]));
    for (line, text) in ["Hardware trial", "Total 1250"].iter().enumerate() {
        let mut x = 30.0;
        for ch in text.chars() {
            let id = font.glyph_id(ch);
            let glyph = id.with_scale_and_position(42.0, point(x, 70.0 + line as f32 * 80.0));
            if let Some(glyph) = font.outline_glyph(glyph) {
                let bounds = glyph.px_bounds();
                glyph.draw(|gx, gy, coverage| {
                    let px = gx as i32 + bounds.min.x as i32;
                    let py = gy as i32 + bounds.min.y as i32;
                    if (0..800).contains(&px) && (0..240).contains(&py) {
                        let value = (255.0 * (1.0 - coverage)) as u8;
                        image.put_pixel(px as u32, py as u32, image::Rgb([value, value, value]));
                    }
                });
            }
            x += scaled.h_advance(id);
        }
    }
    image
}
#[test]
fn real_cpu_copied_page_trial_separates_timing_quality_and_review_without_mutation() {
    let f = fixture();
    let p = &f.project;
    printed_image().save(p.image(1)).unwrap();
    let original = fs::read(p.page_path(1)).unwrap();
    let source = fs::read(p.image(1)).unwrap();
    let mut settings = f.settings.clone();
    settings.tesseract = Settings::default().tesseract;
    settings.experience.live_preview = true;
    let events = Arc::new(Mutex::new(vec![]));
    let captured = events.clone();
    let log = Activity {
        callback: Some(Arc::new(move |event| {
            if let Some(preview) = event.preview {
                captured.lock().unwrap().push(preview);
            }
        })),
        ..Default::default()
    };
    let options = Options {
        repetitions: 2,
        reference: Some(PageContent {
            blocks: vec![Block::text("Hardware trial\nTotal 1250")],
            ..Default::default()
        }),
        ..Default::default()
    };
    let report = trials::run(p, &options, &settings, &Control::default(), &log).unwrap();
    assert_eq!(report.trials.len(), 2);
    assert!(report.copied_source);
    assert!(report.reference_supplied);
    assert!(
        report.trials.iter().all(|trial| trial.success),
        "{:?}",
        report.trials.iter().map(|t| &t.error).collect::<Vec<_>>()
    );
    for trial in &report.trials {
        assert!(trial.elapsed_seconds > 0.0);
        assert!(trial.startup_seconds.is_none());
        assert!(trial.read_seconds.unwrap() > 0.0);
        assert!(trial.content.as_ref().unwrap().plain().contains("1250"));
        assert_eq!(trial.scorecard.as_ref().unwrap().exact_numbers.errors, 0);
        assert!(trial.managed_pid.is_none());
        assert_eq!(trial.memory.runtime_vram_highwater_mib, None);
    }
    assert_eq!(fs::read(p.page_path(1)).unwrap(), original);
    assert_eq!(fs::read(p.image(1)).unwrap(), source);
    let events = events.lock().unwrap();
    assert!(events.last().unwrap().clear);
    for event in events.iter().filter(|event| event.image.is_some()) {
        let copy = event.image.as_ref().unwrap();
        assert_ne!(copy, &p.image(1));
        assert!(!copy.exists());
    }
}
#[test]
#[ignore = "Explicit opt-in: requires a managed local model and selected physical GPU; measures only this host"]
fn actual_managed_gpu_trial_when_explicitly_configured() {
    let root = std::env::var_os("NOTEGEN_HARDWARE_TRIAL_PROJECT")
        .map(PathBuf::from)
        .expect("Set NOTEGEN_HARDWARE_TRIAL_PROJECT");
    let reader =
        std::env::var("NOTEGEN_HARDWARE_TRIAL_READER").expect("Set NOTEGEN_HARDWARE_TRIAL_READER");
    let index = std::env::var("NOTEGEN_HARDWARE_TRIAL_GPU")
        .unwrap_or_else(|_| "0".into())
        .parse::<u32>()
        .unwrap();
    let project = Project::read(root).unwrap();
    let settings = project.manifest.settings.clone();
    let options = Options {
        reader: Some(reader),
        devices: vec![DeviceRequest {
            backend: Backend::Cuda,
            gpu_index: Some(index),
        }],
        repetitions: 1,
        ..Default::default()
    };
    let report = trials::run(
        &project,
        &options,
        &settings,
        &Control::default(),
        &Activity::default(),
    )
    .unwrap();
    assert!(report.trials[0].success, "{:?}", report.trials[0].error);
    assert!(report.trials[0].acceleration_verified);
    assert!(report.trials[0].managed_pid.is_some());
    assert!(!report.trials[0].memory.samples.is_empty());
}
