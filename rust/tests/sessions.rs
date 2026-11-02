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

#[test]
fn native_success_export_and_selected_cleanup_preserve_original_and_untracked_files() {
    use ab_glyph::FontArc;
    use notegen::storage::{Project, hash_file, read_json};
    use std::{
        collections::BTreeSet,
        io::Read,
        path::{Path, PathBuf},
    };

    let control = Control::default();
    let temp = tempfile::tempdir().unwrap();
    let font_path = [
        "C:/Windows/Fonts/arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]
    .into_iter()
    .find(|path| Path::new(path).is_file())
    .expect("Native session test requires a system font");
    let font = FontArc::try_from_vec(fs::read(font_path).unwrap()).unwrap();
    let source = temp.path().join("original-invoice.png");
    let mut image = image::RgbImage::from_pixel(1000, 650, image::Rgb([255, 255, 255]));
    imageproc::drawing::draw_text_mut(
        &mut image,
        image::Rgb([0, 0, 0]),
        40,
        60,
        52.,
        &font,
        "Invoice Total 1250",
    );
    image.save(&source).unwrap();
    let original_hash = hash_file(&source, &control).unwrap();
    let models = temp.path().join("empty-models");
    fs::create_dir(&models).unwrap();
    let settings = Settings {
        preset: Preset::Fast,
        models_dir: models,
        pdfium: std::env::var_os("NOTEGEN_PDFIUM")
            .map(PathBuf::from)
            .unwrap_or_else(|| Settings::default().pdfium),
        cpu_threads: 1,
        auto_rotate: false,
        deskew: false,
        ..Default::default()
    };
    let session = Session::create(temp.path(), &control).unwrap();
    let marker = session.root.join("notegen-session.json");
    let marker_bytes = fs::read(&marker).unwrap();
    let run = sessions::run(&session, &source, &settings, &control, &Activity::default()).unwrap();
    assert!(run.success, "{:?}", run.error);
    assert!(!run.cancelled);
    assert!(run.error.is_none());
    let project_path = run.project.as_ref().unwrap();
    let project = Project::read(project_path.clone()).unwrap();
    assert_eq!(project.manifest.pages, vec![1]);
    assert!(project.load(1).unwrap().content.plain().contains("1250"));
    assert_eq!(
        project_path.parent().unwrap(),
        session.root.join("work/.notegen")
    );
    assert_eq!(run.retention.as_ref().unwrap().phase, "converted");
    assert!(!run.retention.as_ref().unwrap().export_verified);
    assert_eq!(hash_file(&source, &control).unwrap(), original_hash);

    let external = temp.path().join("exported-invoice.docx");
    let receipt = sessions::export(&session, &external, &control).unwrap();
    assert_eq!(receipt.kind, "docx");
    assert_eq!(receipt.path, external.canonicalize().unwrap());
    assert_eq!(
        receipt.sha256,
        hash_file(run.output.as_ref().unwrap(), &control).unwrap()
    );
    let exported_hash = hash_file(&external, &control).unwrap();
    {
        let mut word = zip::ZipArchive::new(fs::File::open(&external).unwrap()).unwrap();
        let mut xml = String::new();
        word.by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        assert!(
            xml.contains("1250"),
            "Native OCR value is missing from exported Word"
        );
    }
    let manual = project_path.join("manual-note.txt");
    fs::write(&manual, b"Untracked user note must remain").unwrap();
    let relative_project = project_path.strip_prefix(&session.root).unwrap();
    let source_copy = PathBuf::from("source/page-00001.png");
    let preview = sessions::inspect(&session, &control).unwrap();
    assert!(preview.export_verified);
    assert_eq!(preview.export.as_ref(), Some(&receipt));
    let tracked: BTreeSet<_> = preview.files.iter().map(|file| file.path.clone()).collect();
    assert_eq!(
        preview
            .files
            .iter()
            .find(|file| file.path == source_copy)
            .unwrap()
            .sha256,
        original_hash
    );
    for expected in [
        source_copy.clone(),
        PathBuf::from("work/result.docx"),
        relative_project.join("project.json"),
        relative_project.join("page-00001.png"),
        relative_project.join("page-00001.json"),
        relative_project.join("preparation-report.json"),
        relative_project.join("activity.jsonl"),
        relative_project.join("hardware.json"),
        relative_project.join("plan.json"),
    ] {
        assert!(
            tracked.contains(&expected),
            "Actual conversion artifact was not tracked: {}",
            expected.display()
        );
    }
    for file in &preview.files {
        assert!(
            file.path == source_copy
                || file.path == Path::new("work/result.docx")
                || file.path.starts_with(relative_project)
        );
        assert_eq!(
            file.sha256,
            hash_file(&session.root.join(&file.path), &control).unwrap()
        );
        assert_eq!(
            file.bytes,
            fs::metadata(session.root.join(&file.path)).unwrap().len()
        );
    }
    assert!(
        preview
            .preserved
            .iter()
            .any(|file| file.path == relative_project.join("manual-note.txt"))
    );
    let preparation: serde_json::Value =
        read_json(&project_path.join("preparation-report.json"), 16_384).unwrap();
    assert_eq!(preparation["requested_mode"], "sequential");
    assert_eq!(preparation["enabled"], false);

    let selected = sessions::discard(
        &session,
        &preview,
        std::slice::from_ref(&source_copy),
        &control,
    )
    .unwrap();
    assert_eq!(selected.removed, vec![source_copy.clone()]);
    assert!(!selected.cancelled);
    assert!(!selected.recovery_required);
    assert!(!session.root.join(&source_copy).exists());
    assert!(project.page_path(1).is_file());
    assert!(manual.is_file());
    assert_eq!(hash_file(&source, &control).unwrap(), original_hash);
    assert_eq!(hash_file(&external, &control).unwrap(), exported_hash);

    let remaining = sessions::inspect(&session, &control).unwrap();
    assert!(remaining.export_verified);
    assert!(!remaining.files.is_empty());
    assert!(remaining.files.iter().all(|file| file.path != source_copy));
    let all_remaining: Vec<_> = remaining
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect();
    let expected_bytes: u64 = remaining.files.iter().map(|file| file.bytes).sum();
    let cleanup = sessions::discard(&session, &remaining, &all_remaining, &control).unwrap();
    assert_eq!(
        cleanup.removed.iter().collect::<BTreeSet<_>>(),
        all_remaining.iter().collect::<BTreeSet<_>>()
    );
    assert_eq!(cleanup.removed_bytes, expected_bytes);
    assert!(!cleanup.cancelled);
    assert!(!cleanup.recovery_required);
    assert!(
        all_remaining
            .iter()
            .all(|path| !session.root.join(path).exists())
    );
    assert_eq!(
        fs::read(&manual).unwrap(),
        b"Untracked user note must remain"
    );
    assert_eq!(fs::read(&marker).unwrap(), marker_bytes);
    assert!(session.root.join("retention.json").is_file());
    assert!(session.root.join("session.lock").is_file());
    assert_eq!(hash_file(&source, &control).unwrap(), original_hash);
    assert_eq!(hash_file(&external, &control).unwrap(), exported_hash);
    let reopened = Session::read(&session.root).unwrap();
    let final_preview = sessions::inspect(&reopened, &control).unwrap();
    assert_eq!(final_preview.phase, "discarded");
    assert!(final_preview.files.is_empty());
    assert!(final_preview.export_verified);
    assert!(
        final_preview
            .preserved
            .iter()
            .any(|file| file.path == relative_project.join("manual-note.txt"))
    );
    assert!(sessions::recover(&reopened, &control).unwrap().is_empty());
}
