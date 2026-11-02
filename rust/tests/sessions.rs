use notegen::{
    sessions::{self, Session},
    storage::{Activity, Control},
    types::*,
};
use std::{fs, sync::atomic::Ordering};

#[test]
fn marked_sessions_copy_sources_without_changing_or_adopting_originals() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("private-source.png");
    image::RgbImage::from_pixel(4, 5, image::Rgb([30, 40, 50]))
        .save(&source)
        .unwrap();
    let bytes = fs::read(&source).unwrap();
    let session = Session::create(temp.path(), &Control::default()).unwrap();
    let copied = sessions::copy_source(&session, &source, &Control::default()).unwrap();
    assert_eq!(fs::read(copied).unwrap(), bytes);
    assert_eq!(fs::read(&source).unwrap(), bytes);
    let reopened = Session::read(&session.root).unwrap();
    let preview = sessions::inspect(&reopened, &Control::default()).unwrap();
    assert_eq!(preview.phase, "prepared");
    assert_eq!(preview.files.len(), 1);
    assert!(!preview.export_verified);
    assert!(preview.note.contains("not encryption"));
    assert!(preview.note.contains("unmanaged"));
    assert!(
        sessions::discard(
            &session,
            &preview,
            &[preview.files[0].path.clone()],
            &Control::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(&source).unwrap(), bytes);
    let arbitrary = temp.path().join("ordinary-project");
    fs::create_dir(&arbitrary).unwrap();
    assert!(Session::read(&arbitrary).is_err());
}

#[test]
fn failure_and_cancellation_keep_retained_artifacts_and_original_input() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("broken.png");
    fs::write(&source, b"Invalid image fixture").unwrap();
    let session = Session::create(temp.path(), &Control::default()).unwrap();
    let settings = Settings {
        preset: Preset::Fast,
        models_dir: temp.path().join("no-models"),
        ..Default::default()
    };
    let report = sessions::run(
        &session,
        &source,
        &settings,
        &Control::default(),
        &Activity::default(),
    )
    .unwrap();
    assert!(!report.success);
    assert!(report.error.is_some());
    assert_eq!(report.retention.as_ref().unwrap().phase, "failed");
    assert_eq!(report.retention.as_ref().unwrap().files.len(), 1);
    assert_eq!(fs::read(&source).unwrap(), b"Invalid image fixture");
    let c = Control::default();
    c.cancelled.store(true, Ordering::Relaxed);
    assert!(
        sessions::copy_source(
            &Session::create(temp.path(), &Control::default()).unwrap(),
            &source,
            &c
        )
        .is_err()
    );
    assert_eq!(fs::read(&source).unwrap(), b"Invalid image fixture");
}

#[test]
fn private_sessions_reject_external_servers_and_marker_relocation() {
    let temp = tempfile::tempdir().unwrap();
    let session = Session::create(temp.path(), &Control::default()).unwrap();
    let settings = Settings {
        external_server: true,
        ..Default::default()
    };
    assert!(
        sessions::run(
            &session,
            &temp.path().join("absent.pdf"),
            &settings,
            &Control::default(),
            &Activity::default()
        )
        .is_err()
    );
    assert_eq!(
        sessions::inspect(&session, &Control::default())
            .unwrap()
            .phase,
        "created"
    );
    let moved = temp.path().join("notegen-session-relocated");
    fs::rename(&session.root, &moved).unwrap();
    assert!(Session::read(&moved).is_err());
}

#[test]
fn forged_inventory_cannot_reach_outside_the_marked_session() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original.png");
    image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3]))
        .save(&original)
        .unwrap();
    let session = Session::create(temp.path(), &Control::default()).unwrap();
    sessions::copy_source(&session, &original, &Control::default()).unwrap();
    let before = fs::read(&original).unwrap();
    let state_path = session.root.join("retention.json");
    let mut state: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    state["owned"][0]["path"] = "../original.png".into();
    fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
    assert!(sessions::inspect(&session, &Control::default()).is_err());
    assert_eq!(fs::read(&original).unwrap(), before);
}
