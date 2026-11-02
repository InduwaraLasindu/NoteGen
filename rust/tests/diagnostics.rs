use notegen::{
    diagnostics::*,
    setup,
    storage::{self, Activity, Control},
    types::*,
};
use std::{fs, io::Read, path::PathBuf, sync::atomic::Ordering};

#[test]
fn default_summary_excludes_private_fields_and_ignores_unselected_sources() {
    let mut s = Settings {
        subject: "PRIVATE_TOPIC".into(),
        models_dir: "PRIVATE_MODEL_PATH".into(),
        server: "PRIVATE_SERVER_PATH".into(),
        ..Default::default()
    };
    s.writer.root = "PRIVATE_WRITER_PATH".into();
    let options = SupportOptions::default();
    let unavailable = vec![PathBuf::from("unavailable-private-document")];
    let input = SupportInput {
        settings: &s,
        activity: &unavailable,
        documents: &unavailable,
        options: &options,
    };
    let p = support_preview(&input, &Control::default()).unwrap();
    assert_eq!(p.files.len(), 1);
    assert_eq!(p.files[0].name, "settings-summary.json");
    let text = String::from_utf8(
        inspect_support_file(&input, &p.files[0].name, &Control::default()).unwrap(),
    )
    .unwrap();
    for secret in [
        "PRIVATE_TOPIC",
        "PRIVATE_MODEL_PATH",
        "PRIVATE_SERVER_PATH",
        "PRIVATE_WRITER_PATH",
        "password",
    ] {
        assert!(!text.contains(secret));
    }
    let summary: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(summary["settings"].get("endpoint").is_none());
    assert!(!p.files[0].document_content);
}
#[test]
fn activity_preview_is_an_allowlist_and_never_contains_raw_messages_or_drafts() {
    let root = tempfile::tempdir().unwrap();
    let log = root.path().join("PRIVATE_LOG_NAME.jsonl");
    fs::write(&log,r#"{"timestamp":"2026-11-02T00:00:00Z","stage":"read","code":"NG201","page":1,"message":"SECRET_TOKEN C:/Users/Private","preview":{"text":"PRIVATE_RECOGNITION"},"done":2,"total":3}
{"stage":"PRIVATE_STAGE","code":"PRIVATE_CODE","message":"PRIVATE_MESSAGE"}
invalid PRIVATE_LINE"#).unwrap();
    let s = Settings::default();
    let options = SupportOptions {
        include_activity: true,
        ..Default::default()
    };
    let logs = vec![log];
    let input = SupportInput {
        settings: &s,
        activity: &logs,
        documents: &[],
        options: &options,
    };
    let p = support_preview(&input, &Control::default()).unwrap();
    assert_eq!(p.files.len(), 2);
    let text = String::from_utf8(
        inspect_support_file(&input, "activity-0001.json", &Control::default()).unwrap(),
    )
    .unwrap();
    assert!(text.contains("NG201"));
    assert!(text.contains("other"));
    assert!(text.contains("malformed_entries_omitted"));
    for secret in [
        "SECRET_TOKEN",
        "C:/Users/Private",
        "PRIVATE_RECOGNITION",
        "PRIVATE_STAGE",
        "PRIVATE_CODE",
        "PRIVATE_MESSAGE",
        "PRIVATE_LINE",
        "PRIVATE_LOG_NAME",
    ] {
        assert!(!text.contains(secret));
    }
}
#[test]
fn document_bytes_require_opt_in_and_explicit_file_selection() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("private-notes.txt");
    fs::write(&source, "PRIVATE_DOCUMENT_CONTENT").unwrap();
    let s = Settings::default();
    let documents = vec![source];
    let options = SupportOptions {
        include_document_content: true,
        ..Default::default()
    };
    let input = SupportInput {
        settings: &s,
        activity: &[],
        documents: &documents,
        options: &options,
    };
    let p = support_preview(&input, &Control::default()).unwrap();
    assert!(p.files.iter().any(|f| f.document_content));
    let out = root.path().join("support.zip");
    export_support(
        &input,
        &p,
        &p.token,
        &["settings-summary.json".into()],
        &out,
        &Control::default(),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(&out).unwrap()).unwrap();
    assert_eq!(zip.len(), 2);
    assert!(zip.by_name("document-0001.bin").is_err());
    let out2 = root.path().join("explicit.zip");
    export_support(
        &input,
        &p,
        &p.token,
        &["document-0001.bin".into()],
        &out2,
        &Control::default(),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(&out2).unwrap()).unwrap();
    let mut text = String::new();
    zip.by_name("document-0001.bin")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert_eq!(text, "PRIVATE_DOCUMENT_CONTENT");
    assert_eq!(
        zip.file_names().collect::<Vec<_>>(),
        vec!["document-0001.bin", "support-manifest.json"]
    );
}
#[test]
fn support_preview_fingerprints_reject_tampering_and_changed_sources() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fs::write(&source, "one").unwrap();
    let s = Settings::default();
    let documents = vec![source.clone()];
    let options = SupportOptions {
        include_document_content: true,
        ..Default::default()
    };
    let input = SupportInput {
        settings: &s,
        activity: &[],
        documents: &documents,
        options: &options,
    };
    let p = support_preview(&input, &Control::default()).unwrap();
    let out = root.path().join("support.zip");
    let selected = vec!["document-0001.bin".into()];
    assert!(export_support(&input, &p, "wrong", &selected, &out, &Control::default()).is_err());
    let mut tampered = p.clone();
    tampered.files[0].bytes += 1;
    assert!(
        export_support(
            &input,
            &tampered,
            &p.token,
            &selected,
            &out,
            &Control::default()
        )
        .is_err()
    );
    fs::write(&source, "two").unwrap();
    assert!(export_support(&input, &p, &p.token, &selected, &out, &Control::default()).is_err());
    assert!(!out.exists());
}
#[test]
fn support_cancellation_collisions_write_failures_and_bad_selections_preserve_output() {
    let root = tempfile::tempdir().unwrap();
    let s = Settings::default();
    let options = SupportOptions::default();
    let input = SupportInput {
        settings: &s,
        activity: &[],
        documents: &[],
        options: &options,
    };
    let c = Control::default();
    let p = support_preview(&input, &c).unwrap();
    let selected = vec!["settings-summary.json".into()];
    let out = root.path().join("support.zip");
    fs::write(&out, "existing").unwrap();
    assert!(export_support(&input, &p, &p.token, &selected, &out, &c).is_err());
    assert_eq!(fs::read(&out).unwrap(), b"existing");
    let fresh = root.path().join("fresh.zip");
    c.cancelled.store(true, Ordering::Relaxed);
    assert!(export_support(&input, &p, &p.token, &selected, &fresh, &c).is_err());
    assert!(!fresh.exists());
    let c = Control::default();
    let blocked = root.path().join("file-instead-of-folder");
    fs::write(&blocked, "preserve").unwrap();
    assert!(
        export_support(
            &input,
            &p,
            &p.token,
            &selected,
            &blocked.join("out.zip"),
            &c
        )
        .is_err()
    );
    assert_eq!(fs::read(&blocked).unwrap(), b"preserve");
    for bad in [
        vec![],
        vec!["../private".into()],
        vec![
            "settings-summary.json".into(),
            "settings-summary.json".into(),
        ],
    ] {
        assert!(export_support(&input, &p, &p.token, &bad, &fresh, &c).is_err());
        assert!(!fresh.exists());
    }
}
#[test]
fn activity_read_failures_non_utf8_and_size_limits_are_reported_without_export() {
    let root = tempfile::tempdir().unwrap();
    let s = Settings::default();
    let options = SupportOptions {
        include_activity: true,
        ..Default::default()
    };
    let bad = root.path().join("bad");
    fs::write(&bad, [0xff]).unwrap();
    for source in [root.path().to_path_buf(), root.path().join("missing"), bad] {
        let logs = vec![source];
        let input = SupportInput {
            settings: &s,
            activity: &logs,
            documents: &[],
            options: &options,
        };
        assert!(support_preview(&input, &Control::default()).is_err());
    }
    let huge = root.path().join("huge");
    fs::File::create(&huge).unwrap().set_len(2_000_001).unwrap();
    let logs = vec![huge];
    let input = SupportInput {
        settings: &s,
        activity: &logs,
        documents: &[],
        options: &options,
    };
    assert!(support_preview(&input, &Control::default()).is_err());
}
#[test]
fn troubleshooting_covers_fault_families_without_echoing_unknown_private_input() {
    for code in [
        "NG100", "NG102", "NG110", "NG200", "NG201", "NG202", "NG203", "NG300", "NG400", "NG401",
        "NG500", "NG610", "NG620", "NG630", "NG640", "NG650", "NG710", "NG720", "NG730", "NG900",
    ] {
        let t = troubleshoot(code);
        assert_eq!(t.code, code);
        assert!(!t.checks.is_empty());
        assert!(!t.actions.is_empty());
    }
    let t = troubleshoot("PRIVATE_EXCEPTION C:/Users/Private");
    assert_eq!(t.code, "unknown");
    assert!(!serde_json::to_string(&t).unwrap().contains("PRIVATE"));
}
fn repair_settings(root: &std::path::Path) -> Settings {
    Settings {
        models_dir: root.join("models"),
        server: root.join("runtime/llama-server.exe"),
        ..Default::default()
    }
}
#[test]
fn repair_preview_distinguishes_missing_corrupt_and_unreadable_pinned_assets() {
    let root = tempfile::tempdir().unwrap();
    let s = repair_settings(root.path());
    let ids: Vec<String> = vec!["lighton3-1b".into()];
    let (_, pin) = setup::pinned(&ids[0]).unwrap();
    let version = s
        .models_dir
        .join("versions/lighton3-1b")
        .join(&pin.revision);
    fs::create_dir_all(&version).unwrap();
    fs::write(version.join("weights.gguf"), b"corrupt").unwrap();
    fs::create_dir(version.join("projector.gguf")).unwrap();
    let input = RepairInput {
        settings: &s,
        model_ids: &ids,
        runtime_backend: Some("cpu"),
    };
    let p = repair_preview(&input, &Control::default()).unwrap();
    assert_eq!(
        p.files
            .iter()
            .find(|f| f.key == "model/lighton3-1b/weights")
            .unwrap()
            .health,
        AssetHealth::Corrupt
    );
    assert_eq!(
        p.files
            .iter()
            .find(|f| f.key == "model/lighton3-1b/projector")
            .unwrap()
            .health,
        AssetHealth::Unreadable
    );
    assert!(
        p.files
            .iter()
            .any(|f| f.key.starts_with("runtime-archive/") && f.health == AssetHealth::Missing)
    );
    assert!(p.files.iter().all(|f| f.expected_sha256.len() == 64));
    assert_eq!(fs::read(version.join("weights.gguf")).unwrap(), b"corrupt");
    assert!(
        !serde_json::to_string(&p)
            .unwrap()
            .contains(&root.path().to_string_lossy().to_string())
    );
}
#[test]
fn repair_rejects_cancelled_stale_or_tampered_previews_before_any_download_or_registration_write() {
    let root = tempfile::tempdir().unwrap();
    let s = repair_settings(root.path());
    let ids: Vec<String> = vec!["lighton3-1b".into()];
    fs::create_dir_all(&s.models_dir).unwrap();
    let registration = s.models_dir.join("lighton3-1b.notegen.json");
    fs::write(&registration, "REGISTRATION_PRESERVED").unwrap();
    let input = RepairInput {
        settings: &s,
        model_ids: &ids,
        runtime_backend: None,
    };
    let c = Control::default();
    let p = repair_preview(&input, &c).unwrap();
    let selected = vec!["model/lighton3-1b/weights".into()];
    assert!(repair_selected(&input, &p, "wrong", &selected, &c, &Activity::default()).is_err());
    let mut tampered = p.clone();
    tampered.files[0].expected_bytes += 1;
    assert!(
        repair_selected(
            &input,
            &tampered,
            &p.token,
            &selected,
            &c,
            &Activity::default()
        )
        .is_err()
    );
    let (_, pin) = setup::pinned(&ids[0]).unwrap();
    let version = s.models_dir.join("versions/lighton3-1b").join(pin.revision);
    fs::create_dir_all(&version).unwrap();
    fs::write(version.join("weights.gguf"), "appeared").unwrap();
    assert!(repair_selected(&input, &p, &p.token, &selected, &c, &Activity::default()).is_err());
    c.cancelled.store(true, Ordering::Relaxed);
    assert!(repair_preview(&input, &c).is_err());
    assert_eq!(fs::read(&registration).unwrap(), b"REGISTRATION_PRESERVED");
    assert_eq!(fs::read(version.join("weights.gguf")).unwrap(), b"appeared");
}
#[test]
fn repair_requires_supported_unique_pins_and_respects_storage_failures() {
    let root = tempfile::tempdir().unwrap();
    let s = repair_settings(root.path());
    let c = Control::default();
    for ids in [
        vec![],
        vec!["unknown-reader".into()],
        vec!["lighton3-1b".into(), "lighton3-1b".into()],
    ] {
        assert!(
            repair_preview(
                &RepairInput {
                    settings: &s,
                    model_ids: &ids,
                    runtime_backend: None
                },
                &c
            )
            .is_err()
        );
    }
    assert!(
        repair_preview(
            &RepairInput {
                settings: &s,
                model_ids: &[],
                runtime_backend: Some("unsupported")
            },
            &c
        )
        .is_err()
    );
    let ids: Vec<String> = vec!["lighton3-1b".into()];
    let (_, pin) = setup::pinned(&ids[0]).unwrap();
    let path = s
        .models_dir
        .join("versions/lighton3-1b")
        .join(pin.revision)
        .join("weights.gguf");
    fs::create_dir_all(&path).unwrap();
    let input = RepairInput {
        settings: &s,
        model_ids: &ids,
        runtime_backend: None,
    };
    let p = repair_preview(&input, &c).unwrap();
    assert!(
        repair_selected(
            &input,
            &p,
            &p.token,
            &["model/lighton3-1b/weights".into()],
            &c,
            &Activity::default()
        )
        .is_err()
    );
    assert!(path.is_dir());
    let _ = storage::fingerprint(&p).unwrap();
}
