//! Durable learned-example history. Inactive sample images are retained for undo.
use crate::{
    storage::{Control, atomic_json, fingerprint, read_json},
    types::*,
    watch::no_links,
    writers::{self, Profile},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Read,
    path::Path,
};

const MAX_STATES: usize = 65;
const MAX_HISTORY: u64 = 128_000_000;
pub const RETENTION_NOTE: &str = "The latest 64 learned-example edits support undo and redo across restarts. New edits after undo replace the redo branch. Inactive sample images are retained locally for recovery. Forgotten and quarantined examples are excluded from active recognition and font exports. Delete the entire profile explicitly to remove its local history; exported copies remain where you saved them.";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    profile: Profile,
    images: BTreeMap<String, String>,
    label: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct History {
    schema: u32,
    writer: String,
    cursor: usize,
    states: Vec<State>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    schema: u32,
    before_history: Option<String>,
    before: Profile,
    after: Profile,
    history: History,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Timeline {
    pub cursor: usize,
    pub labels: Vec<String>,
    pub note: String,
}
fn fail(message: &str) -> anyhow::Error {
    Fault::new("NG610", message).into()
}
fn same_content(left: &Profile, right: &Profile) -> Result<bool> {
    let mut left = left.clone();
    let mut right = right.clone();
    left.revision = 0;
    right.revision = 0;
    Ok(fingerprint(&left)? == fingerprint(&right)?)
}
fn raw(root: &Path, id: &str) -> Result<Profile> {
    let path = writers::folder(root, id)?.join("profile.json");
    no_links(&path)?;
    let profile: Profile = read_json(&path, 8_000_000)?;
    writers::validate(&profile)?;
    anyhow::ensure!(profile.id == id, fail("Writer identity changed"));
    Ok(profile)
}
fn image_hash(root: &Path, name: &str, c: &Control) -> Result<String> {
    let path = root.join(name);
    no_links(&path)?;
    let meta = path.metadata()?;
    anyhow::ensure!(
        meta.is_file() && meta.len() <= 1_000_000,
        fail("Writer history image is missing or oversized")
    );
    c.check()?;
    let mut bytes = vec![];
    File::open(&path)?.take(1_000_001).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 1_000_000 && bytes.len() as u64 == meta.len(),
        fail("Writer image changed while reading history")
    );
    Ok(hex::encode(Sha256::digest(bytes)))
}
fn state(dir: &Path, profile: &Profile, label: &str) -> Result<State> {
    writers::validate(profile)?;
    let mut images = BTreeMap::new();
    let c = Control::default();
    let mut total = 0u64;
    for sample in &profile.samples {
        total = total.saturating_add(dir.join(&sample.image).metadata()?.len());
        anyhow::ensure!(
            total <= 100_000_000,
            fail("Writer samples exceed the 100 MB history budget")
        );
        images.insert(sample.image.clone(), image_hash(dir, &sample.image, &c)?);
    }
    Ok(State {
        profile: profile.clone(),
        images,
        label: label.into(),
    })
}
fn validate(history: &History, id: &str) -> Result<()> {
    anyhow::ensure!(
        history.schema == 1
            && history.writer == id
            && !history.states.is_empty()
            && history.states.len() <= MAX_STATES
            && history.cursor < history.states.len(),
        fail("Invalid writer history")
    );
    for entry in &history.states {
        writers::validate(&entry.profile)?;
        anyhow::ensure!(
            entry.profile.id == id
                && !entry.label.is_empty()
                && entry.label.len() <= 200
                && entry.images.len() == entry.profile.samples.len()
                && entry
                    .profile
                    .samples
                    .iter()
                    .all(|s| entry.images.get(&s.image).is_some_and(|h| h.len() == 64
                        && h.bytes()
                            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))),
            fail("Invalid writer history state")
        );
    }
    Ok(())
}
fn history(dir: &Path, id: &str) -> Result<Option<History>> {
    let path = dir.join("writer-history.json");
    no_links(&path)?;
    if !path.exists() {
        return Ok(None);
    }
    let h: History = read_json(&path, MAX_HISTORY)?;
    validate(&h, id)?;
    Ok(Some(h))
}
fn verify_images(dir: &Path, entry: &State) -> Result<()> {
    let c = Control::default();
    let mut total = 0u64;
    for (name, expected) in &entry.images {
        total = total.saturating_add(dir.join(name).metadata()?.len());
        anyhow::ensure!(
            total <= 100_000_000,
            fail("Writer samples exceed the 100 MB history budget")
        );
        anyhow::ensure!(
            image_hash(dir, name, &c)? == *expected,
            fail("A retained sample image changed; preserve it before recovering this history")
        );
    }
    Ok(())
}
pub(crate) fn recover_locked(root: &Path, id: &str) -> Result<()> {
    let dir = writers::folder(root, id)?;
    let pending_path = dir.join("writer-pending.json");
    no_links(&pending_path)?;
    if !pending_path.exists() {
        return Ok(());
    }
    let pending: Pending = read_json(&pending_path, MAX_HISTORY + 16_000_000)?;
    validate(&pending.history, id)?;
    writers::validate(&pending.before)?;
    writers::validate(&pending.after)?;
    anyhow::ensure!(
        pending.schema == 1
            && pending.before.id == id
            && pending.after.id == id
            && same_content(
                &pending.after,
                &pending.history.states[pending.history.cursor].profile
            )?,
        fail("Invalid pending writer transaction")
    );
    let current = raw(root, id)?;
    anyhow::ensure!(
        fingerprint(&current)? == fingerprint(&pending.before)?
            || fingerprint(&current)? == fingerprint(&pending.after)?,
        fail("Writer changed outside its pending transaction; recovery was retained")
    );
    let current_history = history(&dir, id)?;
    let current_hash = current_history.as_ref().map(fingerprint).transpose()?;
    anyhow::ensure!(
        current_hash == pending.before_history
            || current_hash == Some(fingerprint(&pending.history)?),
        fail("Writer history changed during recovery; pending transaction was retained")
    );
    verify_images(&dir, &pending.history.states[pending.history.cursor])?;
    atomic_json(&dir.join("profile.json"), &pending.after)?;
    atomic_json(&dir.join("writer-history.json"), &pending.history)?;
    fs::remove_file(pending_path)?;
    Ok(())
}
pub(crate) fn lock(root: &Path, id: &str) -> Result<File> {
    let dir = writers::folder(root, id)?;
    no_links(&dir)?;
    let path = dir.join(".lock");
    no_links(&path)?;
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock()
        .map_err(|_| fail("Writer profile is busy; wait for its current operation"))?;
    recover_locked(root, id)?;
    Ok(file)
}
fn publish(
    root: &Path,
    id: &str,
    before: &Profile,
    after: &Profile,
    old: Option<&History>,
    next: History,
) -> Result<()> {
    let dir = writers::folder(root, id)?;
    validate(&next, id)?;
    anyhow::ensure!(
        fingerprint(&raw(root, id)?)? == fingerprint(before)?,
        fail("Writer profile changed; reload before applying this edit")
    );
    let pending = Pending {
        schema: 1,
        before_history: old.map(fingerprint).transpose()?,
        before: before.clone(),
        after: after.clone(),
        history: next,
    };
    anyhow::ensure!(
        serde_json::to_vec(&pending)?.len() as u64 <= MAX_HISTORY,
        fail(
            "Writer history is full; create a named snapshot and branch before making further changes"
        )
    );
    atomic_json(&dir.join("writer-pending.json"), &pending)?;
    recover_locked(root, id)
}
/// Caller owns the profile lock; both images and the original profile remain intact on failure.
pub(crate) fn commit_locked(
    root: &Path,
    before: &Profile,
    after: &Profile,
    label: &str,
) -> Result<()> {
    anyhow::ensure!(
        !label.trim().is_empty()
            && label.len() <= 200
            && before.id == after.id
            && after.revision > before.revision,
        fail("Invalid writer edit transaction")
    );
    let dir = writers::folder(root, &before.id)?;
    let old = history(&dir, &before.id)?;
    let mut next = if let Some(old) = &old {
        anyhow::ensure!(
            same_content(before, &old.states[old.cursor].profile)?,
            fail("Writer changed outside its history; preserve a copy before starting new history")
        );
        verify_images(&dir, &old.states[old.cursor])?;
        old.clone()
    } else {
        History {
            schema: 1,
            writer: before.id.clone(),
            cursor: 0,
            states: vec![state(&dir, before, "Original learned examples")?],
        }
    };
    next.states.truncate(next.cursor + 1);
    next.states.push(state(&dir, after, label)?);
    while next.states.len() > MAX_STATES {
        next.states.remove(0);
    }
    next.cursor = next.states.len() - 1;
    publish(root, &before.id, before, after, old.as_ref(), next)
}
pub fn recover(root: &Path, id: &str) -> Result<Profile> {
    let _lock = lock(root, id)?;
    raw(root, id)
}
pub fn timeline(root: &Path, id: &str) -> Result<Timeline> {
    let _lock = lock(root, id)?;
    let dir = writers::folder(root, id)?;
    raw(root, id)?;
    let h = history(&dir, id)?;
    Ok(Timeline {
        cursor: h.as_ref().map_or(0, |h| h.cursor),
        labels: h.map_or_else(Vec::new, |h| {
            h.states.into_iter().map(|s| s.label).collect()
        }),
        note: RETENTION_NOTE.into(),
    })
}
fn move_cursor(root: &Path, id: &str, redo: bool) -> Result<Profile> {
    let _lock = lock(root, id)?;
    let dir = writers::folder(root, id)?;
    let before = raw(root, id)?;
    let old = history(&dir, id)?.ok_or_else(|| fail("No learned-example history is available"))?;
    anyhow::ensure!(
        same_content(&before, &old.states[old.cursor].profile)?,
        fail("Writer changed outside history; preserve a copy before undo or redo")
    );
    verify_images(&dir, &old.states[old.cursor])?;
    let mut next = old.clone();
    next.cursor = if redo {
        next.cursor
            .checked_add(1)
            .filter(|i| *i < next.states.len())
    } else {
        next.cursor.checked_sub(1)
    }
    .ok_or_else(|| {
        fail(if redo {
            "No learned-example edit to redo"
        } else {
            "No learned-example edit to undo"
        })
    })?;
    verify_images(&dir, &next.states[next.cursor])?;
    let mut after = next.states[next.cursor].profile.clone();
    after.revision = before.revision;
    writers::bump_revision(&mut after)?;
    publish(root, id, &before, &after, Some(&old), next)?;
    Ok(after)
}
pub fn undo(root: &Path, id: &str) -> Result<Profile> {
    move_cursor(root, id, false)
}
pub fn redo(root: &Path, id: &str) -> Result<Profile> {
    move_cursor(root, id, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn interrupted(root: &Path, profile: &Profile) -> Profile {
        let dir = writers::folder(root, &profile.id).unwrap();
        let mut after = profile.clone();
        after.notes = "durable pending change".into();
        writers::bump_revision(&mut after).unwrap();
        let h = History {
            schema: 1,
            writer: profile.id.clone(),
            cursor: 1,
            states: vec![
                state(&dir, profile, "Original").unwrap(),
                state(&dir, &after, "Pending edit").unwrap(),
            ],
        };
        atomic_json(
            &dir.join("writer-pending.json"),
            &Pending {
                schema: 1,
                before_history: None,
                before: profile.clone(),
                after: after.clone(),
                history: h,
            },
        )
        .unwrap();
        after
    }
    #[test]
    fn interrupted_profile_publication_replays_idempotently_before_next_mutation() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let p = writers::create(root, "Recover").unwrap();
        let after = interrupted(root, &p);
        assert!(writers::read(root, &p.id).is_err());
        atomic_json(&root.join(&p.id).join("profile.json"), &after).unwrap();
        let restored = recover(root, &p.id).unwrap();
        assert_eq!(restored.notes, "durable pending change");
        let timeline = timeline(root, &p.id).unwrap();
        assert_eq!(timeline.cursor, 1);
        assert_eq!(timeline.labels.len(), 2);
        assert_eq!(
            fingerprint(&recover(root, &p.id).unwrap()).unwrap(),
            fingerprint(&restored).unwrap()
        );
        let undone = undo(root, &p.id).unwrap();
        assert!(undone.notes.is_empty());
        assert!(undone.revision > restored.revision);
    }
    #[test]
    fn interrupted_transaction_never_overwrites_an_unrecognized_outside_edit() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let p = writers::create(root, "Conflict").unwrap();
        interrupted(root, &p);
        let mut outside = p.clone();
        outside.notes = "outside edit".into();
        let path = root.join(&p.id).join("profile.json");
        atomic_json(&path, &outside).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(recover(root, &p.id).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(root.join(&p.id).join("writer-pending.json").is_file());
        assert!(!root.join(&p.id).join("writer-history.json").exists());
    }
}
