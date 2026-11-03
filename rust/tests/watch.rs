use notegen::{
    storage::Control,
    watch::{self, DuplicatePolicy, WatchConfig, WatchState},
};
use std::{fs, sync::atomic::Ordering};
fn config(root: &std::path::Path) -> WatchConfig {
    WatchConfig {
        folder: root.into(),
        stable_seconds: 3,
        ..Default::default()
    }
}
#[test]
fn watched_files_must_stabilize_and_failed_admission_is_not_silently_consumed() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let mut state = WatchState::default();
    let c = Control::default();
    fs::write(dir.path().join("page.PNG"), b"scan one").unwrap();
    assert_eq!(
        watch::scan(&config, &mut state, 10, &c)
            .unwrap()
            .waiting_stable,
        vec!["page.PNG"]
    );
    assert!(
        watch::scan(&config, &mut state, 12, &c)
            .unwrap()
            .ready
            .is_empty()
    );
    let ready = watch::scan(&config, &mut state, 13, &c).unwrap().ready;
    assert_eq!(ready.len(), 1);
    assert!(state.imported.is_empty());
    assert_eq!(
        watch::scan(&config, &mut state, 14, &c)
            .unwrap()
            .ready
            .len(),
        1
    );
    watch::record_import(&config, &mut state, &ready[0], "admitted-job", 14, &c).unwrap();
    assert_eq!(state.imported.len(), 1);
    assert!(
        watch::scan(&config, &mut state, 15, &c)
            .unwrap()
            .ready
            .is_empty()
    );
    let stored = serde_json::to_vec(&state).unwrap();
    let restored: WatchState = serde_json::from_slice(&stored).unwrap();
    assert_eq!(restored.imported["page.PNG"].project_id, "admitted-job");
}
#[test]
fn changed_files_restart_stability_and_stale_candidates_cannot_be_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let mut state = WatchState::default();
    let c = Control::default();
    let path = dir.path().join("page.pdf");
    fs::write(&path, b"first").unwrap();
    watch::scan(&config, &mut state, 0, &c).unwrap();
    let old = watch::scan(&config, &mut state, 3, &c)
        .unwrap()
        .ready
        .remove(0);
    fs::write(&path, b"new and longer").unwrap();
    assert!(watch::record_import(&config, &mut state, &old, "job", 4, &c).is_err());
    assert!(state.imported.is_empty());
    assert_eq!(
        watch::scan(&config, &mut state, 4, &c)
            .unwrap()
            .waiting_stable
            .len(),
        1
    );
    assert!(
        watch::scan(&config, &mut state, 6, &c)
            .unwrap()
            .ready
            .is_empty()
    );
    assert_eq!(
        watch::scan(&config, &mut state, 7, &c).unwrap().ready.len(),
        1
    );
}
#[test]
fn preserved_size_and_mtime_never_hide_changed_imported_content_under_either_policy() {
    for policy in [DuplicatePolicy::Skip, DuplicatePolicy::NewRevision] {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config(dir.path());
        config.duplicate_policy = policy;
        let path = dir.path().join("page.png");
        fs::write(&path, b"AAAA").unwrap();
        let mut state = WatchState::default();
        let c = Control::default();
        watch::scan(&config, &mut state, 0, &c).unwrap();
        let original = watch::scan(&config, &mut state, 3, &c)
            .unwrap()
            .ready
            .remove(0);
        watch::record_import(&config, &mut state, &original, "original", 3, &c).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, b"BBBB").unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), original.bytes);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        let changed = watch::scan(&config, &mut state, 4, &c).unwrap();
        assert!(changed.ready.is_empty());
        assert_eq!(changed.waiting_stable, ["page.png"]);
        assert_eq!(state.imported["page.png"].sha256, original.sha256);
        assert!(
            watch::scan(&config, &mut state, 6, &c)
                .unwrap()
                .ready
                .is_empty()
        );
        let replacement = watch::scan(&config, &mut state, 7, &c)
            .unwrap()
            .ready
            .remove(0);
        assert_ne!(replacement.sha256, original.sha256);
        assert!(watch::record_import(&config, &mut state, &original, "stale", 7, &c).is_err());
        watch::record_import(&config, &mut state, &replacement, "replacement", 7, &c).unwrap();
        assert_eq!(state.imported["page.png"].project_id, "replacement");
        assert!(
            watch::scan(&config, &mut state, 8, &c)
                .unwrap()
                .ready
                .is_empty()
        );
    }
}
#[test]
fn unimported_ready_candidates_are_rehashed_when_metadata_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let config = WatchConfig {
        stable_seconds: 0,
        ..config(dir.path())
    };
    let path = dir.path().join("page.pdf");
    fs::write(&path, b"AAAA").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let mut state = WatchState::default();
    let c = Control::default();
    let original = watch::scan(&config, &mut state, 0, &c)
        .unwrap()
        .ready
        .remove(0);
    fs::write(&path, b"BBBB").unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let replacement = watch::scan(&config, &mut state, 1, &c)
        .unwrap()
        .ready
        .remove(0);
    assert_ne!(replacement.sha256, original.sha256);
    assert!(state.imported.is_empty());
}
#[test]
fn duplicate_content_policy_skips_or_admits_an_explicit_new_revision_under_another_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(dir.path());
    config.stable_seconds = 0;
    let mut state = WatchState::default();
    let c = Control::default();
    fs::write(dir.path().join("original.png"), b"identical scan").unwrap();
    let candidate = watch::scan(&config, &mut state, 0, &c)
        .unwrap()
        .ready
        .remove(0);
    watch::record_import(&config, &mut state, &candidate, "original-project", 0, &c).unwrap();
    fs::write(dir.path().join("renamed.jpg"), b"identical scan").unwrap();
    let report = watch::scan(&config, &mut state, 1, &c).unwrap();
    assert!(report.ready.is_empty());
    assert_eq!(report.skipped_duplicates, vec!["renamed.jpg"]);
    assert_eq!(state.imported.len(), 1);
    config.duplicate_policy = DuplicatePolicy::NewRevision;
    let candidate = watch::scan(&config, &mut state, 2, &c)
        .unwrap()
        .ready
        .remove(0);
    assert_eq!(
        candidate.duplicate_of.as_ref().unwrap().project_id,
        "original-project"
    );
    watch::record_import(&config, &mut state, &candidate, "separate-revision", 2, &c).unwrap();
    assert!(
        watch::scan(&config, &mut state, 3, &c)
            .unwrap()
            .ready
            .is_empty()
    );
}
#[test]
fn watchers_do_not_recurse_or_import_unselected_empty_or_oversized_files_and_pause_is_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(dir.path());
    config.stable_seconds = 0;
    config.max_file_bytes = 10;
    let mut state = WatchState::default();
    let c = Control::default();
    fs::create_dir(dir.path().join("nested")).unwrap();
    fs::write(dir.path().join("nested/hidden.pdf"), b"scan").unwrap();
    fs::write(dir.path().join("wrong.exe"), b"scan").unwrap();
    fs::write(dir.path().join("empty.pdf"), b"").unwrap();
    fs::write(dir.path().join("large.tif"), [1; 11]).unwrap();
    fs::write(dir.path().join("good.jpeg"), b"scan").unwrap();
    config.paused = true;
    let report = watch::scan(&config, &mut state, 0, &c).unwrap();
    assert!(report.paused);
    assert!(report.ready.is_empty());
    assert!(state.observations.is_empty());
    config.paused = false;
    let report = watch::scan(&config, &mut state, 1, &c).unwrap();
    assert_eq!(report.ready.len(), 1);
    assert_eq!(report.ready[0].filename, "good.jpeg");
    assert_eq!(report.rejected.len(), 2);
    assert!(!state.observations.contains_key("hidden.pdf"));
    config.paused = true;
    assert!(watch::record_import(&config, &mut state, &report.ready[0], "job", 1, &c).is_err());
}
#[test]
fn watch_state_is_bound_to_one_folder_and_backward_clock_waits_again() {
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    let config = config(one.path());
    let mut state = WatchState::default();
    let c = Control::default();
    fs::write(one.path().join("page.png"), b"scan").unwrap();
    watch::scan(&config, &mut state, 100, &c).unwrap();
    assert!(
        watch::scan(&config, &mut state, 90, &c)
            .unwrap()
            .ready
            .is_empty()
    );
    assert!(
        watch::scan(&config, &mut state, 92, &c)
            .unwrap()
            .ready
            .is_empty()
    );
    assert_eq!(
        watch::scan(&config, &mut state, 93, &c)
            .unwrap()
            .ready
            .len(),
        1
    );
    let other = WatchConfig {
        folder: two.path().into(),
        ..config
    };
    assert!(watch::scan(&other, &mut state, 101, &c).is_err());
}
#[test]
fn forged_outside_paths_tokens_invalid_rules_and_cancellation_fail_without_import_records() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(dir.path());
    config.stable_seconds = 0;
    let mut state = WatchState::default();
    let c = Control::default();
    fs::write(dir.path().join("page.pdf"), b"scan").unwrap();
    let candidate = watch::scan(&config, &mut state, 0, &c)
        .unwrap()
        .ready
        .remove(0);
    let mut forged = candidate.clone();
    forged.observation_token = "fake".into();
    assert!(watch::record_import(&config, &mut state, &forged, "job", 0, &c).is_err());
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("page.pdf"), b"scan").unwrap();
    forged = candidate;
    forged.path = outside.path().join("page.pdf");
    assert!(watch::record_import(&config, &mut state, &forged, "job", 0, &c).is_err());
    assert!(state.imported.is_empty());
    config.extensions = vec!["exe".into()];
    assert!(watch::scan(&config, &mut state, 1, &c).is_err());
    config.extensions = vec!["pdf".into()];
    c.cancelled.store(true, Ordering::Relaxed);
    assert!(watch::scan(&config, &mut state, 1, &c).is_err());
}
#[cfg(unix)]
#[test]
fn watched_symlinks_are_rejected_and_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("source.png"), b"private").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("source.png"),
        dir.path().join("link.png"),
    )
    .unwrap();
    let config = WatchConfig {
        stable_seconds: 0,
        ..config(dir.path())
    };
    let report = watch::scan(&config, &mut WatchState::default(), 0, &Control::default()).unwrap();
    assert!(report.ready.is_empty());
    assert_eq!(report.rejected.len(), 1);
}
