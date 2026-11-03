//! Registered, bounded migrations with durable before-images and conflict-safe rollback.
use crate::{
    storage::{Control, Project, fingerprint, read_json},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
const LIMIT: u64 = 536_870_912;
const JOURNAL: &str = "project-transaction.json";
fn fail(message: impl Into<String>) -> anyhow::Error {
    Fault::new("NG500", message).into()
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIdentity {
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangeSummary {
    pub file: String,
    pub before: Option<FileIdentity>,
    pub after: Option<FileIdentity>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
enum Kind {
    Project,
    Configuration(String),
}
#[derive(Clone, Serialize)]
pub struct Plan {
    pub root: PathBuf,
    pub label: String,
    pub token: String,
    pub files: Vec<ChangeSummary>,
    pub note: String,
    #[serde(skip)]
    kind: Kind,
    #[serde(skip)]
    before: Vec<Option<Vec<u8>>>,
    #[serde(skip)]
    after: Vec<Option<Vec<u8>>>,
    #[serde(skip)]
    dependencies: Vec<(usize, String, crate::redactions::SourceIdentity)>,
    #[serde(skip)]
    guard_dependencies: Vec<(String, Option<FileIdentity>)>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub root: PathBuf,
    pub id: String,
    pub label: String,
    pub backup: PathBuf,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u32,
    kind: Kind,
    id: String,
    label: String,
    files: Vec<ChangeSummary>,
    guards: Vec<(String, Option<FileIdentity>)>,
}
fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn allowed(file: &str, kind: &Kind) -> bool {
    if file.contains(['/', '\\']) || matches!(file, "." | "..") {
        return false;
    }
    match kind {
        Kind::Configuration(name) => file == name && file.ends_with(".json"),
        Kind::Project => {
            matches!(file, "project.json" | "source-catalog.json") || {
                let Some(rest) = file.strip_prefix("page-") else {
                    return false;
                };
                let Some(number) = rest.get(..5) else {
                    return false;
                };
                number.bytes().all(|b| b.is_ascii_digit())
                    && number
                        .parse::<usize>()
                        .is_ok_and(|n| (1..=3000).contains(&n))
                    && matches!(&rest[5..], ".png" | ".json" | ".edits.json")
            }
        }
    }
}
fn bytes(path: &Path, c: &Control) -> Result<Option<Vec<u8>>> {
    crate::watch::no_links(path)?;
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    anyhow::ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= LIMIT,
        fail("Migration file exceeds its byte budget")
    );
    let mut output = vec![];
    let mut chunk = [0u8; 65536];
    loop {
        c.check()?;
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        anyhow::ensure!(
            output.len() as u64 + n as u64 <= LIMIT,
            fail("Migration source grew beyond its byte budget")
        );
        output.extend_from_slice(&chunk[..n]);
    }
    Ok(Some(output))
}
fn identity(value: &[u8]) -> FileIdentity {
    use sha2::Digest;
    FileIdentity {
        bytes: value.len() as u64,
        sha256: hex::encode(sha2::Sha256::digest(value)),
    }
}
pub(crate) fn snapshot(path: &Path, c: &Control) -> Result<Option<Vec<u8>>> {
    bytes(path, c)
}
pub(crate) fn snapshot_identity(value: &[u8]) -> FileIdentity {
    identity(value)
}
fn current(path: &Path, c: &Control) -> Result<Option<FileIdentity>> {
    use sha2::Digest;
    crate::watch::no_links(path)?;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let meta = file.metadata()?;
    anyhow::ensure!(
        meta.is_file() && meta.len() <= LIMIT,
        fail("Migration target is not a bounded regular file")
    );
    let mut file = file.take(meta.len() + 1);
    let mut hash = sha2::Sha256::new();
    let mut total = 0;
    let mut chunk = [0u8; 65536];
    loop {
        c.check()?;
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        anyhow::ensure!(
            total <= meta.len(),
            fail("Migration file grew during hashing")
        );
        hash.update(&chunk[..n]);
    }
    anyhow::ensure!(
        total == meta.len(),
        fail("Migration file changed during hashing")
    );
    Ok(Some(FileIdentity {
        bytes: total,
        sha256: hex::encode(hash.finalize()),
    }))
}
pub fn ensure_readable(root: &Path) -> Result<()> {
    crate::watch::no_links(&root.join(JOURNAL))?;
    anyhow::ensure!(
        !root.join(JOURNAL).exists(),
        fail("Project migration is pending; reopen for recovery before reading or editing")
    );
    Ok(())
}
pub(crate) fn plan(
    root: &Path,
    label: &str,
    changes: Vec<(String, Option<Vec<u8>>)>,
    c: &Control,
) -> Result<Plan> {
    plan_kind(root, label, changes, Kind::Project, c)
}
fn plan_kind(
    root: &Path,
    label: &str,
    changes: Vec<(String, Option<Vec<u8>>)>,
    kind: Kind,
    c: &Control,
) -> Result<Plan> {
    let root = root.canonicalize()?;
    crate::watch::no_links(&root)?;
    ensure_readable(&root)?;
    anyhow::ensure!(
        !changes.is_empty()
            && changes.len() <= 100
            && !label.trim().is_empty()
            && label.len() <= 200,
        fail("Choose a bounded registered migration")
    );
    let mut files = vec![];
    let mut before = vec![];
    let mut after = vec![];
    let mut seen = std::collections::BTreeSet::new();
    let mut total = 0u64;
    for (file, value) in changes {
        c.check()?;
        anyhow::ensure!(
            allowed(&file, &kind) && seen.insert(file.clone()),
            fail("Unsupported or duplicate migration target")
        );
        let old = bytes(&root.join(&file), c)?;
        total = total
            .saturating_add(old.as_ref().map_or(0, |v| v.len() as u64))
            .saturating_add(value.as_ref().map_or(0, |v| v.len() as u64));
        anyhow::ensure!(
            total <= LIMIT,
            fail("Migration exceeds the 512 MiB transaction budget")
        );
        files.push(ChangeSummary {
            file,
            before: old.as_deref().map(identity),
            after: value.as_deref().map(identity),
        });
        before.push(old);
        after.push(value);
    }
    let dependencies: Vec<(usize, String, crate::redactions::SourceIdentity)> = vec![];
    let guard_dependencies: Vec<(String, Option<FileIdentity>)> = vec![];
    let token = fingerprint(&(
        1,
        &root,
        label,
        &kind,
        &files,
        &dependencies,
        &guard_dependencies,
    ))?;
    Ok(Plan{root,label:label.into(),token,files,kind,before,after,dependencies,guard_dependencies,note:"Explicit supported migration only. Exact before-images and new bytes are retained in a private recovery directory. Interruptions restore the previous state; conflicting outside edits are preserved and require inspection. Unknown schemas are never guessed.".into()})
}
pub(crate) fn bind_pages(
    plan: &mut Plan,
    pages: Vec<(usize, String, crate::redactions::SourceIdentity)>,
) -> Result<()> {
    anyhow::ensure!(
        plan.kind == Kind::Project && pages.len() <= 3000,
        fail("Invalid migration source dependencies")
    );
    plan.dependencies = pages;
    plan.token = fingerprint(&(
        1,
        &plan.root,
        plan.label.as_str(),
        &plan.kind,
        &plan.files,
        &plan.dependencies,
        &plan.guard_dependencies,
    ))?;
    Ok(())
}
pub(crate) fn bind_guard(
    plan: &mut Plan,
    name: &str,
    expected: Option<FileIdentity>,
) -> Result<()> {
    anyhow::ensure!(
        name == "model-lock.json" && plan.kind == Kind::Project,
        fail("Unsupported migration guard")
    );
    plan.guard_dependencies = vec![(name.into(), expected)];
    plan.token = fingerprint(&(
        1,
        &plan.root,
        plan.label.as_str(),
        &plan.kind,
        &plan.files,
        &plan.dependencies,
        &plan.guard_dependencies,
    ))?;
    Ok(())
}
fn check_dependencies(plan: &Plan, c: &Control) -> Result<()> {
    for (name, expected) in &plan.guard_dependencies {
        anyhow::ensure!(
            current(&plan.root.join(name), c)? == *expected,
            fail("Selected exact model lock changed after preview")
        );
    }
    if !plan.dependencies.is_empty() {
        let project = Project {
            root: plan.root.clone(),
            manifest: read_json(&plan.root.join("project.json"), 1_000_000)?,
        };
        for (n, digest, source) in &plan.dependencies {
            let edits = project.root.join(format!("page-{n:05}.edits.json"));
            let base = project.page_path(*n);
            crate::watch::no_links(&edits)?;
            crate::watch::no_links(&base)?;
            let path = if edits.is_file() { edits } else { base };
            let page: Page = read_json(&path, 32_000_000)?;
            anyhow::ensure!(
                fingerprint(&page)? == *digest
                    && crate::redactions::source_identity(&project, &page, c)? == *source,
                fail("Migration source or confirmed page changed after preview")
            );
        }
    }
    Ok(())
}
fn stage_write(path: &Path, value: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(value)?;
    file.sync_all()?;
    Ok(())
}
fn create_journal(root: &Path, j: &Journal) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer_pretty(&mut file, j)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(root.join(JOURNAL))
        .map_err(|error| error.error)?;
    Ok(())
}
fn finish_journal(root: &Path, j: &Journal) -> Result<()> {
    remove_captured(
        root,
        &root.join(JOURNAL),
        &identity(&serde_json::to_vec_pretty(j)?),
    )
}
fn check_journal(j: &Journal) -> Result<()> {
    anyhow::ensure!(
        j.schema == 1 && valid_id(&j.id) && !j.files.is_empty() && j.files.len() <= 100,
        fail("Invalid migration journal")
    );
    let mut seen = std::collections::BTreeSet::new();
    anyhow::ensure!(
        j.guards.len() <= 5
            && j.guards.iter().all(|(name, _)| [
                "review-history.json",
                "review-pending.json",
                "model-lock.json",
                "guidance.json",
                "project-metadata.json"
            ]
            .contains(&name.as_str())),
        fail("Invalid migration revision guards")
    );
    for f in &j.files {
        anyhow::ensure!(
            allowed(&f.file, &j.kind) && seen.insert(&f.file),
            fail("Unsafe migration journal target")
        );
        for id in [&f.before, &f.after].into_iter().flatten() {
            anyhow::ensure!(
                id.bytes <= LIMIT && valid_id(&id.sha256),
                fail("Invalid migration file identity")
            );
        }
    }
    Ok(())
}
fn directory(root: &Path, id: &str) -> PathBuf {
    root.join(".notegen-transactions").join(id)
}
fn lock(root: &Path) -> Result<(fs::File, fs::File)> {
    crate::watch::no_links(&root.join("migration.lock"))?;
    crate::watch::no_links(&root.join("review.lock"))?;
    let mut files = vec![];
    for name in ["migration.lock", "review.lock"] {
        let f = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(name))?;
        f.try_lock().map_err(|_| {
            fail("Another project edit or migration is active; retry after it finishes")
        })?;
        files.push(f);
    }
    Ok((files.remove(0), files.remove(0)))
}
fn write_copy(
    root: &Path,
    target: &Path,
    source: &Path,
    expected: &Option<FileIdentity>,
    wanted: &FileIdentity,
    c: &Control,
) -> Result<()> {
    anyhow::ensure!(
        current(source, c)?.as_ref() == Some(wanted),
        fail("Migration recovery bytes changed")
    );
    let mut temp = tempfile::NamedTempFile::new_in(root)?;
    let mut input = fs::File::open(source)?.take(wanted.bytes + 1);
    let mut total = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        c.check()?;
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        anyhow::ensure!(total <= wanted.bytes, fail("Migration staging file grew"));
        temp.write_all(&buffer[..n])?;
    }
    temp.as_file().sync_all()?;
    anyhow::ensure!(
        current(temp.path(), c)?.as_ref() == Some(wanted),
        fail("Migration staging content changed")
    );
    let expected = crate::diagnostics::RepairFile {
        key: "project-migration".into(),
        expected_bytes: wanted.bytes,
        expected_sha256: wanted.sha256.clone(),
        current_bytes: expected.as_ref().map(|v| v.bytes),
        current_sha256: expected.as_ref().map(|v| v.sha256.clone()),
        health: if expected.is_some() {
            crate::diagnostics::AssetHealth::Corrupt
        } else {
            crate::diagnostics::AssetHealth::Missing
        },
    };
    crate::diagnostics::publish_repair(temp.into_temp_path(), target, &expected, c)?;
    Ok(())
}
fn remove_captured(root: &Path, target: &Path, expected: &FileIdentity) -> Result<()> {
    let retained = tempfile::Builder::new()
        .prefix(".notegen-migration-retained-")
        .tempdir_in(root)?
        .keep();
    let captured = retained.join("captured");
    fs::rename(target, &captured)?;
    if current(&captured, &Control::default())?.as_ref() != Some(expected) {
        let mut restore = tempfile::TempPath::try_from_path(&captured)?;
        restore.disable_cleanup(true);
        let _ = restore.persist_noclobber(target);
        return Err(fail(format!(
            "Concurrent content was preserved; inspect {}",
            retained.display()
        )));
    }
    Ok(())
}
fn restore(root: &Path, j: &Journal, c: &Control) -> Result<()> {
    check_journal(j)?;
    for (name, expected) in &j.guards {
        anyhow::ensure!(
            current(&root.join(name), c)? == *expected,
            fail(
                "Project history/context/lock changed during migration; all versions retained for inspection"
            )
        );
    }
    let dir = directory(root, &j.id);
    crate::watch::no_links(&dir)?;
    for (i, f) in j.files.iter().enumerate().rev() {
        let target = root.join(&f.file);
        let now = current(&target, c)?;
        if now == f.before {
            continue;
        }
        anyhow::ensure!(
            now == f.after || now.is_none() && f.before.is_some(),
            fail(format!(
                "Migration recovery conflict at {}; all versions retained at {}",
                target.display(),
                dir.display()
            ))
        );
        if let Some(before) = &f.before {
            write_copy(
                root,
                &target,
                &dir.join(format!("before-{i}")),
                &now,
                before,
                c,
            )?;
        } else if let Some(now) = now {
            remove_captured(root, &target, &now)?;
        }
    }
    finish_journal(root, j)?;
    Ok(())
}
pub fn recover(root: &Path, c: &Control) -> Result<()> {
    recover_kind(root, &Kind::Project, c)
}
fn recover_kind(root: &Path, kind: &Kind, c: &Control) -> Result<()> {
    crate::watch::no_links(&root.join(JOURNAL))?;
    if !root.join(JOURNAL).exists() {
        return Ok(());
    }
    let _guard = lock(root)?;
    let j: Journal = read_json(&root.join(JOURNAL), 1_000_000)?;
    anyhow::ensure!(
        &j.kind == kind,
        fail("Migration journal belongs to another target kind")
    );
    restore(root, &j, c)
}
pub fn apply(plan: &Plan, token: &str, c: &Control) -> Result<Receipt> {
    anyhow::ensure!(
        token == plan.token
            && token
                == fingerprint(&(
                    1,
                    &plan.root,
                    plan.label.as_str(),
                    &plan.kind,
                    &plan.files,
                    &plan.dependencies,
                    &plan.guard_dependencies
                ))?,
        fail("Confirm the exact fresh migration preview")
    );
    let _guard = lock(&plan.root)?;
    ensure_readable(&plan.root)?;
    check_dependencies(plan, c)?;
    for f in &plan.files {
        anyhow::ensure!(
            current(&plan.root.join(&f.file), c)? == f.before,
            fail("Migration source changed after preview")
        );
    }
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(|_| fail("Cannot allocate a migration identity"))?;
    let id = fingerprint(&(&plan.token, nonce))?;
    let dir = directory(&plan.root, &id);
    crate::watch::no_links(&dir)?;
    fs::create_dir_all(dir.parent().unwrap())?;
    fs::create_dir(&dir)?;
    for (i, (before, after)) in plan.before.iter().zip(&plan.after).enumerate() {
        c.check()?;
        if let Some(v) = before {
            stage_write(&dir.join(format!("before-{i}")), v)?;
        }
        if let Some(v) = after {
            stage_write(&dir.join(format!("after-{i}")), v)?;
        }
    }
    let j = Journal {
        schema: 1,
        kind: plan.kind.clone(),
        id: id.clone(),
        label: plan.label.clone(),
        files: plan.files.clone(),
        guards: if plan.kind == Kind::Project {
            [
                "review-history.json",
                "review-pending.json",
                "model-lock.json",
                "guidance.json",
                "project-metadata.json",
            ]
            .into_iter()
            .map(|name| Ok((name.into(), current(&plan.root.join(name), c)?)))
            .collect::<Result<Vec<_>>>()?
        } else {
            vec![]
        },
    };
    check_journal(&j)?;
    stage_write(&dir.join("receipt.json"), &serde_json::to_vec_pretty(&j)?)?;
    create_journal(&plan.root, &j)?;
    let result: Result<()> = (|| {
        for (i, f) in j.files.iter().enumerate() {
            c.check()?;
            anyhow::ensure!(
                current(&plan.root.join(&f.file), c)? == f.before,
                fail("Migration target changed before commit")
            );
            if let Some(wanted) = &f.after {
                write_copy(
                    &plan.root,
                    &plan.root.join(&f.file),
                    &dir.join(format!("after-{i}")),
                    &f.before,
                    wanted,
                    c,
                )?;
            } else if let Some(before) = &f.before {
                remove_captured(&plan.root, &plan.root.join(&f.file), before)?;
            }
        }
        check_dependencies(plan, c)?;
        for f in &j.files {
            anyhow::ensure!(
                current(&plan.root.join(&f.file), c)? == f.after,
                fail("Migration target changed before publication")
            );
        }
        Ok(())
    })();
    if let Err(error) = result {
        return match restore(&plan.root, &j, &Control::default()) {
            Ok(()) => Err(error),
            Err(recovery) => Err(error.context(format!(
                "Recovery retained at {}: {recovery}",
                dir.display()
            ))),
        };
    }
    finish_journal(&plan.root, &j)?;
    Ok(Receipt {
        root: plan.root.clone(),
        id,
        label: plan.label.clone(),
        backup: dir,
    })
}
pub fn rollback_preview(receipt: &Receipt, c: &Control) -> Result<Plan> {
    anyhow::ensure!(
        valid_id(&receipt.id) && receipt.backup == directory(&receipt.root, &receipt.id),
        fail("Invalid migration receipt path")
    );
    crate::watch::no_links(&receipt.backup)?;
    let j: Journal = read_json(&receipt.backup.join("receipt.json"), 1_000_000)?;
    check_journal(&j)?;
    anyhow::ensure!(
        !j.files.iter().any(|file| file.file.starts_with("page-")
            && file.before.is_none()
            && file.after.is_some()),
        fail(
            "This page transaction allocated stable source slots; use layout undo/redo so archived source references remain reserved"
        )
    );
    for (name, expected) in &j.guards {
        anyhow::ensure!(
            current(&receipt.root.join(name), c)? == *expected,
            fail(
                "Project has later history/context/lock edits; use layout undo or preserve a separate revision"
            )
        );
    }
    anyhow::ensure!(
        j.id == receipt.id,
        fail("Migration receipt identity changed")
    );
    let mut changes = vec![];
    for (i, f) in j.files.iter().enumerate() {
        anyhow::ensure!(
            current(&receipt.root.join(&f.file), c)? == f.after,
            fail("Migration has later edits; rollback would overwrite them")
        );
        let before = bytes(&receipt.backup.join(format!("before-{i}")), c)?;
        anyhow::ensure!(
            before.as_deref().map(identity) == f.before,
            fail("Migration backup changed")
        );
        changes.push((f.file.clone(), before));
    }
    let mut plan = plan_kind(
        &receipt.root,
        "Rollback supported migration",
        changes,
        j.kind,
        c,
    )?;
    anyhow::ensure!(
        plan.files
            .iter()
            .zip(&j.files)
            .all(|(planned, old)| planned.before == old.after),
        fail("Project changed while preparing rollback")
    );
    for (name, expected) in &j.guards {
        anyhow::ensure!(
            current(&receipt.root.join(name), c)? == *expected,
            fail("Project history changed while preparing rollback")
        );
    }
    plan.guard_dependencies = j.guards.clone();
    plan.token = fingerprint(&(
        1,
        &plan.root,
        plan.label.as_str(),
        &plan.kind,
        &plan.files,
        &plan.dependencies,
        &plan.guard_dependencies,
    ))?;
    Ok(plan)
}
pub fn read_receipt(path: &Path, c: &Control) -> Result<Receipt> {
    c.check()?;
    crate::watch::no_links(path)?;
    let path = path.canonicalize()?;
    let file = if path.is_dir() {
        path.join("receipt.json")
    } else {
        path
    };
    anyhow::ensure!(
        file.file_name().is_some_and(|v| v == "receipt.json"),
        fail("Choose a migration receipt.json")
    );
    let backup = file
        .parent()
        .ok_or_else(|| fail("Invalid receipt folder"))?;
    let id = backup
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| fail("Invalid receipt identity"))?;
    let transactions = backup
        .parent()
        .ok_or_else(|| fail("Invalid transaction folder"))?;
    anyhow::ensure!(
        transactions
            .file_name()
            .is_some_and(|v| v == ".notegen-transactions")
            && valid_id(id),
        fail("Receipt is outside a migration recovery directory")
    );
    let root = transactions
        .parent()
        .ok_or_else(|| fail("Invalid migration root"))?;
    let j: Journal = read_json(&file, 1_000_000)?;
    check_journal(&j)?;
    anyhow::ensure!(j.id == id, fail("Receipt identity changed"));
    Ok(Receipt {
        root: root.into(),
        id: id.into(),
        label: j.label,
        backup: backup.into(),
    })
}
pub fn preview_settings(path: &Path, c: &Control) -> Result<Plan> {
    let path = std::path::absolute(path)?;
    let file = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| fail("Choose a JSON settings filename"))?;
    let old = bytes(&path, c)?.ok_or_else(|| fail("Settings file is missing"))?;
    let settings: Settings = serde_json::from_slice(&old)?;
    settings.validate()?;
    let plan = plan_kind(
        path.parent().unwrap(),
        "Expand supported settings defaults",
        vec![(file.into(), Some(serde_json::to_vec_pretty(&settings)?))],
        Kind::Configuration(file.into()),
        c,
    )?;
    anyhow::ensure!(
        plan.files[0].before.as_ref() == Some(&identity(&old)),
        fail("Settings changed during migration preview")
    );
    Ok(plan)
}
pub fn recover_settings(path: &Path, c: &Control) -> Result<()> {
    let path = std::path::absolute(path)?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| fail("Choose a settings file"))?;
    recover_kind(path.parent().unwrap(), &Kind::Configuration(name.into()), c)
}
pub fn preview_project_sources(project: &Project, c: &Control) -> Result<Plan> {
    crate::project_layout::migration_plan(project, c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::atomic_json;
    fn interrupted(root: &Path) -> Journal {
        fs::write(root.join("project.json"), b"original manifest").unwrap();
        let plan = plan(
            root,
            "Interrupted transaction",
            vec![
                ("project.json".into(), Some(b"new manifest".to_vec())),
                (
                    "page-00003.json".into(),
                    Some(b"new immutable page".to_vec()),
                ),
            ],
            &Control::default(),
        )
        .unwrap();
        let j = Journal {
            schema: 1,
            kind: Kind::Project,
            id: "a".repeat(64),
            label: plan.label.clone(),
            files: plan.files.clone(),
            guards: vec![],
        };
        let dir = directory(root, &j.id);
        fs::create_dir_all(&dir).unwrap();
        for (i, (before, after)) in plan.before.iter().zip(&plan.after).enumerate() {
            if let Some(bytes) = before {
                stage_write(&dir.join(format!("before-{i}")), bytes).unwrap();
            }
            if let Some(bytes) = after {
                stage_write(&dir.join(format!("after-{i}")), bytes).unwrap();
            }
        }
        atomic_json(&root.join(JOURNAL), &j).unwrap();
        fs::write(root.join("project.json"), b"new manifest").unwrap();
        fs::write(root.join("page-00003.json"), b"new immutable page").unwrap();
        j
    }
    #[test]
    fn durable_interruption_recovery_restores_before_state_and_retains_created_bytes() {
        let root = tempfile::tempdir().unwrap();
        let j = interrupted(root.path());
        assert!(ensure_readable(root.path()).is_err());
        recover(root.path(), &Control::default()).unwrap();
        assert_eq!(
            fs::read(root.path().join("project.json")).unwrap(),
            b"original manifest"
        );
        assert!(!root.path().join("page-00003.json").exists());
        assert!(!root.path().join(JOURNAL).exists());
        assert_eq!(
            fs::read(directory(root.path(), &j.id).join("after-1")).unwrap(),
            b"new immutable page"
        );
        assert!(fs::read_dir(root.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".notegen-migration-retained-")
        }));
    }
    #[test]
    fn recovery_conflict_preserves_concurrent_content_and_backup_for_inspection() {
        let root = tempfile::tempdir().unwrap();
        let j = interrupted(root.path());
        fs::write(
            root.path().join("project.json"),
            b"concurrent user replacement",
        )
        .unwrap();
        assert!(recover(root.path(), &Control::default()).is_err());
        assert_eq!(
            fs::read(root.path().join("project.json")).unwrap(),
            b"concurrent user replacement"
        );
        assert!(root.path().join(JOURNAL).is_file());
        assert_eq!(
            fs::read(directory(root.path(), &j.id).join("before-0")).unwrap(),
            b"original manifest"
        );
    }
    #[test]
    fn untrusted_journal_paths_are_rejected_before_touching_outside_data() {
        let outer = tempfile::tempdir().unwrap();
        let root = outer.path().join("project");
        fs::create_dir(&root).unwrap();
        let outside = outer.path().join("outside.json");
        fs::write(&outside, b"untouched").unwrap();
        let j = Journal {
            schema: 1,
            kind: Kind::Project,
            id: "b".repeat(64),
            label: "untrusted".into(),
            files: vec![ChangeSummary {
                file: "../outside.json".into(),
                before: Some(identity(b"untouched")),
                after: None,
            }],
            guards: vec![],
        };
        atomic_json(&root.join(JOURNAL), &j).unwrap();
        assert!(recover(&root, &Control::default()).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"untouched");
        assert!(root.join(JOURNAL).exists());
    }
}
