use notegen::{cleanup, storage::Control, types::*};
use std::{fs, sync::atomic::Ordering};
#[test]
fn cleanup_never_classifies_user_data_partial_downloads_or_unverified_archives_as_reclaimable() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = dir.path().join("runtime");
    let cache = runtime.join("downloads");
    fs::create_dir_all(&cache).unwrap();
    for name in [
        "user-scans.zip",
        "page.png",
        "profile.json",
        "weights.gguf",
        "llama-b12032-bin-win-cpu-x64.zip",
        "llama-b12032-bin-win-cpu-x64.zip.part",
    ] {
        fs::write(cache.join(name), b"private test data").unwrap();
    }
    let settings = Settings {
        server: runtime.join("llama-server.exe"),
        models_dir: dir.path().join("models"),
        ..Default::default()
    };
    let preview = cleanup::preview(
        &settings,
        &[dir.path().join("projects")],
        &Control::default(),
    )
    .unwrap();
    assert!(preview.files.is_empty());
    assert_eq!(preview.reclaimable_bytes, 0);
    assert!(preview.note.contains("partial downloads"));
    for name in [
        "user-scans.zip",
        "page.png",
        "profile.json",
        "weights.gguf",
        "llama-b12032-bin-win-cpu-x64.zip",
    ] {
        assert!(cache.join(name).exists());
    }
}
#[test]
fn cleanup_rejects_unpreviewed_paths_invalid_tokens_duplicate_selection_and_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let private = dir.path().join("private-scan.png");
    fs::write(&private, b"keep").unwrap();
    let settings = Settings {
        server: dir.path().join("runtime/llama-server.exe"),
        models_dir: dir.path().join("models"),
        ..Default::default()
    };
    let c = Control::default();
    let preview = cleanup::preview(&settings, &[], &c).unwrap();
    assert!(
        cleanup::apply(
            &settings,
            &[],
            std::slice::from_ref(&private),
            &preview.token,
            &c
        )
        .is_err()
    );
    assert!(
        cleanup::apply(
            &settings,
            &[],
            &[private.clone(), private.clone()],
            &preview.token,
            &c
        )
        .is_err()
    );
    assert!(cleanup::apply(&settings, &[], std::slice::from_ref(&private), "stale", &c).is_err());
    assert!(cleanup::apply(&settings, &[], &[], &preview.token, &c).is_err());
    assert_eq!(fs::read(&private).unwrap(), b"keep");
    c.cancelled.store(true, Ordering::Relaxed);
    assert!(cleanup::preview(&settings, &[], &c).is_err());
    assert!(cleanup::apply(&settings, &[], &[private], &preview.token, &c).is_err());
}
#[test]
fn malformed_active_or_rollback_registry_fails_closed_before_any_cache_removal() {
    let dir = tempfile::tempdir().unwrap();
    let models = dir.path().join("models");
    fs::create_dir_all(&models).unwrap();
    fs::write(
        models.join("reader.previous.json"),
        b"corrupted registration",
    )
    .unwrap();
    let settings = Settings {
        models_dir: models,
        server: dir.path().join("runtime/llama-server.exe"),
        ..Default::default()
    };
    assert!(cleanup::preview(&settings, &[], &Control::default()).is_err());
}
#[cfg(unix)]
#[test]
fn cleanup_does_not_follow_a_symlinked_cache_root_or_known_archive() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let runtime = dir.path().join("runtime");
    fs::create_dir_all(&runtime).unwrap();
    std::os::unix::fs::symlink(outside.path(), runtime.join("downloads")).unwrap();
    let settings = Settings {
        server: runtime.join("llama-server.exe"),
        models_dir: dir.path().join("models"),
        ..Default::default()
    };
    assert!(cleanup::preview(&settings, &[], &Control::default()).is_err());
    fs::remove_file(runtime.join("downloads")).unwrap();
    fs::create_dir(runtime.join("downloads")).unwrap();
    let source = outside.path().join("private.zip");
    fs::write(&source, b"private").unwrap();
    std::os::unix::fs::symlink(
        &source,
        runtime.join("downloads/llama-b12032-bin-win-cpu-x64.zip"),
    )
    .unwrap();
    assert!(cleanup::preview(&settings, &[], &Control::default()).is_err());
    assert_eq!(fs::read(source).unwrap(), b"private");
}
