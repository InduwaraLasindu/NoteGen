use notegen::{migrations, storage::Control};
use std::{fs, sync::atomic::Ordering};
#[test]
fn supported_settings_migration_has_exact_backup_and_byte_exact_rollback() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("settings.json");
    let original = b"{\n  \"preset\": \"balanced\", \"handwriting\": true\n}\n";
    fs::write(&path, original).unwrap();
    let plan = migrations::preview_settings(&path, &Control::default()).unwrap();
    assert!(migrations::apply(&plan, "wrong", &Control::default()).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    let receipt = migrations::apply(&plan, &plan.token, &Control::default()).unwrap();
    let after: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(after["handwriting"], true);
    assert_eq!(after["preset"], "balanced");
    assert!(after.get("experience").is_some());
    assert!(receipt.backup.join("before-0").is_file());
    let rollback = migrations::rollback_preview(&receipt, &Control::default()).unwrap();
    migrations::apply(&rollback, &rollback.token, &Control::default()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(receipt.backup.join("after-0").is_file());
}
#[test]
fn migration_rejects_stale_preview_later_edits_unknown_fields_and_cancellation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("settings.json");
    fs::write(&path, b"{\"preset\":\"fast\"}").unwrap();
    let plan = migrations::preview_settings(&path, &Control::default()).unwrap();
    let cancel = Control::default();
    cancel.cancelled.store(true, Ordering::Relaxed);
    assert!(migrations::apply(&plan, &plan.token, &cancel).is_err());
    assert!(!root.path().join("project-transaction.json").exists());
    fs::write(&path, b"{\"preset\":\"balanced\",\"handwriting\":true}").unwrap();
    let concurrent = fs::read(&path).unwrap();
    assert!(migrations::apply(&plan, &plan.token, &Control::default()).is_err());
    assert_eq!(fs::read(&path).unwrap(), concurrent);
    let fresh = migrations::preview_settings(&path, &Control::default()).unwrap();
    let receipt = migrations::apply(&fresh, &fresh.token, &Control::default()).unwrap();
    fs::write(
        &path,
        b"{\"preset\":\"fast\",\"subject\":\"concurrent user text\"}",
    )
    .unwrap();
    assert!(migrations::rollback_preview(&receipt, &Control::default()).is_err());
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("concurrent user text")
    );
    fs::write(&path, b"{\"future_unrecognized_setting\":1250}").unwrap();
    assert!(migrations::preview_settings(&path, &Control::default()).is_err());
}
