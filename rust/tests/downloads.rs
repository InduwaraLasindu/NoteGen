use chrono::{TimeZone, Utc};
use notegen::{
    downloads::{
        self, Gate, JobOverride, Manager, NetworkCost, NetworkPolicy, Policy, ReceiveBudget,
        Status, TransferSpec, UtcWindow,
    },
    models::Asset,
    storage::{Control, fingerprint},
    types::Fault,
};
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

fn now(day: u32, hour: u32, minute: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, day, hour, minute, 0)
        .unwrap()
}
fn spec(root: &Path, name: &str) -> TransferSpec {
    TransferSpec {
        url: format!(
            "https://huggingface.co/notegen/tests/resolve/{}/{}.gguf",
            "a".repeat(40),
            name
        ),
        target: root.join(format!("{name}.gguf")),
        asset: Asset {
            file: format!("{name}.gguf"),
            bytes: 32,
            sha256: "b".repeat(64),
        },
    }
}

#[test]
fn utc_windows_have_exact_boundaries_and_cross_midnight_on_the_starting_weekday() {
    let policy = Policy {
        utc_windows: vec![UtcWindow {
            weekdays: vec![0],
            start_minute: 600,
            end_minute: 660,
        }],
        ..Default::default()
    };
    policy.validate().unwrap();
    assert!(!policy.window_open(now(2, 9, 59)));
    assert!(policy.window_open(now(2, 10, 0)));
    assert!(policy.window_open(now(2, 10, 59)));
    assert!(!policy.window_open(now(2, 11, 0)));
    assert!(!policy.window_open(now(3, 10, 0)));
    let overnight = Policy {
        utc_windows: vec![UtcWindow {
            weekdays: vec![0],
            start_minute: 23 * 60,
            end_minute: 60,
        }],
        ..Default::default()
    };
    assert!(overnight.window_open(now(2, 23, 0)));
    assert!(overnight.window_open(now(3, 0, 59)));
    assert!(!overnight.window_open(now(3, 1, 0)));
    assert!(!overnight.window_open(now(3, 23, 0)));
    for invalid in [
        UtcWindow {
            weekdays: vec![],
            start_minute: 1,
            end_minute: 2,
        },
        UtcWindow {
            weekdays: vec![7],
            start_minute: 1,
            end_minute: 2,
        },
        UtcWindow {
            weekdays: vec![0, 0],
            start_minute: 1,
            end_minute: 2,
        },
        UtcWindow {
            weekdays: vec![0],
            start_minute: 1440,
            end_minute: 2,
        },
        UtcWindow {
            weekdays: vec![0],
            start_minute: 1,
            end_minute: 1,
        },
    ] {
        assert!(
            Policy {
                utc_windows: vec![invalid],
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}

#[test]
fn network_and_per_job_override_policies_are_explicit_with_no_default_unknown_deadlock() {
    let defaults = Policy::default();
    for cost in [
        NetworkCost::Unknown,
        NetworkCost::Metered,
        NetworkCost::Unmetered,
    ] {
        assert_eq!(
            downloads::policy_gate(
                &defaults,
                &JobOverride::default(),
                false,
                now(2, 10, 0),
                cost
            )
            .unwrap(),
            Gate::Ready
        );
    }
    let policy = Policy {
        metered: NetworkPolicy::Pause,
        unknown: NetworkPolicy::Pause,
        ..Default::default()
    };
    for cost in [NetworkCost::Unknown, NetworkCost::Metered] {
        assert_eq!(
            downloads::policy_gate(&policy, &JobOverride::default(), false, now(2, 10, 0), cost)
                .unwrap(),
            Gate::Wait(Status::WaitingNetwork)
        );
    }
    let over = JobOverride {
        allow_metered: true,
        allow_unknown: true,
        ignore_window: true,
    };
    assert_eq!(
        downloads::policy_gate(&policy, &over, false, now(2, 10, 0), NetworkCost::Metered).unwrap(),
        Gate::Ready
    );
    assert_eq!(
        downloads::policy_gate(&policy, &over, true, now(2, 10, 0), NetworkCost::Unknown).unwrap(),
        Gate::Wait(Status::Paused)
    );
}

#[test]
fn receive_budget_is_global_bounded_and_uses_monotonic_debt() {
    let mut budget = ReceiveBudget::default();
    assert_eq!(budget.wait_ms(100), 0);
    budget.charge(1000, 100, 1000).unwrap();
    assert_eq!(budget.wait_ms(100), 1000);
    // A second transfer charges the same budget, not a fresh per-job allowance.
    budget.charge(500, 200, 1000).unwrap();
    assert_eq!(budget.wait_ms(200), 1400);
    assert_eq!(budget.wait_ms(1600), 0);
    assert!(
        budget
            .charge(downloads::MAX_RECEIVE_CHUNK + 1, 1600, 1000)
            .is_err()
    );
    budget.charge(1, 2000, 0).unwrap();
    assert_eq!(budget.wait_ms(2000), 0);
    budget.charge(1, 2000, 1).unwrap();
    assert_eq!(budget.wait_ms(1999), 1001);
}

#[test]
fn queue_persists_pause_cancel_overrides_and_requeues_interrupted_receipts() {
    let root = tempfile::tempdir().unwrap();
    let manager = Manager::open(&root.path().join("manager")).unwrap();
    let a = manager.enqueue(spec(root.path(), "first")).unwrap();
    let b = manager.enqueue(spec(root.path(), "second")).unwrap();
    let c = manager.enqueue(spec(root.path(), "third")).unwrap();
    assert_eq!(
        manager.enqueue(spec(root.path(), "first")).unwrap().id,
        a.id
    );
    manager.pause(&a.id).unwrap();
    manager.cancel(&b.id).unwrap();
    manager
        .set_override(
            &c.id,
            JobOverride {
                allow_metered: true,
                ..Default::default()
            },
        )
        .unwrap();
    fs::write(root.path().join("third.gguf.part"), b"12345678").unwrap();
    assert!(Manager::open(&root.path().join("manager")).is_err());
    drop(manager);
    let store_path = root.path().join("manager/jobs.json");
    let mut store: serde_json::Value =
        serde_json::from_slice(&fs::read(&store_path).unwrap()).unwrap();
    store["jobs"][2]["status"] = "downloading".into();
    store["jobs"][2]["received_bytes"] = 8.into();
    fs::write(&store_path, serde_json::to_vec(&store).unwrap()).unwrap();
    let manager = Manager::open(&root.path().join("manager")).unwrap();
    let jobs = manager.list().unwrap();
    assert_eq!(jobs[0].status, Status::Paused);
    assert_eq!(jobs[1].status, Status::Cancelled);
    assert_eq!(jobs[2].status, Status::Queued);
    assert_eq!(jobs[2].received_bytes, 8);
    assert!(jobs[2].override_policy.allow_metered);
    manager.resume(&b.id).unwrap();
    assert_eq!(manager.list().unwrap()[1].status, Status::Queued);
    fs::write(root.path().join("second.gguf.part"), b"retained partial").unwrap();
    manager.forget(&b.id).unwrap();
    assert!(root.path().join("second.gguf.part").exists());
    assert_eq!(manager.list().unwrap().len(), 2);
}

#[test]
fn queue_rejects_secret_urls_registration_paths_duplicate_destinations_and_tampering() {
    let root = tempfile::tempdir().unwrap();
    let manager = Manager::open(&root.path().join("manager")).unwrap();
    for url in [
        "https://github.com/file?token=PRIVATE_SYNTHETIC",
        "https://user:PRIVATE_SYNTHETIC@github.com/file",
        "http://github.com/file",
        "https://example.com/file",
        "https://github.com/file#PRIVATE_SYNTHETIC",
    ] {
        let mut candidate = spec(root.path(), "bad");
        candidate.url = url.into();
        assert!(manager.enqueue(candidate).is_err());
    }
    let mut candidate = spec(root.path(), "bad");
    candidate.target = root.path().join("reader.notegen.json");
    assert!(manager.enqueue(candidate).is_err());
    let first = manager.enqueue(spec(root.path(), "first")).unwrap();
    let mut duplicate = first.transfer.clone();
    duplicate.asset.sha256 = "c".repeat(64);
    assert!(manager.enqueue(duplicate).is_err());
    let json = fs::read_to_string(root.path().join("manager/jobs.json")).unwrap();
    assert!(!json.contains("PRIVATE_SYNTHETIC") && !json.contains("authorization"));
    drop(manager);
    let path = root.path().join("manager/jobs.json");
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["jobs"][0]["received_bytes"] = 33.into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(Manager::open(&root.path().join("manager")).is_err());
}

#[test]
fn dedicated_transfer_cancellation_interrupts_global_pause_without_cancelling_parent() {
    let control = Control {
        transfer_cancelled: Some(Arc::new(AtomicBool::new(true))),
        ..Default::default()
    };
    control.paused.store(true, Ordering::Relaxed);
    let error = control.check().unwrap_err();
    assert_eq!(error.downcast_ref::<Fault>().unwrap().code, "NG900");
    assert!(!control.cancelled.load(Ordering::Relaxed));
}

#[test]
fn resumed_standalone_job_never_changes_installed_registration() {
    let root = tempfile::tempdir().unwrap();
    let manager = Manager::open(&root.path().join("manager")).unwrap();
    let registration = root.path().join("reader.notegen.json");
    fs::write(&registration, b"original active registration").unwrap();
    let before = fs::read(&registration).unwrap();
    let job = manager.enqueue(spec(root.path(), "download-only")).unwrap();
    manager.pause(&job.id).unwrap();
    let result = manager
        .run_pending(&Control::default(), &Default::default())
        .unwrap();
    assert_eq!(result.completed, 0);
    assert_eq!(result.failed, 0);
    assert_eq!(fs::read(registration).unwrap(), before);
    assert!(!job.transfer.target.exists());
    assert_eq!(fingerprint(&manager.list().unwrap()).unwrap().len(), 64);
}

#[test]
fn explicit_enqueue_revalidates_completed_staging_after_consumption_but_preserves_user_stops() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("manager");
    let candidate = spec(root.path(), "consumed");
    let manager = Manager::open(&folder).unwrap();
    let job = manager.enqueue(candidate.clone()).unwrap();
    drop(manager);
    let state = folder.join("jobs.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&state).unwrap()).unwrap();
    value["jobs"][0]["status"] = "complete".into();
    value["jobs"][0]["received_bytes"] = 32.into();
    fs::write(&state, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::write(&candidate.target, [1u8; 32]).unwrap();
    let final_file = root.path().join("activated.gguf");
    fs::rename(&candidate.target, &final_file).unwrap();
    let manager = Manager::open(&folder).unwrap();
    assert_eq!(manager.list().unwrap()[0].status, Status::Complete);
    let queued = manager.enqueue(candidate.clone()).unwrap();
    assert_eq!(queued.id, job.id);
    assert_eq!(queued.status, Status::Queued);
    assert_eq!(queued.received_bytes, 0);
    assert!(!candidate.target.exists());
    assert_eq!(fs::read(final_file).unwrap(), [1u8; 32]);
    manager.pause(&job.id).unwrap();
    assert_eq!(
        manager.enqueue(candidate.clone()).unwrap().status,
        Status::Paused
    );
    manager.cancel(&job.id).unwrap();
    assert_eq!(
        manager.enqueue(candidate).unwrap().status,
        Status::Cancelled
    );
}

#[cfg(unix)]
#[test]
fn manager_and_transfer_paths_reject_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let actual = root.path().join("actual");
    fs::create_dir(&actual).unwrap();
    let linked = root.path().join("linked");
    std::os::unix::fs::symlink(&actual, &linked).unwrap();
    assert!(Manager::open(&linked).is_err());
    let manager = Manager::open(&root.path().join("manager")).unwrap();
    let mut candidate = spec(root.path(), "bad");
    candidate.target = linked.join("asset.gguf");
    assert!(manager.enqueue(candidate).is_err());
}
