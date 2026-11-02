//! Durable, source-preserving review revisions and previewed correction rules.
use crate::{
    storage::{Control, Project, atomic_json, fingerprint, read_json},
    types::*,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};

const HISTORY_LIMIT: u64 = 256_000_000;
const MAX_REVISIONS: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RevisionSummary {
    pub id: u64,
    pub label: String,
    pub created: String,
    pub pages: Vec<usize>,
    pub active: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Timeline {
    pub cursor: usize,
    pub revisions: Vec<RevisionSummary>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Change {
    before: Page,
    after: Page,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct FileChange {
    name: String,
    before: Option<Value>,
    after: Option<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Revision {
    id: u64,
    label: String,
    created: String,
    changes: Vec<Change>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    files: Vec<FileChange>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct History {
    schema: u32,
    project: String,
    next_id: u64,
    cursor: usize,
    revisions: Vec<Revision>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pending {
    schema: u32,
    project: String,
    before_history: String,
    history: History,
    changes: Vec<Change>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    files: Vec<FileChange>,
}
fn file_value(p: &Project, name: &str) -> Result<Option<Value>> {
    anyhow::ensure!(
        name == "guidance.json",
        fail("Unsupported review attachment")
    );
    let path = p.root.join(name);
    if path.is_file() {
        Ok(Some(read_json(&path, 2_000_000)?))
    } else {
        Ok(None)
    }
}
fn validate_files(files: &[FileChange]) -> Result<()> {
    anyhow::ensure!(files.len() <= 1, fail("Too many review attachments"));
    for f in files {
        anyhow::ensure!(
            f.name == "guidance.json"
                && serde_json::to_vec(&f.before)?.len() <= 2_000_000
                && serde_json::to_vec(&f.after)?.len() <= 2_000_000,
            fail("Invalid or oversized review attachment")
        );
    }
    Ok(())
}

fn history_path(p: &Project) -> PathBuf {
    p.root.join("review-history.json")
}
fn pending_path(p: &Project) -> PathBuf {
    p.root.join("review-pending.json")
}
fn overlay(p: &Project, page: usize) -> PathBuf {
    p.root.join(format!("page-{page:05}.edits.json"))
}
fn fail(message: &str) -> anyhow::Error {
    Fault::new("NG500", message).into()
}
fn lock(p: &Project) -> Result<fs::File> {
    let f = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(p.root.join("review.lock"))?;
    f.try_lock()
        .map_err(|_| fail("Another review operation is active; retry after it finishes"))?;
    Ok(f)
}
fn page_valid(p: &Project, page: &Page) -> Result<()> {
    page.validate()?;
    anyhow::ensure!(
        p.manifest.pages.contains(&page.page) && page.image == format!("page-{:05}.png", page.page),
        fail("Revision references an invalid source page")
    );
    anyhow::ensure!(
        page.agreement.is_finite()
            && (0.0..=1.0).contains(&page.agreement)
            && page.seconds.is_finite()
            && page.seconds >= 0.0
            && page.candidates.len() <= 1024
            && page.source_warnings.len() <= 1000
            && page.source_warnings.iter().all(|w| w.len() <= 8000),
        fail("Invalid revision evidence")
    );
    page.content.validate()?;
    for c in &page.candidates {
        anyhow::ensure!(
            c.reliability.is_finite()
                && c.reliability >= 0.0
                && c.model_id.len() <= 1000
                && c.family.len() <= 1000
                && c.view.len() <= 1000
                && c.region.as_ref().is_none_or(BoxRect::valid),
            fail("Invalid reader evidence in review revision")
        );
        c.content.validate()?;
    }
    Ok(())
}
fn empty(p: &Project) -> Result<History> {
    Ok(History {
        schema: 1,
        project: fingerprint(&p.manifest)?,
        next_id: 1,
        cursor: 0,
        revisions: vec![],
    })
}
fn read_history(p: &Project) -> Result<History> {
    let h: History = if history_path(p).is_file() {
        read_json(&history_path(p), HISTORY_LIMIT)?
    } else {
        empty(p)?
    };
    validate_history(p, &h)?;
    Ok(h)
}
fn validate_history(p: &Project, h: &History) -> Result<()> {
    anyhow::ensure!(
        h.schema == 1
            && h.project == fingerprint(&p.manifest)?
            && h.cursor <= h.revisions.len()
            && h.revisions.len() <= MAX_REVISIONS,
        fail("Invalid or mismatched review history")
    );
    let mut ids = BTreeSet::new();
    for r in &h.revisions {
        anyhow::ensure!(
            r.id > 0
                && r.id < h.next_id
                && ids.insert(r.id)
                && !r.label.trim().is_empty()
                && r.label.len() <= 200
                && (!r.changes.is_empty() || !r.files.is_empty())
                && r.changes.len() <= p.manifest.pages.len(),
            fail("Invalid review revision")
        );
        let mut pages = BTreeSet::new();
        for c in &r.changes {
            page_valid(p, &c.before)?;
            page_valid(p, &c.after)?;
            anyhow::ensure!(
                c.before.page == c.after.page && pages.insert(c.after.page),
                fail("Duplicate or mismatched revision pages")
            );
        }
        validate_files(&r.files)?;
    }
    Ok(())
}
fn recover_locked(p: &Project) -> Result<()> {
    if !pending_path(p).is_file() {
        return Ok(());
    }
    let pending: Pending = read_json(&pending_path(p), HISTORY_LIMIT)?;
    anyhow::ensure!(
        pending.schema == 1 && pending.project == fingerprint(&p.manifest)?,
        fail("Review recovery belongs to another project")
    );
    validate_history(p, &pending.history)?;
    let current = fingerprint(&read_history(p)?)?;
    anyhow::ensure!(
        current == pending.before_history || current == fingerprint(&pending.history)?,
        fail("Review history changed during recovery; pending transaction preserved")
    );
    for c in &pending.changes {
        page_valid(p, &c.before)?;
        page_valid(p, &c.after)?;
        anyhow::ensure!(c.before.page == c.after.page, fail("Invalid recovery page"));
        let current = fingerprint(&p.load(c.after.page)?)?;
        anyhow::ensure!(
            current == fingerprint(&c.before)? || current == fingerprint(&c.after)?,
            fail("A page changed outside the pending review; recovery preserved for inspection")
        );
    }
    validate_files(&pending.files)?;
    for f in &pending.files {
        let current = file_value(p, &f.name)?;
        anyhow::ensure!(
            fingerprint(&current)? == fingerprint(&f.before)?
                || fingerprint(&current)? == fingerprint(&f.after)?,
            fail("Context changed outside the pending review; recovery preserved for inspection")
        );
    }
    // A durable journal is written before any overlay. Replaying is idempotent.
    for c in &pending.changes {
        atomic_json(&overlay(p, c.after.page), &c.after)?;
    }
    for f in &pending.files {
        let path = p.root.join(&f.name);
        match &f.after {
            Some(value) => atomic_json(&path, value)?,
            None => {
                if path.is_file() {
                    fs::remove_file(&path)?;
                }
            }
        }
    }
    atomic_json(&history_path(p), &pending.history)?;
    fs::remove_file(pending_path(p))?;
    Ok(())
}
/// Finish an interrupted review transaction without touching original reader checkpoints.
pub fn recover(p: &Project) -> Result<()> {
    let _guard = lock(p)?;
    recover_locked(p)
}
fn transact(
    p: &Project,
    old: &History,
    next: History,
    changes: Vec<Change>,
    files: Vec<FileChange>,
) -> Result<Vec<Page>> {
    validate_history(p, &next)?;
    for c in &changes {
        page_valid(p, &c.before)?;
        page_valid(p, &c.after)?;
        anyhow::ensure!(
            fingerprint(&p.load(c.before.page)?)? == fingerprint(&c.before)?,
            fail("Review is stale; reload the page before applying this change")
        );
    }
    validate_files(&files)?;
    for f in &files {
        anyhow::ensure!(
            fingerprint(&file_value(p, &f.name)?)? == fingerprint(&f.before)?,
            fail("Context is stale; reload before saving")
        );
    }
    let pending = Pending {
        schema: 1,
        project: fingerprint(&p.manifest)?,
        before_history: fingerprint(old)?,
        history: next,
        changes,
        files,
    };
    anyhow::ensure!(
        serde_json::to_vec(&pending)?.len() as u64 <= HISTORY_LIMIT,
        fail("Review history limit reached; copy the project before starting a new history")
    );
    atomic_json(&pending_path(p), &pending)?;
    recover_locked(p)?;
    Ok(pending.changes.into_iter().map(|c| c.after).collect())
}
fn commit_locked(p: &Project, changes: Vec<Change>, label: &str) -> Result<u64> {
    commit_locked_files(p, changes, vec![], label)
}
fn commit_locked_files(
    p: &Project,
    changes: Vec<Change>,
    files: Vec<FileChange>,
    label: &str,
) -> Result<u64> {
    anyhow::ensure!(
        !label.trim().is_empty() && label.len() <= 200,
        fail("Revision name must contain 1–200 bytes")
    );
    anyhow::ensure!(
        !changes.is_empty() || !files.is_empty(),
        fail("No changes to save")
    );
    let old = read_history(p)?;
    for c in &changes {
        let known = old.revisions[..old.cursor]
            .iter()
            .rev()
            .find_map(|r| {
                r.changes
                    .iter()
                    .find(|change| change.after.page == c.before.page)
                    .map(|change| &change.after)
            })
            .or_else(|| {
                old.revisions.iter().find_map(|r| {
                    r.changes
                        .iter()
                        .find(|change| change.before.page == c.before.page)
                        .map(|change| &change.before)
                })
            });
        if let Some(known) = known {
            anyhow::ensure!(
                fingerprint(known)? == fingerprint(&c.before)?,
                fail(
                    "Page changed outside review history; preserve a project copy before starting a new history"
                )
            );
        }
    }
    for f in &files {
        let known = old.revisions[..old.cursor]
            .iter()
            .rev()
            .find_map(|r| {
                r.files
                    .iter()
                    .find(|change| change.name == f.name)
                    .map(|change| &change.after)
            })
            .or_else(|| {
                old.revisions.iter().find_map(|r| {
                    r.files
                        .iter()
                        .find(|change| change.name == f.name)
                        .map(|change| &change.before)
                })
            });
        if let Some(known) = known {
            anyhow::ensure!(
                fingerprint(known)? == fingerprint(&f.before)?,
                fail(
                    "Context changed outside review history; preserve a project copy before starting new history"
                )
            );
        }
    }
    let mut next = old.clone();
    next.revisions.truncate(next.cursor);
    anyhow::ensure!(
        next.revisions.len() < MAX_REVISIONS,
        fail("Review history limit reached; copy the project before starting a new history")
    );
    let id = next.next_id;
    next.next_id = next
        .next_id
        .checked_add(1)
        .ok_or_else(|| fail("Revision counter exhausted"))?;
    next.revisions.push(Revision {
        id,
        label: label.trim().into(),
        created: chrono::Utc::now().to_rfc3339(),
        changes: changes.clone(),
        files: files.clone(),
    });
    next.cursor = next.revisions.len();
    transact(p, &old, next, changes, files)?;
    Ok(id)
}
/// Commit a complete page edit only if its expected source state is still current.
pub fn commit(p: &Project, expected: &Page, replacement: &Page, label: &str) -> Result<u64> {
    commit_pages(p, &[(expected.clone(), replacement.clone())], label)
}
/// Commit selected edits across pages as one all-or-nothing undoable revision.
pub fn commit_pages(p: &Project, pages: &[(Page, Page)], label: &str) -> Result<u64> {
    let _guard = lock(p)?;
    recover_locked(p)?;
    let mut changes = vec![];
    let mut seen = BTreeSet::new();
    for (expected, replacement) in pages {
        anyhow::ensure!(
            expected.page == replacement.page && seen.insert(expected.page),
            fail("An edit cannot change or duplicate a source page identity")
        );
        let mut after = replacement.clone();
        after.edited = true;
        changes.push(Change {
            before: expected.clone(),
            after,
        });
    }
    commit_locked(p, changes, label)
}
/// Commit corrected pages and their clarification context in the same recoverable revision.
pub fn commit_guidance(
    p: &Project,
    pages: &[(Page, Page)],
    expected: &crate::assistance::Guidance,
    guidance: &crate::assistance::Guidance,
    label: &str,
) -> Result<u64> {
    let _guard = lock(p)?;
    recover_locked(p)?;
    anyhow::ensure!(
        guidance.notes.len() <= 1000
            && guidance.notes.iter().all(|n| n.len() <= 8000)
            && guidance.answers.len() <= 1000
            && guidance.answers.iter().all(|a| a.id.len() <= 200
                && a.replacement.len() <= 120000
                && a.context.len() <= 4000),
        fail("Clarification memory limits exceeded")
    );
    let mut changes = vec![];
    let mut seen = BTreeSet::new();
    for (before, replacement) in pages {
        anyhow::ensure!(
            before.page == replacement.page && seen.insert(before.page),
            fail("Invalid or duplicate clarification page")
        );
        let mut after = replacement.clone();
        after.edited = true;
        changes.push(Change {
            before: before.clone(),
            after,
        });
    }
    let before = file_value(p, "guidance.json")?;
    let current: crate::assistance::Guidance = before
        .clone()
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_default();
    anyhow::ensure!(
        fingerprint(&current)? == fingerprint(expected)?,
        fail("Project context changed while editing; reload the context before saving")
    );
    let after = Some(serde_json::to_value(guidance)?);
    let files = if fingerprint(&before)? == fingerprint(&after)? {
        vec![]
    } else {
        vec![FileChange {
            name: "guidance.json".into(),
            before,
            after,
        }]
    };
    commit_locked_files(p, changes, files, label)
}
pub fn timeline(p: &Project) -> Result<Timeline> {
    let _guard = lock(p)?;
    recover_locked(p)?;
    let h = read_history(p)?;
    Ok(Timeline {
        cursor: h.cursor,
        revisions: h
            .revisions
            .iter()
            .enumerate()
            .map(|(i, r)| RevisionSummary {
                id: r.id,
                label: r.label.clone(),
                created: r.created.clone(),
                pages: r.changes.iter().map(|c| c.after.page).collect(),
                active: i < h.cursor,
            })
            .collect(),
    })
}
fn move_cursor(p: &Project, old: &History, target: usize) -> Result<Vec<Page>> {
    anyhow::ensure!(
        target <= old.revisions.len(),
        fail("Unknown review revision")
    );
    if target == old.cursor {
        return Ok(vec![]);
    }
    let mut selected: BTreeMap<usize, Change> = BTreeMap::new();
    let mut files: BTreeMap<String, FileChange> = BTreeMap::new();
    let steps: Vec<&Revision> = if target < old.cursor {
        old.revisions[target..old.cursor].iter().rev().collect()
    } else {
        old.revisions[old.cursor..target].iter().collect()
    };
    for revision in steps {
        for c in &revision.changes {
            let (from, to) = if target < old.cursor {
                (&c.after, &c.before)
            } else {
                (&c.before, &c.after)
            };
            let entry = selected.entry(from.page).or_insert_with(|| Change {
                before: from.clone(),
                after: from.clone(),
            });
            anyhow::ensure!(
                fingerprint(&entry.after)? == fingerprint(from)?,
                fail("Revision chain does not match; original history preserved")
            );
            entry.after = to.clone();
        }
        for f in &revision.files {
            let (from, to) = if target < old.cursor {
                (&f.after, &f.before)
            } else {
                (&f.before, &f.after)
            };
            let entry = files.entry(f.name.clone()).or_insert_with(|| FileChange {
                name: f.name.clone(),
                before: from.clone(),
                after: from.clone(),
            });
            anyhow::ensure!(
                fingerprint(&entry.after)? == fingerprint(from)?,
                fail("Context revision chain does not match; original history preserved")
            );
            entry.after = to.clone();
        }
    }
    let mut next = old.clone();
    next.cursor = target;
    transact(
        p,
        old,
        next,
        selected.into_values().collect(),
        files.into_values().collect(),
    )
}
pub fn undo(p: &Project) -> Result<Vec<Page>> {
    let _guard = lock(p)?;
    recover_locked(p)?;
    let h = read_history(p)?;
    anyhow::ensure!(h.cursor > 0, fail("Nothing to undo"));
    move_cursor(p, &h, h.cursor - 1)
}
pub fn redo(p: &Project) -> Result<Vec<Page>> {
    let _guard = lock(p)?;
    recover_locked(p)?;
    let h = read_history(p)?;
    anyhow::ensure!(h.cursor < h.revisions.len(), fail("Nothing to redo"));
    move_cursor(p, &h, h.cursor + 1)
}
/// Restore the state immediately after a named revision; revision 0 is the initial state.
pub fn restore(p: &Project, revision: u64) -> Result<Vec<Page>> {
    let _guard = lock(p)?;
    recover_locked(p)?;
    let h = read_history(p)?;
    let cursor = if revision == 0 {
        0
    } else {
        h.revisions
            .iter()
            .position(|r| r.id == revision)
            .map(|i| i + 1)
            .ok_or_else(|| fail("Unknown review revision"))?
    };
    move_cursor(p, &h, cursor)
}
/// Read a saved page at a revision without moving the current review cursor.
pub fn snapshot(p: &Project, revision: u64, page: usize) -> Result<Page> {
    let _guard = lock(p)?;
    recover_locked(p)?;
    let h = read_history(p)?;
    anyhow::ensure!(
        p.manifest.pages.contains(&page),
        fail("Unknown source page")
    );
    let cursor = if revision == 0 {
        0
    } else {
        h.revisions
            .iter()
            .position(|r| r.id == revision)
            .map(|i| i + 1)
            .ok_or_else(|| fail("Unknown review revision"))?
    };
    if let Some(saved) = h.revisions[..cursor].iter().rev().find_map(|r| {
        r.changes
            .iter()
            .find(|c| c.after.page == page)
            .map(|c| &c.after)
    }) {
        return Ok(saved.clone());
    }
    if let Some(saved) = h.revisions.iter().find_map(|r| {
        r.changes
            .iter()
            .find(|c| c.before.page == page)
            .map(|c| &c.before)
    }) {
        return Ok(saved.clone());
    }
    p.load(page)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Difference {
    pub page: usize,
    pub block: Option<usize>,
    pub field: String,
    pub before: Value,
    pub after: Value,
}
/// Field-level comparison retains source geometry, uncertainty and reader evidence changes.
pub fn compare(before: &Page, after: &Page) -> Result<Vec<Difference>> {
    anyhow::ensure!(
        before.page == after.page,
        fail("Compare requires the same source page")
    );
    let a = serde_json::to_value(before)?;
    let b = serde_json::to_value(after)?;
    let mut out = vec![];
    for field in [
        "reviewed",
        "edited",
        "agreement",
        "source_warnings",
        "candidates",
    ] {
        if a[field] != b[field] {
            out.push(Difference {
                page: before.page,
                block: None,
                field: field.into(),
                before: a[field].clone(),
                after: b[field].clone(),
            });
        }
    }
    for field in ["warnings", "summary"] {
        if a["content"][field] != b["content"][field] {
            out.push(Difference {
                page: before.page,
                block: None,
                field: field.into(),
                before: a["content"][field].clone(),
                after: b["content"][field].clone(),
            });
        }
    }
    for i in 0..before.content.blocks.len().max(after.content.blocks.len()) {
        let av = before
            .content
            .blocks
            .get(i)
            .map(serde_json::to_value)
            .transpose()?
            .unwrap_or(Value::Null);
        let bv = after
            .content
            .blocks
            .get(i)
            .map(serde_json::to_value)
            .transpose()?
            .unwrap_or(Value::Null);
        if av == bv {
            continue;
        }
        if av.is_null() || bv.is_null() {
            out.push(Difference {
                page: before.page,
                block: Some(i),
                field: "block".into(),
                before: av,
                after: bv,
            });
            continue;
        }
        for field in [
            "kind",
            "text",
            "box",
            "confidence",
            "rows",
            "merges",
            "header_rows",
            "description",
            "interpretation",
            "uncertain",
            "alternatives",
        ] {
            if av[field] != bv[field] {
                out.push(Difference {
                    page: before.page,
                    block: Some(i),
                    field: field.into(),
                    before: av[field].clone(),
                    after: bv[field].clone(),
                });
            }
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub page: usize,
    pub block: usize,
    pub row: Option<usize>,
    pub col: Option<usize>,
    pub priority: u8,
    pub kind: String,
    pub message: String,
    pub reading: String,
    pub region: Option<BoxRect>,
    pub alternatives: Vec<String>,
    #[serde(default)]
    pub preview_truncated: bool,
}
/// Real issues retained globally; an additional page-zero notice reports omissions.
pub const MAX_REVIEW_ISSUES: usize = 10_000;
/// Highest-impact details retained for a single table.
pub const MAX_TABLE_ISSUES: usize = 128;
pub const ISSUE_PREVIEW_CHARS: usize = 240;
pub const ISSUE_PREVIEW_ALTERNATIVES: usize = 4;
fn short_preview(mut characters: impl Iterator<Item = char>) -> (String, bool) {
    let mut text: String = characters.by_ref().take(ISSUE_PREVIEW_CHARS).collect();
    let truncated = characters.next().is_some();
    if truncated {
        text.push('…');
    }
    (text, truncated)
}
fn block_preview(b: &Block) -> (String, bool) {
    if b.kind == "table" {
        short_preview(b.rows.iter().enumerate().flat_map(|(row, values)| {
            values.iter().enumerate().flat_map(move |(col, value)| {
                let separator = if col > 0 {
                    Some('\t')
                } else if row > 0 {
                    Some('\n')
                } else {
                    None
                };
                separator.into_iter().chain(value.chars())
            })
        }))
    } else {
        short_preview(b.text.chars())
    }
}
fn issue(page: usize, block: usize, b: &Block, priority: u8, kind: &str, message: &str) -> Issue {
    issue_with_reading(page, block, b, priority, kind, message, None)
}
fn issue_with_reading(
    page: usize,
    block: usize,
    b: &Block,
    priority: u8,
    kind: &str,
    message: &str,
    reading: Option<&str>,
) -> Issue {
    let (reading, mut truncated) =
        reading.map_or_else(|| block_preview(b), |text| short_preview(text.chars()));
    let alternatives = b
        .alternatives
        .iter()
        .take(ISSUE_PREVIEW_ALTERNATIVES)
        .map(|text| {
            let (preview, shortened) = short_preview(text.chars());
            truncated |= shortened;
            preview
        })
        .collect();
    truncated |= b.alternatives.len() > ISSUE_PREVIEW_ALTERNATIVES;
    Issue {
        id: format!("p{page}-b{block}-{kind}"),
        page,
        block,
        row: None,
        col: None,
        priority,
        kind: kind.into(),
        message: if truncated {
            format!(
                "{message} Preview shortened; full text and all {} reader alternatives remain in the project.",
                b.alternatives.len()
            )
        } else {
            message.into()
        },
        reading,
        region: b.bbox.as_ref().filter(|region| region.valid()).cloned(),
        alternatives,
        preview_truncated: truncated,
    }
}
type IssueRank = (
    std::cmp::Reverse<u8>,
    usize,
    usize,
    Option<usize>,
    Option<usize>,
    String,
);
struct RankedIssue {
    rank: IssueRank,
    issue: Issue,
}
impl PartialEq for RankedIssue {
    fn eq(&self, other: &Self) -> bool {
        self.rank == other.rank
    }
}
impl Eq for RankedIssue {}
impl PartialOrd for RankedIssue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for RankedIssue {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank.cmp(&other.rank)
    }
}
struct BoundedIssues {
    heap: std::collections::BinaryHeap<RankedIssue>,
    limit: usize,
    total: usize,
}
impl BoundedIssues {
    fn new(limit: usize) -> Self {
        Self {
            heap: std::collections::BinaryHeap::with_capacity(limit),
            limit,
            total: 0,
        }
    }
    fn retain(&mut self, issue: Issue) {
        let rank = (
            std::cmp::Reverse(issue.priority),
            issue.page,
            issue.block,
            issue.row,
            issue.col,
            issue.id.clone(),
        );
        if self.heap.len() < self.limit {
            self.heap.push(RankedIssue { rank, issue });
        } else if self.heap.peek().is_some_and(|worst| rank < worst.rank) {
            self.heap.pop();
            self.heap.push(RankedIssue { rank, issue });
        }
    }
    fn push(&mut self, issue: Issue) {
        self.total = self.total.saturating_add(1);
        self.retain(issue);
    }
    fn absorb(&mut self, batch: Self) {
        self.total = self.total.saturating_add(batch.total);
        for retained in batch.heap {
            self.retain(retained.issue);
        }
    }
    fn sorted(self) -> Vec<Issue> {
        self.heap
            .into_sorted_vec()
            .into_iter()
            .map(|ranked| ranked.issue)
            .collect()
    }
}
fn truncation_notice(kind: &str, message: String) -> Issue {
    Issue {
        id: format!("review-{kind}-notice"),
        page: 0,
        block: 0,
        row: None,
        col: None,
        priority: 255,
        kind: kind.into(),
        message,
        reading: String::new(),
        region: None,
        alternatives: vec![],
        preview_truncated: true,
    }
}
fn number(value: &str) -> Option<f64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let value = value.strip_prefix(['$', '£', '€']).unwrap_or(value).trim();
    let negative = value.starts_with('(') && value.ends_with(')');
    let value = if negative {
        &value[1..value.len() - 1]
    } else {
        value
    };
    // Ambiguous grouped/decimal punctuation is deliberately left unclassified.
    if value.contains(',') || value.contains('%') {
        return None;
    }
    let n = value.parse::<f64>().ok()?;
    n.is_finite().then_some(if negative { -n } else { n })
}
fn total_label(value: &str) -> Option<&'static str> {
    let value = value.trim();
    ["total", "grand total", "subtotal"]
        .into_iter()
        .find(|label| value.eq_ignore_ascii_case(label))
}
fn numeric_tokens(value: &str) -> impl Iterator<Item = &str> {
    static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"[-+]?\d+(?:[.,]\d+)*(?:[eE][-+]?\d+)?").expect("fixed numeric pattern")
    });
    PATTERN.find_iter(value).map(|m| m.as_str())
}
/// Conservative table suggestions. Values and source evidence are never rewritten.
pub fn table_issues(page: usize, index: usize, b: &Block) -> Vec<Issue> {
    let candidates = table_candidates(page, index, b);
    let total = candidates.total;
    let mut retained = candidates.sorted();
    if total > retained.len() {
        retained.insert(0, truncation_notice("table-truncated", format!("Table preview for page {page}, block {} retains the {MAX_TABLE_ISSUES} highest-impact suggestions; {} further suggestions were omitted. Full source and reader evidence remain in the project.", index + 1, total - retained.len())));
    }
    retained
}
fn table_candidates(page: usize, index: usize, b: &Block) -> BoundedIssues {
    let mut out = BoundedIssues::new(MAX_TABLE_ISSUES);
    if b.kind != "table" || b.rows.is_empty() {
        return out;
    }
    let cols = b.rows[0].len();
    if b.rows.len() > 100 || cols == 0 || cols > 40 || b.header_rows > b.rows.len() {
        out.push(issue(page, index, b, 95, "table-dimensions", "The table exceeds supported row/column/header limits; repair or split it before export."));
        return out;
    }
    if b.rows.iter().any(|r| r.len() != cols) {
        out.push(issue(
            page,
            index,
            b,
            95,
            "row-length",
            "Rows have different column counts; repair the grid before export.",
        ));
        return out;
    }
    for col in 1..cols {
        let values: Vec<(usize, &str)> = b
            .rows
            .iter()
            .enumerate()
            .skip(b.header_rows)
            .filter_map(|(r, row)| row.get(col).map(|s| (r, s.as_str())))
            .filter(|(_, s)| !s.trim().is_empty())
            .collect();
        let numeric = values.iter().filter(|(_, s)| number(s).is_some()).count();
        if numeric >= 2 && numeric * 2 > values.len() {
            for (r, s) in &values {
                if number(s).is_none()
                    && !["—", "-", "n/a"]
                        .iter()
                        .any(|missing| s.trim().eq_ignore_ascii_case(missing))
                {
                    let mut i = issue_with_reading(
                        page,
                        index,
                        b,
                        75,
                        "column-type",
                        "This cell differs from the mostly numeric column; confirm its digits, units or intended text.",
                        Some(s),
                    );
                    i.id = format!("p{page}-b{index}-r{r}-c{col}-type");
                    i.row = Some(*r);
                    i.col = Some(col);
                    out.push(i);
                }
            }
        }
    }
    if !b.merges.is_empty() {
        return out;
    }
    for (r, row) in b.rows.iter().enumerate().skip(b.header_rows) {
        let Some(label) = row.first().and_then(|value| total_label(value)) else {
            continue;
        };
        for (col, stated) in row.iter().enumerate().skip(1) {
            let Some(stated) = number(stated) else {
                continue;
            };
            let start = if label == "grand total" {
                b.header_rows
            } else {
                b.rows[..r]
                    .iter()
                    .rposition(|row| {
                        row.first()
                            .is_some_and(|v| matches!(total_label(v), Some("total" | "subtotal")))
                    })
                    .map_or(b.header_rows, |i| i + 1)
            };
            let mut terms = vec![];
            let mut complete = true;
            for source in &b.rows[start..r] {
                if source.first().is_some_and(|v| total_label(v).is_some()) {
                    continue;
                }
                let value = &source[col];
                if value.trim().is_empty() {
                    continue;
                }
                if let Some(n) = number(value) {
                    terms.push(n);
                } else {
                    complete = false;
                }
            }
            if !complete || terms.len() < 2 {
                continue;
            }
            let calculated: f64 = terms.iter().sum();
            if calculated.is_finite()
                && (calculated - stated).abs() > 0.005_f64.max(stated.abs() * 1e-9)
            {
                let reading = format!("Stated {stated}; calculated {calculated}");
                let mut i = issue_with_reading(
                    page,
                    index,
                    b,
                    98,
                    "total",
                    "The stated total differs from the numeric cells above. Check the source; no value has been changed.",
                    Some(&reading),
                );
                i.id = format!("p{page}-b{index}-r{r}-c{col}-total");
                i.row = Some(r);
                i.col = Some(col);
                out.push(i);
            }
        }
    }
    out
}
/// Pending review ordered by likely impact, with stable source locations.
pub fn issues(p: &Project) -> Result<Vec<Issue>> {
    let _guard = lock(p)?;
    recover_locked(p)?;
    let mut out = BoundedIssues::new(MAX_REVIEW_ISSUES);
    for n in &p.manifest.pages {
        let page = p.load(*n)?;
        if page.reviewed {
            continue;
        }
        let visible = crate::redactions::visible_blocks(p, &page, &Control::default())?;
        for (index, b) in visible.iter().map(|(index, block)| (*index, block)) {
            out.absorb(table_candidates(*n, index, b));
            let reading = b.plain();
            let numeric = reading.chars().any(|c| c.is_ascii_digit());
            if numeric
                && b.alternatives.iter().any(|a| {
                    let alternative = a
                        .split_once(": ")
                        .filter(|(prefix, _)| !prefix.contains(char::is_whitespace))
                        .map_or(a.as_str(), |(_, text)| text);
                    numeric_tokens(alternative).ne(numeric_tokens(&reading))
                })
            {
                out.push(issue(
                    *n,
                    index,
                    b,
                    100,
                    "numeric-conflict",
                    "Readers disagree on digits or signs; confirm against the source.",
                ));
            } else if b.uncertain
                || b.confidence
                    .is_none_or(|c| c < p.manifest.settings.review_threshold)
                || reading
                    .as_bytes()
                    .windows(b"[illegible]".len())
                    .any(|span| span.eq_ignore_ascii_case(b"[illegible]"))
            {
                let priority = if numeric {
                    90
                } else if b.kind == "equation" {
                    85
                } else {
                    65
                };
                out.push(issue(
                    *n,
                    index,
                    b,
                    priority,
                    "uncertain",
                    "Confirm this source region and its reader alternatives.",
                ));
            } else if b.kind == "figure" {
                out.push(issue(
                    *n,
                    index,
                    b,
                    60,
                    "interpretation",
                    "Confirm the interpretation separately from visible source evidence.",
                ));
            }
        }
    }
    let total = out.total;
    let mut retained = out.sorted();
    if total > retained.len() {
        retained.insert(0, truncation_notice("queue-truncated", format!("Review queue retains {} of {total} suggestions; {} were omitted. Limits are {MAX_REVIEW_ISSUES} issues overall and {MAX_TABLE_ISSUES} per table, keeping higher-impact issues first. Full source and reader evidence remain in the project. Review/approve retained pages and refresh to expose more suggestions.", retained.len(), total - retained.len())));
    }
    Ok(retained)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub find: String,
    pub replace: String,
    pub pages: Vec<usize>,
    pub exclude_blocks: Vec<(usize, usize)>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuleMatch {
    pub page: usize,
    pub block: usize,
    pub field: String,
    pub before: String,
    pub after: String,
    pub occurrences: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RulePreview {
    pub rule: Rule,
    pub matches: Vec<RuleMatch>,
    pub page_digests: BTreeMap<usize, String>,
}
fn validate_rule(p: &Project, r: &Rule) -> Result<()> {
    anyhow::ensure!(
        !r.find.is_empty()
            && r.find.len() <= 1000
            && r.replace.len() <= 1000
            && !r.pages.is_empty()
            && r.pages.len() <= 3000
            && r.exclude_blocks.len() <= 10000
            && r.pages.iter().all(|n| p.manifest.pages.contains(n))
            && r.pages.iter().collect::<BTreeSet<_>>().len() == r.pages.len(),
        fail("Invalid correction rule: select valid pages and a nonempty literal search")
    );
    Ok(())
}
fn preview_locked(p: &Project, r: &Rule) -> Result<RulePreview> {
    validate_rule(p, r)?;
    let mut preview = RulePreview {
        rule: r.clone(),
        matches: vec![],
        page_digests: BTreeMap::new(),
    };
    for n in &r.pages {
        let page = p.load(*n)?;
        preview.page_digests.insert(*n, fingerprint(&page)?);
        let visible = crate::redactions::visible_blocks(p, &page, &Control::default())?;
        for (index, b) in visible.iter().map(|(index, block)| (*index, block)) {
            if r.exclude_blocks.contains(&(*n, index)) {
                continue;
            }
            let mut fields = vec![
                ("text".to_string(), &b.text),
                ("description".to_string(), &b.description),
                ("interpretation".to_string(), &b.interpretation),
            ];
            for (row, cells) in b.rows.iter().enumerate() {
                for (col, cell) in cells.iter().enumerate() {
                    fields.push((format!("cell:{row}:{col}"), cell));
                }
            }
            for (field, before) in fields {
                let occurrences = before.matches(&r.find).count();
                if occurrences == 0 {
                    continue;
                }
                let after = before.replace(&r.find, &r.replace);
                anyhow::ensure!(
                    after.len() < 120000,
                    fail("Rule replacement would exceed a field limit")
                );
                preview.matches.push(RuleMatch {
                    page: *n,
                    block: index,
                    field,
                    before: before.clone(),
                    after,
                    occurrences,
                });
                anyhow::ensure!(
                    preview.matches.len() <= 50000,
                    fail("Rule preview exceeds 50,000 fields; select fewer pages")
                );
            }
        }
    }
    Ok(preview)
}
/// Case-sensitive literal replacements are previewed before any write; reader evidence is excluded.
pub fn preview_rule(p: &Project, r: &Rule) -> Result<RulePreview> {
    let _guard = lock(p)?;
    recover_locked(p)?;
    preview_locked(p, r)
}
/// Apply the exact inspected preview as one undoable transaction, rejecting stale or altered previews.
pub fn apply_rule(p: &Project, preview: &RulePreview, label: &str) -> Result<u64> {
    let _guard = lock(p)?;
    recover_locked(p)?;
    anyhow::ensure!(
        fingerprint(preview)? == fingerprint(&preview_locked(p, &preview.rule)?)?,
        fail("Rule preview is stale or changed; preview it again")
    );
    let mut changes = BTreeMap::new();
    for m in &preview.matches {
        if let std::collections::btree_map::Entry::Vacant(entry) = changes.entry(m.page) {
            let before = p.load(m.page)?;
            entry.insert(Change {
                after: before.clone(),
                before,
            });
        }
        let c = changes
            .get_mut(&m.page)
            .ok_or_else(|| fail("Invalid preview page"))?;
        let b = c
            .after
            .content
            .blocks
            .get_mut(m.block)
            .ok_or_else(|| fail("Invalid preview block"))?;
        match m.field.as_str() {
            "text" => b.text = m.after.clone(),
            "description" => b.description = m.after.clone(),
            "interpretation" => b.interpretation = m.after.clone(),
            field => {
                let parts: Vec<_> = field.split(':').collect();
                anyhow::ensure!(
                    parts.len() == 3 && parts[0] == "cell",
                    fail("Invalid correction field")
                );
                let row = parts[1].parse::<usize>()?;
                let col = parts[2].parse::<usize>()?;
                let cell = b
                    .rows
                    .get_mut(row)
                    .and_then(|r| r.get_mut(col))
                    .ok_or_else(|| fail("Invalid correction cell"))?;
                *cell = m.after.clone();
            }
        }
        c.after.edited = true;
        c.after.reviewed = false;
    }
    commit_locked(p, changes.into_values().collect(), label)
}
