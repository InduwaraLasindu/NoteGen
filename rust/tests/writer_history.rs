use image::{DynamicImage, GrayImage, Luma};
use notegen::{
    profile_tools,
    storage::{Control, atomic_json},
    types::*,
    writer_history as history, writers,
};
use std::fs;
fn ink() -> DynamicImage {
    let mut image = GrayImage::from_pixel(50, 70, Luma([255]));
    for y in 10..60 {
        for x in 10..25 {
            image[(x, y)] = Luma([0]);
        }
    }
    DynamicImage::ImageLuma8(image)
}
#[test]
fn learning_quarantine_forget_and_recovery_survive_restarts_with_monotonic_revision() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let p = writers::create(root, "Example").unwrap();
    let p =
        writers::add_example(root, &p.id, &ink(), BoxRect::full(), "Ж", true, "confirmed").unwrap();
    let sample = p.samples[0].clone();
    let p = profile_tools::quarantine(root, &p.id, &sample.id, "Wrong label").unwrap();
    let p = writers::delete_example(root, &p.id, &sample.id).unwrap();
    assert!(p.samples.is_empty());
    assert!(root.join(&p.id).join(&sample.image).is_file());
    assert!(writers::reference(root, &p.id).unwrap().is_none());
    let undo = history::undo(root, &p.id).unwrap();
    assert!(undo.revision > p.revision);
    assert!(undo.samples[0].quarantine.is_some());
    let undo2 = history::undo(root, &p.id).unwrap();
    assert!(undo2.revision > undo.revision);
    assert!(undo2.samples[0].quarantine.is_none());
    assert!(writers::reference(root, &p.id).unwrap().is_some());
    let undo3 = history::undo(root, &p.id).unwrap();
    assert!(undo3.samples.is_empty());
    assert!(history::undo(root, &p.id).is_err());
    let p = history::redo(root, &p.id).unwrap();
    assert_eq!(p.samples[0].label, "Ж");
    history::redo(root, &p.id).unwrap();
    let p = history::redo(root, &p.id).unwrap();
    assert!(p.samples.is_empty());
    assert!(history::redo(root, &p.id).is_err());
    assert_eq!(history::timeline(root, &p.id).unwrap().cursor, 3);
}
#[test]
fn new_learning_replaces_redo_branch_and_exports_exclude_retained_inactive_samples() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let p = writers::create(root, "Branch").unwrap();
    let p =
        writers::add_example(root, &p.id, &ink(), BoxRect::full(), "A", true, "private").unwrap();
    let orphan = p.samples[0].image.clone();
    history::undo(root, &p.id).unwrap();
    let p =
        writers::add_example(root, &p.id, &ink(), BoxRect::full(), "B", true, "confirmed").unwrap();
    assert!(history::redo(root, &p.id).is_err());
    assert!(root.join(&p.id).join(&orphan).is_file());
    let output = root.join("profile.zip");
    writers::export_profile(root, &p.id, &output, &Control::default()).unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(output).unwrap()).unwrap();
    assert!(zip.by_name(&format!("samples/{orphan}")).is_err());
    assert!(zip.by_name("writer-history.json").is_err());
    assert!(
        zip.by_name(&format!("samples/{}", p.samples[0].image))
            .is_ok()
    );
}
#[test]
fn outside_profile_changes_and_changed_retained_pixels_are_never_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let p = writers::create(root, "Stale").unwrap();
    let p =
        writers::add_example(root, &p.id, &ink(), BoxRect::full(), "A", true, "confirmed").unwrap();
    let path = root.join(&p.id).join("profile.json");
    let mut external = p.clone();
    external.notes = "outside edit".into();
    atomic_json(&path, &external).unwrap();
    let before = fs::read(&path).unwrap();
    assert!(history::undo(root, &p.id).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    atomic_json(&path, &p).unwrap();
    writers::delete_example(root, &p.id, &p.samples[0].id).unwrap();
    let image = root.join(&p.id).join(&p.samples[0].image);
    fs::write(&image, b"changed pixels").unwrap();
    let before = fs::read(&path).unwrap();
    assert!(history::undo(root, &p.id).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(image).unwrap(), b"changed pixels");
}
#[test]
fn history_rolls_at_64_edits_without_preventing_further_learning() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let p = writers::create(root, "Many edits").unwrap();
    for index in 0..70 {
        writers::update(root, &p.id, "Many edits", &index.to_string()).unwrap();
    }
    let timeline = history::timeline(root, &p.id).unwrap();
    assert_eq!(timeline.labels.len(), 65);
    assert_eq!(timeline.cursor, 64);
    for _ in 0..64 {
        history::undo(root, &p.id).unwrap();
    }
    assert_eq!(writers::read(root, &p.id).unwrap().notes, "5");
    assert!(history::undo(root, &p.id).is_err());
    writers::update(root, &p.id, "Many edits", "new branch").unwrap();
    assert!(history::redo(root, &p.id).is_err());
}
