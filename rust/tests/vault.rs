use notegen::{
    storage::{Control, Manifest, Project},
    types::Settings,
    vault::{self, Credentials, Options, SecretString},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, atomic::Ordering},
    time::Duration,
};

// Bound aggregate KDF memory even on high-core CI workers.
static CRYPTO: Mutex<()> = Mutex::new(());
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    project: PathBuf,
    key: PathBuf,
    container: PathBuf,
    passphrase: SecretString,
    options: Options,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let project = root.join("original ' & σ notes");
        Project::open(
            project.clone(),
            Manifest {
                schema: 2,
                input_sha256: "a".repeat(64),
                fingerprint: "b".repeat(64),
                pages: vec![],
                models: vec![],
                settings: Settings::default(),
            },
        )
        .unwrap();
        fs::create_dir(project.join("private subdir")).unwrap();
        fs::write(
            project.join("private subdir/medical-secret-1250.txt"),
            b"PRIVATE patient 1250\r\n\0exact bytes",
        )
        .unwrap();
        fs::write(project.join("project.lock"), []).unwrap();
        let key = root.join("recovery ' & key.agekey");
        let receipt = vault::generate_recovery_key(&key, &Control::default()).unwrap();
        assert!(receipt.private_permissions);
        assert!(receipt.note.contains("not encrypted"));
        let container = root.join("notes.ngvault");
        Self {
            _temp: temp,
            root,
            project,
            key,
            container,
            passphrase: SecretString::from("a long private test passphrase".to_string()),
            options: Options {
                scrypt_log_n: 16,
                ..Default::default()
            },
        }
    }
    fn create(&self) -> vault::Receipt {
        vault::create(
            &self.project,
            &self.container,
            &self.passphrase,
            &self.key,
            &self.options,
            &Control::default(),
        )
        .unwrap()
    }
    fn open(&self) -> vault::WorkCopy {
        vault::unlock(
            &self.container,
            &Credentials::Passphrase(&self.passphrase),
            &self.root,
            None,
            &self.options,
            &Control::default(),
        )
        .unwrap()
    }
}
fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn tree(root: &Path) -> BTreeMap<PathBuf, String> {
    let mut pending = vec![root.to_owned()];
    let mut files = BTreeMap::new();
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    hash(&fs::read(path).unwrap()),
                );
            }
        }
    }
    files
}
fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn snapshot_encrypts_private_manifest_and_round_trips_exact_bytes_without_adopting_original() {
    let _serial = CRYPTO.lock().unwrap_or_else(|error| error.into_inner());
    let f = Fixture::new();
    let original = tree(&f.project);
    let created = f.create();
    assert_eq!(created.revision, 1);
    assert_eq!(created.sha256, hash(&fs::read(&f.container).unwrap()));
    let encrypted = fs::read(&f.container).unwrap();
    assert!(!contains(&encrypted, b"medical-secret"));
    assert!(!contains(&encrypted, b"PRIVATE patient"));
    assert!(!contains(
        &encrypted,
        f.project.to_string_lossy().as_bytes()
    ));
    let inventory = vault::verify(
        &f.container,
        &Credentials::RecoveryKey(&f.key),
        &Control::default(),
    )
    .unwrap();
    assert_eq!(inventory.vault_id, created.vault_id);
    assert!(
        inventory
            .files
            .iter()
            .any(|entry| entry.path.ends_with("medical-secret-1250.txt"))
    );
    let mut copy = f.open();
    assert!(copy.private_permissions());
    assert!(copy.project_path().starts_with(copy.work_root()));
    assert!(!copy.work_root().starts_with(&f.project));
    assert_eq!(tree(&copy.project_path()), original);
    Project::read(copy.project_path()).unwrap();
    let root = copy.work_root().to_owned();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&f.key).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let locked = copy
        .commit_and_lock(Duration::from_secs(1), &Control::default())
        .unwrap();
    assert!(locked.fully_locked, "{:?}", locked.retained);
    assert!(!root.exists());
    assert!(locked.receipt.previous.unwrap().is_file());
    assert_eq!(tree(&f.project), original);
}

