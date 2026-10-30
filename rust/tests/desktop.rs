#![cfg(feature = "desktop")]
use eframe::egui;
use notegen::{desktop::*, storage::*, types::*};
use std::{fs, path::PathBuf};

#[test]
fn queue_recovers_interrupted_jobs_without_duplicates() {
    let mut state = Workspace::default();
    assert!(state.enqueue(PathBuf::from("scan.pdf")));
    assert!(!state.enqueue(PathBuf::from("scan.pdf")));
    assert!(!state.enqueue(PathBuf::from("document.docx")));
    state.queue[0].status = JobStatus::Running;
    state.recover();
    assert_eq!(state.queue[0].status, JobStatus::Pending);
    assert_eq!(state.next(), Some(0));
    state.queue[0].status = JobStatus::Complete;
    assert_eq!(state.next(), None);
    let encoded = serde_json::to_string(&state).unwrap();
    let decoded: Workspace = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.queue.len(), 1);
}
#[test]
fn settings_transaction_rejects_invalid_values_without_mutating() {
    let mut state = Workspace::default();
    let before = serde_json::to_string(&state.settings).unwrap();
    assert!(state.apply(r#"{"vram_fraction":1.5}"#).is_err());
    assert_eq!(serde_json::to_string(&state.settings).unwrap(), before);
    assert!(state.apply(r#"{"typo":true}"#).is_err());
    state
        .apply(r#"{"preset":"maximum","gpu_index":1,"ui":{"reduced_motion":true}}"#)
        .unwrap();
    assert_eq!(state.settings.preset, Preset::Maximum);
    assert!(state.settings.ui.reduced_motion);
}
#[test]
fn output_names_protect_previous_documents() {
    let dir = tempfile::tempdir().unwrap();
    let input = PathBuf::from("scan.pdf");
    fs::write(dir.path().join("scan.docx"), b"original").unwrap();
    fs::write(dir.path().join("scan-2.docx"), b"original").unwrap();
    assert_eq!(
        output_name(&input, dir.path()),
        dir.path().join("scan-3.docx")
    );
}
#[test]
fn manual_review_persists_edits_and_approval() {
    let dir = tempfile::tempdir().unwrap();
    let p = Project::open(
        dir.path().into(),
        Manifest {
            schema: 2,
            input_sha256: "test".into(),
            fingerprint: "test".into(),
            pages: vec![1],
            models: vec![],
            settings: Settings::default(),
        },
    )
    .unwrap();
    let mut page = Page {
        page: 1,
        image: "page-00001.png".into(),
        content: PageContent {
            blocks: vec![Block::text("Corrected 1250")],
            ..Default::default()
        },
        ..Default::default()
    };
    save_review(&p, &mut page, true).unwrap();
    let read = p.load(1).unwrap();
    assert!(read.edited && read.reviewed);
    assert!(!read.needs_review(0.9));
    assert_eq!(read.content.plain(), "Corrected 1250");
}
#[test]
fn native_ui_renders_light_dark_and_reduced_motion() {
    for dark in [true, false] {
        let mut state = Workspace::default();
        state.settings.ui.dark = dark;
        state.settings.ui.reduced_motion = true;
        state.enqueue("example.pdf".into());
        let mut app = Desktop::new(state);
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let mut result = ctx.run_ui(egui::RawInput::default(), |ui| app.render(ui));
            assert!(!result.shapes.is_empty());
            assert!(result.textures_delta.set.len() < 10);
            result.textures_delta.clear();
        }
    }
}
