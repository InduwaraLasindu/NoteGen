use notegen::{
    languages::*,
    storage::{Activity, Control},
};
use std::{fs, sync::atomic::Ordering};
#[test]
fn official_resource_catalog_is_immutable_complete_and_bounded() {
    let packs = catalog();
    assert_eq!(packs.len(), 6);
    for code in ["eng", "sin", "tam", "ara", "heb", "osd"] {
        let p = packs.iter().find(|p| p.code == code).unwrap();
        assert_eq!(p.revision, "87416418657359cb625c412a48b6e1d6d41c29bd");
        assert!(p.url.starts_with("https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/87416418657359cb625c412a48b6e1d6d41c29bd/"));
        assert_eq!(p.sha256.len(), 64);
        assert!(p.bytes > 100_000 && p.bytes < 20_000_000);
        assert_eq!(p.license, "Apache-2.0");
    }
}
#[test]
fn manual_mixed_language_order_is_preserved_and_invalid_codes_rejected() {
    assert_eq!(
        selection(&["sin".into(), "eng".into(), "tam".into()]).unwrap(),
        "sin+eng+tam"
    );
    for invalid in [
        vec![],
        vec!["osd".into()],
        vec!["sin".into(), "sin".into()],
        vec!["../eng".into()],
        vec!["fra".into()],
    ] {
        assert!(selection(&invalid).is_err());
    }
}
#[test]
fn plan_includes_english_orientation_license_and_exact_missing_bytes() {
    let root = tempfile::tempdir().unwrap();
    let p = plan(
        root.path(),
        &["sin".into(), "tam".into()],
        &Control::default(),
    )
    .unwrap();
    assert_eq!(p.recognition_languages, "sin+tam");
    assert_eq!(p.files.len(), 5);
    assert!(p.files.iter().any(|f| f.pack.code == "license"));
    assert_eq!(p.remaining_bytes, p.total_bytes);
    assert_eq!(
        p.total_bytes,
        4_113_088 + 1_727_413 + 3_237_963 + 10_562_727 + 11_358
    );
    assert!(verify_selection(root.path(), "sin+tam", &Control::default()).is_err());
}
#[test]
fn corrupt_files_are_reported_without_writing_or_treating_them_as_installed() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("sin.traineddata"), "corrupt").unwrap();
    let p = plan(root.path(), &["sin".into()], &Control::default()).unwrap();
    assert!(p.files.iter().all(|f| !f.verified));
    assert_eq!(
        fs::read(root.path().join("sin.traineddata")).unwrap(),
        b"corrupt"
    );
}
#[test]
fn cancellation_and_invalid_destinations_fail_before_download() {
    let root = tempfile::tempdir().unwrap();
    let c = Control::default();
    c.cancelled.store(true, Ordering::Relaxed);
    let out = root.path().join("new");
    assert!(install(&out, &["sin".into()], &c, &Activity::default()).is_err());
    assert!(!out.exists());
    let file = root.path().join("file");
    fs::write(&file, "preserve").unwrap();
    assert!(
        install(
            &file,
            &["sin".into()],
            &Control::default(),
            &Activity::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(file).unwrap(), b"preserve");
}
#[test]
#[ignore = "Requires NOTEGEN_LANGUAGE_EVIDENCE with actual publisher files; not a language accuracy benchmark"]
fn optional_actual_pinned_files_match_all_lengths_and_checksums() {
    let root = std::env::var_os("NOTEGEN_LANGUAGE_EVIDENCE")
        .expect("Set NOTEGEN_LANGUAGE_EVIDENCE to the verified resource folder");
    let root = std::path::Path::new(&root);
    let p = plan(
        root,
        &[
            "eng".into(),
            "sin".into(),
            "tam".into(),
            "ara".into(),
            "heb".into(),
        ],
        &Control::default(),
    )
    .unwrap();
    assert!(p.files.iter().all(|f| f.verified));
    assert_eq!(p.remaining_bytes, 0);
    verify_selection(root, "sin+tam+eng", &Control::default()).unwrap();
}