#[test]
fn wrong_credentials_tamper_and_truncation_publish_no_plaintext_copy() {
    let _serial = CRYPTO.lock().unwrap_or_else(|error| error.into_inner());
    let f = Fixture::new();
    f.create();
    let encrypted = fs::read(&f.container).unwrap();
    let work_parent = f.root.join("work-parent");
    fs::create_dir(&work_parent).unwrap();
    let wrong = SecretString::from("this is a different passphrase".to_string());
    assert!(
        vault::unlock(
            &f.container,
            &Credentials::Passphrase(&wrong),
            &work_parent,
            None,
            &f.options,
            &Control::default()
        )
        .is_err()
    );
    assert!(fs::read_dir(&work_parent).unwrap().next().is_none());
    for (index, bytes) in [
        encrypted[..encrypted.len() - 1].to_vec(),
        encrypted[..encrypted.len() / 2].to_vec(),
        {
            let mut damaged = encrypted.clone();
            let last = damaged.len() - 5;
            damaged[last] ^= 1;
            damaged
        },
        b"NoteGen-Encrypted-Workspace-1\n\xff\xff\xff\xff".to_vec(),
    ]
    .into_iter()
    .enumerate()
    {
        let damaged = f.root.join(format!("damaged-{index}.ngvault"));
        fs::write(&damaged, bytes).unwrap();
        assert!(
            vault::unlock(
                &damaged,
                &Credentials::RecoveryKey(&f.key),
                &work_parent,
                Some(&f.passphrase),
                &f.options,
                &Control::default()
            )
            .is_err()
        );
        assert!(fs::read_dir(&work_parent).unwrap().next().is_none());
    }
    assert_eq!(fs::read(&f.container).unwrap(), encrypted);
}

#[test]
fn backup_restore_and_rotation_are_verified_no_clobber_and_retire_old_credentials() {
    let _serial = CRYPTO.lock().unwrap_or_else(|error| error.into_inner());
    let f = Fixture::new();
    let initial = f.create();
    let backup = f.root.join("backup.ngvault");
    let credentials = Credentials::RecoveryKey(&f.key);
    vault::backup(&f.container, &backup, &credentials, &Control::default()).unwrap();
    assert_eq!(fs::read(&backup).unwrap(), fs::read(&f.container).unwrap());
    assert!(vault::backup(&f.container, &backup, &credentials, &Control::default()).is_err());
    let restored = f.root.join("restored.ngvault");
    vault::restore(&backup, &restored, &credentials, &Control::default()).unwrap();
    assert!(vault::restore(&backup, &f.container, &credentials, &Control::default()).is_err());
    let next_key = f.root.join("replacement-recovery.agekey");
    vault::generate_recovery_key(&next_key, &Control::default()).unwrap();
    let next_pass = SecretString::from("new independent private passphrase".to_string());
    let cancelled = Control::default();
    cancelled.cancelled.store(true, Ordering::Relaxed);
    assert!(
        vault::rotate(
            &f.container,
            &credentials,
            &next_pass,
            Some(&next_key),
            &f.options,
            &cancelled
        )
        .is_err()
    );
    assert_eq!(hash(&fs::read(&f.container).unwrap()), initial.sha256);
    let rotated = vault::rotate(
        &f.container,
        &credentials,
        &next_pass,
        Some(&next_key),
        &f.options,
        &Control::default(),
    )
    .unwrap();
    assert_eq!(rotated.revision, 2);
    assert_eq!(rotated.vault_id, initial.vault_id);
    assert_ne!(rotated.sha256, initial.sha256);
    assert!(
        vault::verify(
            &f.container,
            &Credentials::Passphrase(&f.passphrase),
            &Control::default()
        )
        .is_err()
    );
    assert!(vault::verify(&f.container, &credentials, &Control::default()).is_err());
    let new = vault::verify(
        &f.container,
        &Credentials::Passphrase(&next_pass),
        &Control::default(),
    )
    .unwrap();
    assert_eq!(new.revision, 2);
    vault::verify(
        &f.container,
        &Credentials::RecoveryKey(&next_key),
        &Control::default(),
    )
    .unwrap();
    for old in [&backup, &restored, rotated.previous.as_ref().unwrap()] {
        assert_eq!(
            vault::verify(old, &credentials, &Control::default())
                .unwrap()
                .revision,
            1
        );
    }
}

