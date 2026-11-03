//! Explicit human annotation labels. Recognition never assigns or removes these labels.
use crate::{
    review,
    storage::{Project, fingerprint},
    types::{BoxRect, Fault, Page, Result},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_LAYERS_PER_PAGE: usize = 1000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    OriginalText,
    Highlight,
    MarginalNote,
    Stamp,
    TeacherCorrection,
    Other,
}
impl LayerKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::OriginalText => "original text",
            Self::Highlight => "highlight",
            Self::MarginalNote => "marginal note",
            Self::Stamp => "stamp",
            Self::TeacherCorrection => "teacher correction",
            Self::Other => "other annotation",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    pub id: String,
    pub kind: LayerKind,
    /// Zero-based index AND full serialized block fingerprint must both still match.
    pub block: usize,
    pub block_fingerprint: String,
    pub label: String,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedLayer {
    pub assignment: Assignment,
    pub page: usize,
    /// `confirmed` means the manually selected block is unchanged, not that OCR is correct.
    pub status: String,
    pub source_region: Option<BoxRect>,
    pub message: String,
}
impl ResolvedLayer {
    pub fn confirmed(&self) -> bool {
        self.status == "confirmed"
    }
    pub fn caption(&self) -> String {
        let a = &self.assignment;
        format!(
            "MANUAL LAYER: {}{}{} — source page {}, assigned block {}, fingerprint {}. {}{}{}",
            a.kind.name(),
            if a.label.is_empty() { "" } else { ": " },
            a.label,
            self.page,
            a.block + 1,
            a.block_fingerprint,
            self.message,
            if a.note.is_empty() { "" } else { " Note: " },
            a.note,
        )
    }
}
fn fail(message: &str) -> anyhow::Error {
    Fault::new("NG500", message).into()
}
fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}
fn text_valid(label: &str, note: &str) -> bool {
    label.len() <= 128
        && note.len() <= 1024
        && label.chars().all(|c| !c.is_control())
        && note
            .chars()
            .all(|c| !c.is_control() || matches!(c, '\n' | '\t'))
}
/// Validate bounded metadata without rejecting stale assignments: stale labels remain reviewable.
pub fn validate_metadata(assignments: &[Assignment]) -> Result<()> {
    anyhow::ensure!(
        assignments.len() <= MAX_LAYERS_PER_PAGE,
        fail("Too many manual annotation layers")
    );
    let mut ids = BTreeSet::new();
    let mut targets = BTreeSet::new();
    for a in assignments {
        anyhow::ensure!(
            hash(&a.id)
                && hash(&a.block_fingerprint)
                && a.block < 500
                && text_valid(&a.label, &a.note)
                && ids.insert(a.id.as_str())
                && targets.insert((a.block, a.kind)),
            fail("Invalid, duplicate or excessive manual annotation metadata")
        );
    }
    Ok(())
}
/// Resolve only exact source bindings. A changed/deleted/reordered block never inherits a label.
pub fn report(page: &Page) -> Result<Vec<ResolvedLayer>> {
    validate_metadata(&page.manual_layers)?;
    let mut digests = BTreeMap::new();
    for a in &page.manual_layers {
        if let std::collections::btree_map::Entry::Vacant(entry) = digests.entry(a.block) {
            entry.insert(
                page.content
                    .blocks
                    .get(a.block)
                    .map(fingerprint)
                    .transpose()?,
            );
        }
    }
    Ok(page.manual_layers.iter().map(|a| {
        let current = digests.get(&a.block).and_then(Option::as_ref) == Some(&a.block_fingerprint);
        ResolvedLayer {
            assignment: a.clone(), page: page.page,
            status: if current { "confirmed" } else { "stale" }.into(),
            source_region: if current { page.content.blocks[a.block].bbox.clone() } else { None },
            message: if current {
                "Human-assigned label; the selected block is unchanged. All content is retained."
            } else {
                "REVIEW NEEDED: the assigned block changed or disappeared. This label is unbound until explicitly reconfirmed; all content is retained."
            }.into(),
        }
    }).collect())
}
pub fn needs_review(page: &Page) -> bool {
    !page.manual_layers.is_empty()
        && report(page).map_or(true, |r| r.iter().any(|a| !a.confirmed()))
}
fn commit(project: &Project, expected: &Page, mut after: Page, label: &str) -> Result<u64> {
    after.edited = true;
    after.reviewed = false;
    after.validate()?;
    anyhow::ensure!(
        serde_json::to_vec(&after)?.len() <= 32_000_000,
        fail("Manual layer edit exceeds the checkpoint size limit")
    );
    review::commit(project, expected, &after, label)
}
/// Add a manual label, or explicitly replace an existing ID. No text or source image is changed.
pub fn set(
    project: &Project,
    expected: &Page,
    block: usize,
    kind: LayerKind,
    label: &str,
    note: &str,
    id: Option<&str>,
) -> Result<u64> {
    expected.validate()?;
    anyhow::ensure!(
        text_valid(label, note),
        fail("Manual layer label or note is invalid or too long")
    );
    let source = expected
        .content
        .blocks
        .get(block)
        .ok_or_else(|| fail("Choose an existing source block"))?;
    let digest = fingerprint(source)?;
    let mut after = expected.clone();
    let new_id = if let Some(id) = id {
        let index = after
            .manual_layers
            .iter()
            .position(|a| a.id == id)
            .ok_or_else(|| fail("Unknown manual annotation ID"))?;
        after.manual_layers.remove(index).id
    } else {
        fingerprint(&(
            expected.page,
            block,
            kind,
            label,
            note,
            &expected.manual_layers,
        ))?
    };
    after.manual_layers.push(Assignment {
        id: new_id,
        kind,
        block,
        block_fingerprint: digest,
        label: label.into(),
        note: note.into(),
    });
    commit(project, expected, after, "Assign manual annotation layer")
}
/// Remove only the selected label. The annotated content is always retained.
pub fn remove(project: &Project, expected: &Page, id: &str) -> Result<u64> {
    let mut after = expected.clone();
    let index = after
        .manual_layers
        .iter()
        .position(|a| a.id == id)
        .ok_or_else(|| fail("Unknown manual annotation ID"))?;
    after.manual_layers.remove(index);
    commit(project, expected, after, "Remove manual annotation label")
}
/// Explicitly rebind a stale label to a selected current block after human comparison.
pub fn reconfirm(project: &Project, expected: &Page, id: &str, block: usize) -> Result<u64> {
    let a = expected
        .manual_layers
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| fail("Unknown manual annotation ID"))?;
    set(
        project,
        expected,
        block,
        a.kind,
        &a.label,
        &a.note,
        Some(id),
    )
}
