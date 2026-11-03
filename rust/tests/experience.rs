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
    b.experience.update_channel = UpdateChannel::Preview;
    b.experience.defer_updates_until = Some(chrono::Utc::now());
    assert_eq!(
        serde_json::to_value(pipeline::recognition_settings(&a)).unwrap(),
        serde_json::to_value(pipeline::recognition_settings(&b)).unwrap()
    );
}

fn app_release(version: &str, preview: bool) -> Value {
    let file = format!("NoteGen-Setup-{version}.exe");
    json!({"tag_name":format!("v{version}"),"draft":false,"prerelease":preview,"body":"Compatibility notes\nKeep a backup.","assets":[{"id":321,"name":file,"size":123456,"digest":format!("sha256:{}","b".repeat(64)),"browser_download_url":format!("https://github.com/InduwaraLasindu/NoteGen/releases/download/v{version}/{file}")}]})
}

#[test]
fn app_channels_select_highest_version_and_stable_excludes_both_kinds_of_preview() {
    let data = json!([
        app_release("0.6.0", false),
        app_release("0.8.0-rc.1", false),
        app_release("0.7.0", true),
        app_release("0.5.1", false)
    ]);
    let stable = maintenance::select_app_release(&data, UpdateChannel::Stable, "0.5.0")
        .unwrap()
        .unwrap();
    assert_eq!(stable.version, "0.6.0");
    assert!(!stable.prerelease);
    let preview = maintenance::select_app_release(&data, UpdateChannel::Preview, "0.5.0")
        .unwrap()
        .unwrap();
    assert_eq!(preview.version, "0.8.0-rc.1");
    assert!(preview.prerelease);
    assert!(
        maintenance::select_app_release(&data, UpdateChannel::Stable, "0.9.0")
            .unwrap()
            .is_none()
    );
    let mut draft = app_release("0.9.0", false);
    draft["draft"] = json!(true);
    assert!(
        maintenance::select_app_release(&draft, UpdateChannel::Preview, "0.5.0")
            .unwrap()
            .is_none()
    );
}

#[test]
fn app_metadata_rejects_noncanonical_versions_duplicate_assets_and_changed_downloads() {
    for bad in ["01.6.0", "0.6.0+build", "../0.6.0", "0.6", "0.6.0-rc/1"] {
        assert!(
            maintenance::select_app_release(
                &app_release(bad, true),
                UpdateChannel::Preview,
                "0.5.0"
            )
            .is_err()
        );
    }
    let mut duplicate = app_release("0.6.0", false);
    let asset = duplicate["assets"][0].clone();
    duplicate["assets"].as_array_mut().unwrap().push(asset);
    assert!(maintenance::select_app_release(&duplicate, UpdateChannel::Stable, "0.5.0").is_err());
    let mut data = app_release("0.6.0", false);
    data["assets"][0]["browser_download_url"] = json!("https://example.com/setup.exe");
    assert!(maintenance::select_app_release(&data, UpdateChannel::Stable, "0.5.0").is_err());
    let mut selected = maintenance::select_app_release(
        &app_release("0.6.0", false),
        UpdateChannel::Stable,
        "0.5.0",
    )
    .unwrap()
    .unwrap();
    selected.asset.file = "../setup.exe".into();
    assert!(maintenance::validate_app_release(&selected).is_err());
    assert!(
        maintenance::select_app_release(
            &json!(vec![app_release("0.6.0", false); 21]),
            UpdateChannel::Stable,
            "0.5.0"
        )
        .is_err()
    );
}

#[test]
fn app_notes_are_bounded_plain_text_and_saved_reports_are_revalidated() {
    let mut data = app_release("0.6.0", false);
    data["body"] = json!(format!("Keep\r\nbackup\u{202e}\u{1b}{}", "語".repeat(7000)));
    let mut selected = maintenance::select_app_release(&data, UpdateChannel::Stable, "0.5.0")
        .unwrap()
        .unwrap();
    assert!(selected.release_notes.contains("shortened"));
    assert!(
        !selected
            .release_notes
            .contains(['\r', '\u{202e}', '\u{1b}'])
    );
    maintenance::validate_app_release(&selected).unwrap();
    selected.release_notes.push('\u{202e}');
    assert!(maintenance::validate_app_release(&selected).is_err());
}

#[test]
fn automatic_check_deferral_is_exact_and_manual_checks_bypass_it() {
    use maintenance::{CheckDecision as D, CheckMode as M, CheckStamp};
    let now = chrono::DateTime::parse_from_rfc3339("2026-11-02T00:00:00Z")
        .unwrap()
        .to_utc();
    let until = now + chrono::Duration::days(7);
    let mut preferences = ExperienceOptions {
        defer_updates_until: Some(until),
        ..Default::default()
    };
    assert_eq!(
        maintenance::check_decision(&preferences, None, now, M::Automatic),
        D::Deferred(until)
    );
    assert_eq!(
        maintenance::check_decision(&preferences, None, now, M::Manual),
        D::Due
    );
    assert_eq!(
        maintenance::check_decision(&preferences, None, until, M::Automatic),
        D::Due
    );
    preferences.check_updates = false;
    assert_eq!(
        maintenance::check_decision(&preferences, None, now, M::Automatic),
        D::Disabled
    );
    assert_eq!(
        maintenance::check_decision(&preferences, None, now, M::Manual),
        D::Due
    );
    preferences.check_updates = true;
    preferences.defer_updates_until = None;
    let stamp = CheckStamp {
        checked_at: now,
        channel: UpdateChannel::Stable,
    };
    assert_eq!(
        maintenance::check_decision(&preferences, Some(&stamp), now, M::Automatic),
        D::Recent(now + chrono::Duration::hours(24))
    );
    preferences.update_channel = UpdateChannel::Preview;
    assert_eq!(
        maintenance::check_decision(&preferences, Some(&stamp), now, M::Automatic),
        D::Due
    );
    let legacy: ExperienceOptions = serde_json::from_value(json!({"check_updates":true})).unwrap();
    assert_eq!(legacy.update_channel, UpdateChannel::Stable);
    assert!(legacy.defer_updates_until.is_none());
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