#[test]
fn recovery_requires_new_passphrase_and_edited_commit_preserves_previous_and_original() {
    let _serial = CRYPTO.lock().unwrap_or_else(|error| error.into_inner());
    let f = Fixture::new();
    f.create();
    let original = tree(&f.project);
    assert!(
        vault::unlock(
            &f.container,
            &Credentials::RecoveryKey(&f.key),
            &f.root,
            None,
            &f.options,
            &Control::default()
        )
        .is_err()
    );
    let next_pass = SecretString::from("recovered independent passphrase".to_string());
    let mut copy = vault::unlock(
        &f.container,
        &Credentials::RecoveryKey(&f.key),
        &f.root,
        Some(&next_pass),
        &f.options,
        &Control::default(),
    )
    .unwrap();
    let edited = copy
        .project_path()
        .join("private subdir/medical-secret-1250.txt");
    fs::write(&edited, b"reviewed 1950\r\n\0new exact bytes").unwrap();
    let receipt = copy
        .commit_and_lock(Duration::from_secs(1), &Control::default())
        .unwrap();
    assert!(receipt.fully_locked, "{:?}", receipt.retained);
    assert_eq!(tree(&f.project), original);
    assert!(
        vault::verify(
            &f.container,
            &Credentials::Passphrase(&f.passphrase),
            &Control::default()
        )
        .is_err()
    );
    vault::verify(
        receipt.receipt.previous.as_ref().unwrap(),
        &Credentials::Passphrase(&f.passphrase),
        &Control::default(),
    )
    .unwrap();
    let mut reopened = vault::unlock(
        &f.container,
        &Credentials::Passphrase(&next_pass),
        &f.root,
        None,
        &f.options,
        &Control::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read(
            reopened
                .project_path()
                .join("private subdir/medical-secret-1250.txt")
        )
        .unwrap(),
        b"reviewed 1950\r\n\0new exact bytes"
    );
    assert!(
        reopened
            .commit_and_lock(Duration::from_secs(1), &Control::default())
            .unwrap()
            .fully_locked
    );
}

#[test]
fn busy_cancelled_or_modified_owner_copies_are_kept_and_unknown_plaintext_is_reported() {
    let _serial = CRYPTO.lock().unwrap_or_else(|error| error.into_inner());
    let f = Fixture::new();
    f.create();
    let cipher = fs::read(&f.container).unwrap();
    let mut copy = f.open();
    let guard = copy.begin_worker().unwrap();
    assert!(
        copy.commit_and_lock(Duration::ZERO, &Control::default())
            .is_err()
    );
    assert!(copy.project_path().join("project.json").is_file());
    assert_eq!(fs::read(&f.container).unwrap(), cipher);
    let cancelled = Control::default();
    cancelled.cancelled.store(true, Ordering::Relaxed);
    assert!(
        copy.commit_and_lock(Duration::from_secs(1), &cancelled)
            .is_err()
    );
    drop(guard);
    let unknown = copy.work_root().join("manual-file-outside-owned-data.txt");
    fs::write(&unknown, b"newer independent private note").unwrap();
    let root = copy.work_root().to_owned();
    let report = copy
        .commit_and_lock(Duration::from_secs(1), &Control::default())
        .unwrap();
    assert!(!report.fully_locked);
    assert!(report.retained.iter().any(|entry| entry.path == unknown));
    assert_eq!(
        fs::read(&unknown).unwrap(),
        b"newer independent private note"
    );
    let retained = vault::inspect_retained(&root, &Control::default()).unwrap();
    assert!(retained.note.contains("retained plaintext"));
    assert_eq!(retained.vault_id, report.receipt.vault_id);
    let mut next = f.open();
    let marker = next.work_root().join("notegen-vault-work.json");
    fs::write(&marker, b"changed ownership").unwrap();
    let current = fs::read(&f.container).unwrap();
    assert!(
        next.commit_and_lock(Duration::from_secs(1), &Control::default())
            .is_err()
    );
    assert_eq!(fs::read(&f.container).unwrap(), current);
    assert!(next.project_path().join("project.json").is_file());
    let abandoned = next.work_root().to_owned();
    drop(next);
    assert!(abandoned.exists());
    assert!(vault::inspect_retained(&abandoned, &Control::default()).is_err());
}

