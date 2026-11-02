use notegen::{maintenance, pipeline, settings_help, setup, telemetry, tuning, types::*};
use serde_json::{Value, json};

fn release(build: &str) -> Value {
    let assets = ["cpu","vulkan"].iter().map(|backend| {
        let name=format!("llama-{build}-bin-win-{backend}-x64.zip");
        json!({"name":name,"size":123456,"digest":format!("sha256:{}","a".repeat(64)),"browser_download_url":format!("https://github.com/ggml-org/llama.cpp/releases/download/{build}/{name}")})
    }).collect::<Vec<_>>();
    json!({"tag_name":build,"draft":false,"assets":assets})
}
#[test]
fn runtime_discovery_skips_a_stable_marker_with_no_binary_assets() {
    let parsed = maintenance::parse_runtime(
        &json!([{ "tag_name":"v0.8.0", "draft":false, "assets":[] },release("b12032")]),
    )
    .unwrap();
    assert_eq!(parsed.build, "b12032");
    assert_eq!(parsed.assets.len(), 2);
}
#[test]
fn runtime_metadata_rejects_missing_digest_wrong_host_and_duplicate_assets() {
    let mut r = release("b12032");
    r["assets"][0]["digest"] = Value::Null;
    assert!(maintenance::parse_runtime(&json!([r])).is_err());
    let mut r = release("b12032");
    r["assets"][0]["browser_download_url"] = json!("https://example.com/runtime.zip");
    assert!(maintenance::parse_runtime(&json!([r])).is_err());
    let r = maintenance::parse_runtime(&json!([release("b12032")])).unwrap();
    let mut assets = r.assets.clone();
    assets.push(assets[0].clone());
    assert!(maintenance::validate_runtime_release(&r.build, &assets).is_err());
    assert!(maintenance::validate_runtime_release("../b12032", &r.assets).is_err());
}
#[test]
fn compiled_runtime_pins_are_valid_for_the_selected_release() {
    let pins: Vec<setup::Download> =
        serde_json::from_str(include_str!("../runtime-downloads.json")).unwrap();
    maintenance::validate_runtime_release("b12032", &pins).unwrap();
}
#[test]
fn app_update_requires_newer_stable_release_and_its_exact_installer() {
    let mut data = json!({"tag_name":"v0.6.0","draft":false,"prerelease":false,"assets":[{"id":123,"name":"NoteGen-Setup-0.6.0.exe","size":123456,"digest":format!("sha256:{}","b".repeat(64)),"browser_download_url":"https://github.com/InduwaraLasindu/NoteGen/releases/download/v0.6.0/NoteGen-Setup-0.6.0.exe"}]});
    assert_eq!(
        maintenance::parse_app(&data).unwrap().unwrap().asset_id,
        123
    );
    data["tag_name"] = json!("v0.3.0");
    assert!(maintenance::parse_app(&data).unwrap().is_none());
    data["tag_name"] = json!(format!("v{}", env!("CARGO_PKG_VERSION")));
    assert!(maintenance::parse_app(&data).unwrap().is_none());
    data["tag_name"] = json!("v0.6.0");
    data["prerelease"] = json!(true);
    assert!(maintenance::parse_app(&data).unwrap().is_none());
    data["prerelease"] = json!(false);
    data["assets"][0]["browser_download_url"] = json!("https://example.com/a.exe");
    assert!(maintenance::parse_app(&data).is_err());
}
#[test]
fn preview_hides_reasoning_tags_including_incomplete_stream_fragments() {
    assert_eq!(
        telemetry::visible_draft("<think>private</think>Total 1250"),
        "Total 1250"
    );
    assert_eq!(telemetry::visible_draft("Draft <think>private"), "Draft ");
    assert_eq!(telemetry::visible_draft("Draft <thi"), "Draft ");
    assert_eq!(
        telemetry::visible_draft(&"é".repeat(20000)).chars().count(),
        16000
    );
}
#[test]
fn live_drafts_cannot_replace_a_different_page_or_resurrect_a_cleared_view() {
    use telemetry::{Change, Preview};
    let mut live = Preview::default();
    assert_eq!(
        live.apply(Preview::source(
            "page-2.png".into(),
            2,
            "first reader",
            None
        )),
        Change::SourceChanged
    );
    assert_eq!(
        live.apply(Preview::draft(2, "draft for page 2")),
        Change::Draft
    );
    assert_eq!(
        live.apply(Preview::source("page-7.png".into(), 7, "re-read", None)),
        Change::SourceChanged
    );
    assert_eq!(live.draft.as_deref(), Some(""));
    assert_eq!(
        live.apply(Preview::draft(2, "stale draft")),
        Change::Ignored
    );
    assert_eq!(live.draft.as_deref(), Some(""));
    assert_eq!(
        live.apply(Preview::draft(7, "current draft")),
        Change::Draft
    );
    assert_eq!(live.reader, "re-read");
    assert_eq!(live.apply(Preview::clear()), Change::Cleared);
    assert!(live.clear);
    assert!(live.image.is_none() && live.draft.is_none());
    assert_eq!(live.apply(Preview::draft(7, "late draft")), Change::Ignored);
    assert!(live.draft.is_none());
}
#[test]
fn another_reader_and_crop_on_the_same_image_reset_the_visible_draft() {
    use telemetry::{Change, Preview};
    let mut live = Preview::source("page.png".into(), 3, "full page", None);
    live.apply(Preview::draft(3, "full-page answer"));
    let region = BoxRect {
        left: 0.1,
        top: 0.2,
        right: 0.6,
        bottom: 0.8,
    };
    assert_eq!(
        live.apply(Preview::source(
            "page.png".into(),
            3,
            "crop reader",
            Some(region)
        )),
        Change::SourceRetained
    );
    assert_eq!(live.reader, "crop reader");
    assert_eq!(live.draft.as_deref(), Some(""));
    assert_eq!(live.region.unwrap().left, 0.1);
}
#[test]
fn legacy_preview_events_still_decode_and_empty_path_resets_clear_state() {
    use telemetry::{Change, Preview};
    let mut value =
        serde_json::to_value(Preview::source("page.png".into(), 1, "reader", None)).unwrap();
    value.as_object_mut().unwrap().remove("clear");
    let mut live: Preview = serde_json::from_value(value).unwrap();
    assert!(!live.clear);
    assert_eq!(
        live.apply(Preview {
            image: Some(Default::default()),
            ..Default::default()
        }),
        Change::Cleared
    );
    assert!(live.image.is_none());
}
#[test]
fn live_and_update_preferences_do_not_invalidate_recognition_checkpoints() {
    let a = Settings::default();
    let mut b = a.clone();
    b.experience.live_preview = false;
    b.experience.check_updates = false;
    assert_eq!(
        serde_json::to_value(pipeline::recognition_settings(&a)).unwrap(),
        serde_json::to_value(pipeline::recognition_settings(&b)).unwrap()
    );
}
#[test]
fn every_serialized_setting_has_an_explanation() {
    fn visit(value: &Value, prefix: &str) {
        if let Some(object) = value.as_object() {
            for (key, v) in object {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                assert!(
                    !settings_help::description(&path).is_empty(),
                    "Missing description for {path}"
                );
                if v.is_object() {
                    visit(v, &path);
                }
            }
        }
    }
    visit(&serde_json::to_value(Settings::default()).unwrap(), "");
}
#[test]
fn hardware_selection_ignores_fast_but_incorrect_or_invalid_trials() {
    let mut trial = tuning::Trial {
        threads: 4,
        milliseconds: 100.,
        correct: true,
        gpu_verified: false,
        detail: String::new(),
    };
    let good = trial.clone();
    trial.milliseconds = 1.;
    trial.correct = false;
    let mut nan = good.clone();
    nan.milliseconds = f64::NAN;
    assert_eq!(tuning::best(&[trial, nan, good]).unwrap().threads, 4);
    let h = notegen::hardware::Hardware {
        logical_cores: 1,
        physical_cores: 1,
        ..Default::default()
    };
    assert_eq!(tuning::candidates(&h), vec![1]);
}
