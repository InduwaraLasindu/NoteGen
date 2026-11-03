//! Explicit, source-bound footnote/citation relationships. No inferred semantic links.
use crate::{
    geometry,
    redactions::{self, SourceIdentity},
    storage::{Control, Project, fingerprint},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
fn fail(message: &str) -> anyhow::Error {
    Fault::new("NG201", message).into()
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Footnote,
    Citation,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Bibliography {
    pub title: String,
    pub author: String,
    pub year: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Location {
    pub page: usize,
    pub source: SourceIdentity,
    pub block_hint: usize,
    pub block_sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConfirmedLink {
    pub schema: u32,
    pub id: String,
    pub kind: Kind,
    pub anchor: Location,
    pub target: Location,
    pub marker_start: usize,
    pub marker_end: usize,
    pub marker: String,
    pub bibliography: Option<Bibliography>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub kind: Kind,
    pub anchor_block: usize,
    pub marker_start: usize,
    pub marker_end: usize,
    pub target_page: usize,
    pub target_block: usize,
    pub bibliography: Option<Bibliography>,
}
fn hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn printable(s: &str, max: usize) -> bool {
    s.len() <= max && !s.chars().any(char::is_control)
}
fn link_id(link: &ConfirmedLink) -> Result<String> {
    fingerprint(&(
        1,
        link.kind,
        &link.anchor,
        &link.target,
        link.marker_start,
        link.marker_end,
        &link.marker,
        &link.bibliography,
    ))
}
pub fn validate_metadata(links: &[ConfirmedLink]) -> Result<()> {
    anyhow::ensure!(
        links.len() <= 256,
        fail("A page exceeds 256 confirmed reference relationships")
    );
    let mut ids = BTreeSet::new();
    for link in links {
        for location in [&link.anchor, &link.target] {
            geometry::validate_source(&location.source)?;
            anyhow::ensure!(
                location.page == location.source.page
                    && location.block_hint < 500
                    && hash(&location.block_sha256),
                fail("Invalid confirmed reference location")
            );
        }
        anyhow::ensure!(
            link.schema == 1
                && hash(&link.id)
                && ids.insert(&link.id)
                && link.id == link_id(link)?
                && link.marker_start < link.marker_end
                && link.marker_end <= 120_000
                && link.marker_end - link.marker_start == link.marker.len()
                && !link.marker.trim().is_empty()
                && printable(&link.marker, 256),
            fail("Invalid confirmed reference marker")
        );
        match (&link.kind, &link.bibliography) {
            (Kind::Footnote, None) => {}
            (Kind::Citation, Some(b)) => anyhow::ensure!(
                !b.title.trim().is_empty()
                    && printable(&b.title, 2048)
                    && printable(&b.author, 512)
                    && printable(&b.year, 32),
                fail("Confirm bounded bibliography title, author and year explicitly")
            ),
            _ => {
                return Err(fail(
                    "Only citations carry explicitly confirmed bibliography metadata",
                ));
            }
        }
    }
    Ok(())
}
fn source(project: &Project, page: &Page, c: &Control) -> Result<SourceIdentity> {
    let source = redactions::source_identity(project, page, c)?;
    redactions::validate_source(&page.confirmed_redactions, &source)?;
    Ok(source)
}
fn visible(page: &Page, block: &Block) -> bool {
    page.confirmed_redactions.is_empty()
        || block.bbox.as_ref().is_some_and(|b| {
            page.confirmed_redactions.iter().all(|m| {
                b.right <= m.region.left
                    || b.left >= m.region.right
                    || b.bottom <= m.region.top
                    || b.top >= m.region.bottom
            })
        })
}
fn prose(block: &Block) -> bool {
    matches!(
        block.kind.as_str(),
        "text" | "heading" | "list" | "caption" | "reference"
    )
}
fn location(project: &Project, page: &Page, index: usize, c: &Control) -> Result<Location> {
    let block = page
        .content
        .blocks
        .get(index)
        .ok_or_else(|| fail("Reference block no longer exists"))?;
    anyhow::ensure!(
        prose(block) && visible(page, block),
        fail(
            "Reference anchors/bodies must be visible prose blocks; covered or ungrounded masked content cannot become semantic links"
        )
    );
    Ok(Location {
        page: page.page,
        source: source(project, page, c)?,
        block_hint: index,
        block_sha256: fingerprint(block)?,
    })
}
fn find_block(page: &Page, where_: &Location) -> Result<usize> {
    if page
        .content
        .blocks
        .get(where_.block_hint)
        .is_some_and(|b| fingerprint(b).is_ok_and(|s| s == where_.block_sha256))
    {
        return Ok(where_.block_hint);
    }
    let found = page
        .content
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            fingerprint(b)
                .ok()
                .filter(|s| s == &where_.block_sha256)
                .map(|_| i)
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(
        found.len() == 1,
        fail("Reference block changed or became ambiguous")
    );
    Ok(found[0])
}
#[derive(Clone, Debug, Serialize)]
pub struct Preview {
    pub link: ConfirmedLink,
    pub before_fingerprint: String,
    pub target_text: String,
    pub token: String,
    pub note: String,
    #[serde(skip)]
    target_before: Page,
}
fn preview_token(link: &ConfirmedLink, before: &str, target: &Page) -> Result<String> {
    fingerprint(&(1, link, before, fingerprint(target)?))
}
pub fn preview(
    project: &Project,
    expected: &Page,
    request: &Request,
    c: &Control,
) -> Result<Preview> {
    let before = fingerprint(expected)?;
    anyhow::ensure!(
        fingerprint(&project.load(expected.page)?)? == before
            && project.manifest.pages.contains(&request.target_page),
        fail("Reference page changed or target is inactive")
    );
    let target = if request.target_page == expected.page {
        expected.clone()
    } else {
        project.load(request.target_page)?
    };
    let anchor = location(project, expected, request.anchor_block, c)?;
    let destination = location(project, &target, request.target_block, c)?;
    anyhow::ensure!(
        anchor.page != destination.page || anchor.block_hint != destination.block_hint,
        fail("A reference cannot target its own paragraph")
    );
    let text = &expected.content.blocks[request.anchor_block].text;
    let marker = text
        .get(request.marker_start..request.marker_end)
        .ok_or_else(|| fail("Select exact UTF-8 marker boundaries inside the source paragraph"))?
        .to_owned();
    let mut link = ConfirmedLink {
        schema: 1,
        id: String::new(),
        kind: request.kind,
        anchor,
        target: destination,
        marker_start: request.marker_start,
        marker_end: request.marker_end,
        marker,
        bibliography: request.bibliography.clone(),
    };
    link.id = link_id(&link)?;
    validate_metadata(std::slice::from_ref(&link))?;
    anyhow::ensure!(
        expected.confirmed_references.len() < 256
            && expected
                .confirmed_references
                .iter()
                .all(|old| old.anchor.block_sha256 != link.anchor.block_sha256
                    || old.marker_end <= link.marker_start
                    || old.marker_start >= link.marker_end),
        fail(
            "This marker already has an overlapping confirmed relationship; remove it explicitly first"
        )
    );
    let target_text = target.content.blocks[request.target_block].text.clone();
    anyhow::ensure!(
        !target_text.trim().is_empty(),
        fail("Confirm a nonempty footnote/bibliography body")
    );
    let token = preview_token(&link, &before, &target)?;
    Ok(Preview{link,before_fingerprint:before,target_text,token,note:"Preview only. Confirm the exact source marker and note/bibliography body. Bibliography fields are explicitly supplied by you, not inferred from recognition. No matching text is linked automatically. Stale/ambiguous relationships export as original plain text with a review warning; recognized text is never rewritten by confirmation.".into(),target_before:target})
}
pub fn confirm(
    project: &Project,
    expected: &Page,
    preview: &Preview,
    token: &str,
    c: &Control,
) -> Result<u64> {
    anyhow::ensure!(
        fingerprint(expected)? == preview.before_fingerprint
            && token == preview.token
            && token
                == preview_token(
                    &preview.link,
                    &preview.before_fingerprint,
                    &preview.target_before
                )?,
        fail("Confirm the exact fresh reference preview")
    );
    let link = &preview.link;
    validate_metadata(std::slice::from_ref(link))?;
    anyhow::ensure!(
        source(project, expected, c)? == link.anchor.source
            && source(project, &preview.target_before, c)? == link.target.source
            && fingerprint(&project.load(link.target.page)?)?
                == fingerprint(&preview.target_before)?,
        fail("Reference target/source changed after preview")
    );
    let mut after = expected.clone();
    after.confirmed_references.push(link.clone());
    after.reviewed = false;
    let mut pages = vec![(expected.clone(), after)];
    if link.target.page != expected.page {
        pages.push((preview.target_before.clone(), preview.target_before.clone()));
    }
    c.check()?;
    crate::review::commit_pages(project, &pages, "Confirm footnote/citation relationship")
}
pub fn remove(project: &Project, expected: &Page, id: &str, c: &Control) -> Result<u64> {
    let mut after = expected.clone();
    after.confirmed_references.retain(|link| link.id != id);
    anyhow::ensure!(
        after.confirmed_references.len() + 1 == expected.confirmed_references.len(),
        fail("Choose one existing confirmed relationship")
    );
    after.reviewed = false;
    c.check()?;
    crate::review::commit(
        project,
        expected,
        &after,
        "Remove confirmed reference relationship",
    )
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedLink {
    pub link: ConfirmedLink,
    pub anchor_block: usize,
    pub target_block: usize,
    pub target_text: String,
    pub target_notes: Vec<String>,
}
impl ResolvedLink {
    pub fn entry_id(&self) -> Result<String> {
        fingerprint(&(&self.link.target, &self.link.bibliography))
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Resolution {
    pub links: Vec<ResolvedLink>,
    pub warnings: Vec<String>,
}
impl Resolution {
    pub fn is_empty(&self) -> bool {
        self.links.is_empty() && self.warnings.is_empty()
    }
    pub fn verify_unchanged(&self, project: &Project, c: &Control) -> Result<()> {
        anyhow::ensure!(
            fingerprint(self)? == fingerprint(&resolve(project, c)?)?,
            fail(
                "Confirmed reference sources/bodies changed during export; no output was published"
            )
        );
        Ok(())
    }
    pub fn anchors(&self, page: usize, block: usize) -> Vec<&ResolvedLink> {
        let mut out = self
            .links
            .iter()
            .filter(|r| r.link.anchor.page == page && r.anchor_block == block)
            .collect::<Vec<_>>();
        out.sort_by_key(|r| r.link.marker_start);
        out
    }
    pub fn relocated(&self, page: usize, block: usize) -> bool {
        self.links
            .iter()
            .any(|r| r.link.target.page == page && r.target_block == block)
            && self.anchors(page, block).is_empty()
    }
}
fn cache_page(
    project: &Project,
    n: usize,
    pages: &mut BTreeMap<usize, Page>,
    bytes: &mut usize,
) -> Result<()> {
    if let std::collections::btree_map::Entry::Vacant(entry) = pages.entry(n) {
        let page = project.load(n)?;
        *bytes = bytes.saturating_add(serde_json::to_vec(&page)?.len());
        anyhow::ensure!(
            *bytes <= 64_000_000,
            fail("Confirmed-reference page cache exceeds 64 MB; export a smaller project revision")
        );
        entry.insert(page);
    }
    Ok(())
}
pub fn resolve(project: &Project, c: &Control) -> Result<Resolution> {
    let mut result = Resolution::default();
    let mut all = vec![];
    let mut pages = BTreeMap::new();
    let mut bytes = 0usize;
    for n in &project.manifest.pages {
        c.check()?;
        let page = project.load(*n)?;
        if !page.confirmed_references.is_empty() {
            all.extend(page.confirmed_references.clone());
            bytes = bytes.saturating_add(serde_json::to_vec(&page)?.len());
            anyhow::ensure!(
                all.len() <= 4096 && bytes <= 64_000_000,
                fail("Confirmed-reference export exceeds 4,096 links or 64 MB")
            );
            pages.insert(*n, page);
        }
    }
    let mut identities = BTreeMap::new();
    let mut text_bytes = 0usize;
    for link in all {
        c.check()?;
        let checked = (|| -> Result<ResolvedLink> {
            anyhow::ensure!(
                project.manifest.pages.contains(&link.target.page),
                fail("Reference target is inactive")
            );
            cache_page(project, link.target.page, &mut pages, &mut bytes)?;
            for location in [&link.anchor, &link.target] {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    identities.entry(location.page)
                {
                    entry.insert(source(project, &pages[&location.page], c)?);
                }
                anyhow::ensure!(
                    identities[&location.page] == location.source,
                    fail("Reference source changed")
                );
            }
            let anchor = &pages[&link.anchor.page];
            let target = &pages[&link.target.page];
            let ai = find_block(anchor, &link.anchor)?;
            let ti = find_block(target, &link.target)?;
            let a = &anchor.content.blocks[ai];
            let t = &target.content.blocks[ti];
            anyhow::ensure!(
                prose(a)
                    && prose(t)
                    && !t.text.trim().is_empty()
                    && visible(anchor, a)
                    && visible(target, t)
                    && a.text.get(link.marker_start..link.marker_end) == Some(link.marker.as_str()),
                fail("Reference marker/body is covered or changed")
            );
            anyhow::ensure!(
                ai != ti || anchor.page != target.page,
                fail("Reference targets itself")
            );
            let mut target_notes = vec![format!("Source page {}, block {}.", target.page, ti + 1)];
            if t.uncertain {
                target_notes.push(
                    "Uncertain source reading: compare this note/reference with the scan.".into(),
                );
            }
            target_notes.extend(
                crate::layers::report(target)?
                    .into_iter()
                    .filter(|layer| layer.assignment.block == ti)
                    .map(|layer| layer.caption()),
            );
            Ok(ResolvedLink {
                link: link.clone(),
                anchor_block: ai,
                target_block: ti,
                target_text: t.text.clone(),
                target_notes,
            })
        })();
        match checked{Ok(r)=>{text_bytes=text_bytes.saturating_add(r.target_text.len()).saturating_add(r.target_notes.iter().map(String::len).sum::<usize>());anyhow::ensure!(text_bytes<=8_000_000,fail("Confirmed-reference bodies exceed 8 MB"));result.links.push(r);},Err(_)=>result.warnings.push(format!("Confirmed reference on source page {} is stale, covered or ambiguous. Its original text remains plain; reconfirm it in Review.",link.anchor.page))}
    }
    // Overlapping imported/edited metadata never selects one target arbitrarily.
    let mut invalid = BTreeSet::new();
    for (i, a) in result.links.iter().enumerate() {
        for b in result.links.iter().skip(i + 1) {
            if a.link.anchor.page == b.link.anchor.page
                && a.anchor_block == b.anchor_block
                && a.link.marker_start < b.link.marker_end
                && b.link.marker_start < a.link.marker_end
            {
                invalid.insert(a.link.id.clone());
                invalid.insert(b.link.id.clone());
            }
            if a.link.kind == Kind::Citation
                && b.link.kind == Kind::Citation
                && a.link.target == b.link.target
                && a.link.bibliography != b.link.bibliography
            {
                invalid.insert(a.link.id.clone());
                invalid.insert(b.link.id.clone());
            }
        }
    }
    if !invalid.is_empty() {
        result.links.retain(|r| !invalid.contains(&r.link.id));
        result.warnings.push("Conflicting confirmed reference relationships remain plain text; choose and reconfirm one target/metadata set.".into());
    }
    c.check()?;
    Ok(result)
}