#[test]
fn bounded_inputs_and_external_destinations_fail_without_mutating_project_or_key() {
    let _serial = CRYPTO.lock().unwrap_or_else(|error| error.into_inner());
    let f = Fixture::new();
    let original = tree(&f.project);
    let key = fs::read(&f.key).unwrap();
    assert!(vault::generate_recovery_key(&f.key, &Control::default()).is_err());
    assert_eq!(fs::read(&f.key).unwrap(), key);
    let small = Options {
        max_total_bytes: 4,
        ..f.options.clone()
    };
    assert!(
        vault::create(
            &f.project,
            &f.container,
            &f.passphrase,
            &f.key,
            &small,
            &Control::default()
        )
        .is_err()
    );
    assert!(!f.container.exists());
    assert!(
        vault::create(
            &f.project,
            &f.project.join("nested/new.ngvault"),
            &f.passphrase,
            &f.key,
            &f.options,
            &Control::default()
        )
        .is_err()
    );
    assert!(!f.project.join("nested").exists());
    fs::write(f.project.join("copied-key.txt"), &key).unwrap();
    assert!(
        vault::create(
            &f.project,
            &f.container,
            &f.passphrase,
            &f.key,
            &f.options,
            &Control::default()
        )
        .is_err()
    );
    fs::remove_file(f.project.join("copied-key.txt")).unwrap();
    f.create();
    assert!(
        vault::backup(
            &f.container,
            &f.project.join("backup.ngvault"),
            &Credentials::RecoveryKey(&f.key),
            &Control::default()
        )
        .is_err()
    );
    assert!(
        vault::unlock(
            &f.container,
            &Credentials::RecoveryKey(&f.key),
            &f.project,
            Some(&f.passphrase),
            &f.options,
            &Control::default()
        )
        .is_err()
    );
    assert_eq!(tree(&f.project), original);
    assert_eq!(fs::read(&f.key).unwrap(), key);
}

#[cfg(windows)]
#[test]
fn failed_atomic_activation_keeps_current_and_a_verified_previous_container() {
    use std::os::windows::fs::OpenOptionsExt;
    let _serial = CRYPTO.lock().unwrap_or_else(|error| error.into_inner());
    let f = Fixture::new();
    let initial = f.create();
    let original = fs::read(&f.container).unwrap();
    // Permit reading/copying, but hold Windows' real delete/rename exclusion.
    let handle = fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(&f.container)
        .unwrap();
    let next = SecretString::from("replacement independent passphrase".to_string());
    assert!(
        vault::rotate(
            &f.container,
            &Credentials::RecoveryKey(&f.key),
            &next,
            None,
            &f.options,
            &Control::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(&f.container).unwrap(), original);
    drop(handle);
    let previous = f
        .root
        .join(format!(".vault-backup-{}.ngvault", initial.sha256));
    assert_eq!(fs::read(&previous).unwrap(), original);
    assert_eq!(
        vault::verify(
            &previous,
            &Credentials::RecoveryKey(&f.key),
            &Control::default()
        )
        .unwrap()
        .revision,
        1
    );
    vault::verify(
        &f.container,
        &Credentials::Passphrase(&f.passphrase),
        &Control::default(),
    )
    .unwrap();
    assert!(
        vault::verify(
            &f.container,
            &Credentials::Passphrase(&next),
            &Control::default()
        )
        .is_err()
    );
}
