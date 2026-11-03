use notegen::{
    hardware::{Gpu, Hardware},
    resources::*,
    types::*,
};

fn hardware() -> Hardware {
    Hardware {
        logical_cores: 8,
        free_ram_mib: 8192,
        ram_mib: 16384,
        gpus: vec![Gpu {
            free_mib: 4096,
            total_mib: 4096,
            ..Default::default()
        }],
        ..Default::default()
    }
}
#[test]
fn all_profiles_preview_apply_and_undo_without_affecting_document_settings() {
    for profile in [
        PowerProfile::Quiet,
        PowerProfile::Battery,
        PowerProfile::Balanced,
        PowerProfile::Throughput,
    ] {
        let mut s = Settings::default();
        let original = serde_json::to_value(&s).unwrap();
        let p = preview(&s, &hardware(), profile, &Reservations::default()).unwrap();
        assert_eq!(serde_json::to_value(&s).unwrap(), original);
        p.apply(&mut s).unwrap();
        assert_eq!(s.cpu_threads, p.after.cpu_threads);
        assert_eq!(s.preset, Preset::Accuracy);
        s.subject = "Changed after preview".into();
        p.undo(&mut s).unwrap();
        assert_eq!(s.subject, "Changed after preview");
        s.subject.clear();
        assert_eq!(serde_json::to_value(s).unwrap(), original);
    }
}
#[test]
fn reservations_bound_cpu_and_memory_and_reject_starvation() {
    let s = Settings::default();
    let r = Reservations {
        cpu_cores: 7,
        ram_mib: Some(4096),
        vram_mib: Some(2048),
    };
    let p = preview(&s, &hardware(), PowerProfile::Throughput, &r).unwrap();
    assert_eq!(p.after.cpu_threads, 1);
    assert_eq!(p.ram_budget_mib, 4096);
    assert_eq!(p.vram_budget_mib, 2048);
    let mut r = r;
    r.cpu_cores = 8;
    assert!(preview(&s, &hardware(), PowerProfile::Quiet, &r).is_err());
    r.cpu_cores = usize::MAX;
    assert!(preview(&s, &hardware(), PowerProfile::Quiet, &r).is_err());
    r.cpu_cores = 0;
    r.ram_mib = Some(u64::MAX);
    assert!(preview(&s, &hardware(), PowerProfile::Quiet, &r).is_err());
}
#[test]
fn stale_resource_previews_and_undo_are_rejected() {
    let mut s = Settings::default();
    let p = preview(
        &s,
        &hardware(),
        PowerProfile::Quiet,
        &Reservations::default(),
    )
    .unwrap();
    s.cpu_threads = 3;
    assert!(p.apply(&mut s).is_err());
    s.cpu_threads = 0;
    p.apply(&mut s).unwrap();
    s.ui.fps = 40;
    assert!(p.undo(&mut s).is_err());
}
#[test]
fn single_core_and_extreme_core_counts_stay_within_valid_limits() {
    let mut hw = hardware();
    hw.logical_cores = 1;
    assert_eq!(
        preview(
            &Settings::default(),
            &hw,
            PowerProfile::Battery,
            &Reservations::default()
        )
        .unwrap()
        .after
        .cpu_threads,
        1
    );
    hw.logical_cores = usize::MAX;
    assert_eq!(
        preview(
            &Settings::default(),
            &hw,
            PowerProfile::Throughput,
            &Reservations::default()
        )
        .unwrap()
        .after
        .cpu_threads,
        512
    );
}
#[test]
fn budget_preserves_order_and_reports_estimates_and_unknowns() {
    let jobs = vec![
        QueueDuration {
            id: "a".into(),
            seconds: Some(10.),
            measured: true,
        },
        QueueDuration {
            id: "b".into(),
            seconds: Some(15.),
            measured: false,
        },
        QueueDuration {
            id: "c".into(),
            seconds: None,
            measured: false,
        },
        QueueDuration {
            id: "d".into(),
            seconds: Some(1.),
            measured: true,
        },
    ];
    let p = time_budget(&jobs, 20.).unwrap();
    assert!(p.jobs[0].within_budget);
    assert!(!p.jobs[1].within_budget);
    assert_eq!(p.jobs[1].finishes_after_seconds, Some(25.));
    assert_eq!(p.jobs[3].finishes_after_seconds, None);
    assert_eq!(p.unresolved, vec!["b", "c"]);
}
#[test]
fn time_budgets_reject_nonfinite_negative_and_duplicate_inputs() {
    assert!(time_budget(&[], f64::NAN).is_err());
    assert!(time_budget(&[], -1.).is_err());
    assert!(time_budget(&[], f64::INFINITY).is_err());
    let mut j = QueueDuration {
        id: "a".into(),
        seconds: Some(-1.),
        measured: true,
    };
    assert!(time_budget(&[j.clone()], 10.).is_err());
    j.seconds = Some(0.);
    assert!(time_budget(&[j.clone(), j], 10.).is_err());
    assert!(time_budget(&[], 0.).unwrap().jobs.is_empty());
}
#[test]
fn persisted_resource_preview_roundtrips_and_preserves_unrelated_settings() {
    let mut s = Settings::default();
    let before = serde_json::to_value(&s).unwrap();
    let original = preview(
        &s,
        &hardware(),
        PowerProfile::Battery,
        &Reservations::default(),
    )
    .unwrap();
    let bytes = serde_json::to_vec(&original).unwrap();
    let restored: ResourcePreview = serde_json::from_slice(&bytes).unwrap();
    restored.apply(&mut s).unwrap();
    s.subject = "Keep this unrelated edit".into();
    restored.undo(&mut s).unwrap();
    assert_eq!(s.subject, "Keep this unrelated edit");
    s.subject.clear();
    assert_eq!(serde_json::to_value(s).unwrap(), before);
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("models_dir"));
    assert!(!text.contains("endpoint"));
    assert!(!text.contains("subject"));
}
#[test]
fn persisted_previews_reject_invalid_apply_and_undo_values_without_mutation() {
    let original_settings = Settings::default();
    let valid = preview(
        &original_settings,
        &hardware(),
        PowerProfile::Quiet,
        &Reservations::default(),
    )
    .unwrap();
    for (field, value) in [
        ("cpu_threads", serde_json::json!(513)),
        ("ram_fraction", serde_json::json!(2.0)),
        ("vram_fraction", serde_json::json!(-0.1)),
        ("fps", serde_json::json!(0)),
        ("idle_fps", serde_json::json!(61)),
    ] {
        let mut tampered = serde_json::to_value(&valid).unwrap();
        tampered["after"][field] = value;
        let decoded: ResourcePreview = serde_json::from_value(tampered).unwrap();
        let mut current = original_settings.clone();
        let before = serde_json::to_value(&current).unwrap();
        assert!(decoded.apply(&mut current).is_err());
        assert_eq!(serde_json::to_value(current).unwrap(), before);
    }
    let mut applied = original_settings.clone();
    valid.apply(&mut applied).unwrap();
    let before_undo = serde_json::to_value(&applied).unwrap();
    let mut tampered = serde_json::to_value(&valid).unwrap();
    tampered["before"]["fps"] = serde_json::json!(0);
    let decoded: ResourcePreview = serde_json::from_value(tampered).unwrap();
    assert!(decoded.undo(&mut applied).is_err());
    assert_eq!(serde_json::to_value(&applied).unwrap(), before_undo);
    let mut nonfinite = valid;
    nonfinite.after.ram_fraction = f32::NAN;
    let mut current = original_settings.clone();
    assert!(nonfinite.apply(&mut current).is_err());
    assert_eq!(
        serde_json::to_value(current).unwrap(),
        serde_json::to_value(original_settings).unwrap()
    );
}
#[test]
fn persisted_previews_deny_unrecognized_top_level_and_nested_fields() {
    let p = preview(
        &Settings::default(),
        &hardware(),
        PowerProfile::Balanced,
        &Reservations::default(),
    )
    .unwrap();
    let mut value = serde_json::to_value(&p).unwrap();
    value["private_path"] = serde_json::json!("not allowed");
    assert!(serde_json::from_value::<ResourcePreview>(value).is_err());
    let mut value = serde_json::to_value(p).unwrap();
    value["after"]["endpoint"] = serde_json::json!("not allowed");
    assert!(serde_json::from_value::<ResourcePreview>(value).is_err());
}
