//! Source-bound Word correction previews. No external relationships are fetched.
use crate::{
    storage::{Control, Project, fingerprint, hash_file},
    types::*,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
};
const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const NS: &str = "urn:notegen:roundtrip:1";
const XML_LIMIT: u64 = 32_000_000;
fn fault(s: &str) -> anyhow::Error {
    Fault::new("NG500", s).into()
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Shape {
    kind: String,
    bbox: Option<BoxRect>,
    rows: usize,
    cols: usize,
    merges: Vec<Merge>,
}
fn shape(b: &Block) -> Shape {
    Shape {
        kind: b.kind.clone(),
        bbox: b.bbox.clone(),
        rows: b.rows.len(),
        cols: b.rows.first().map_or(0, Vec::len),
        merges: b.merges.clone(),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BaselineField {
    tag: String,
    page: usize,
    block: usize,
    field: String,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    format: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Baseline {
    schema: u32,
    input_sha256: String,
    project_digest: String,
    pages: BTreeMap<usize, Vec<Shape>>,
    fields: Vec<BaselineField>,
    layout: Vec<String>,
}
pub(crate) fn baseline(p: &Project) -> Result<Baseline> {
    Ok(Baseline {
        schema: 1,
        input_sha256: p.manifest.input_sha256.clone(),
        project_digest: fingerprint(&p.manifest)?,
        pages: BTreeMap::new(),
        fields: vec![],
        layout: vec![],
    })
}
pub(crate) fn record_page(b: &mut Baseline, page: &Page) {
    b.pages
        .insert(page.page, page.content.blocks.iter().map(shape).collect());
}
pub(crate) fn clean(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| {
            matches!(*c, '\t' | '\n')
                || (*c >= ' ' && *c <= '\u{d7ff}')
                || (*c >= '\u{e000}' && *c <= '\u{fffd}')
                || *c >= '\u{10000}'
        })
        .collect()
}
pub(crate) fn tagged(
    b: &mut Baseline,
    page: usize,
    block: usize,
    field: &str,
    text: &str,
    content: &str,
) -> String {
    let tag = format!("ng:p{page}:b{block}:{field}");
    let id = b.fields.len() + 1;
    b.fields.push(BaselineField {
        tag: tag.clone(),
        page,
        block,
        field: field.into(),
        text: clean(text),
        format: None,
    });
    format!(
        "<w:sdt><w:sdtPr><w:alias w:val=\"NoteGen source page {page}, block {}\"/><w:tag w:val=\"{}\"/><w:id w:val=\"{id}\"/></w:sdtPr><w:sdtContent>{content}</w:sdtContent></w:sdt>",
        block + 1,
        crate::word::xml(&tag)
    )
}
pub(crate) fn tagged_equation(
    b: &mut Baseline,
    page: usize,
    block: usize,
    source: &str,
    content: &str,
) -> String {
    let result = tagged(b, page, block, "text", source, content);
    if let Some(field) = b.fields.last_mut() {
        field.format = Some("omml".into());
    }
    result
}
fn parse(xml: &str) -> Result<roxmltree::Document<'_>> {
    Ok(roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 1_000_000,
            entity_resolver: None,
        },
    )?)
}
fn named(n: roxmltree::Node<'_, '_>, name: &str) -> bool {
    n.tag_name().namespace() == Some(W) && n.tag_name().name() == name
}
fn tag<'a, 'input>(n: roxmltree::Node<'a, 'input>) -> Option<&'a str> {
    n.children()
        .find(|n| named(*n, "sdtPr"))?
        .children()
        .find(|n| named(*n, "tag"))?
        .attribute((W, "val"))
}
fn text(n: roxmltree::Node<'_, '_>) -> String {
    let paragraphs: Vec<_> = n.descendants().filter(|n| named(*n, "p")).collect();
    let mut result = vec![];
    for paragraph in paragraphs {
        let mut line = String::new();
        for node in paragraph.descendants() {
            if named(node, "t") {
                line.push_str(node.text().unwrap_or(""));
            } else if named(node, "tab") {
                line.push('\t');
            } else if named(node, "br") || named(node, "cr") {
                line.push('\n');
            }
        }
        result.push(line);
    }
    result.join("\n")
}
fn layout(doc: &roxmltree::Document<'_>) -> Vec<String> {
    let mut out = vec![];
    for n in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().namespace() == Some(W))
    {
        if n.ancestors().skip(1).any(|a| named(a, "sdt")) {
            continue;
        }
        match n.tag_name().name() {
            "sdt" => out.push(format!("control:{}", tag(n).unwrap_or("unknown"))),
            "p" => out.push(format!("paragraph:{}", text(n))),
            "tbl" | "tr" | "tc" => out.push(n.tag_name().name().into()),
            "gridSpan" | "vMerge" => out.push(format!(
                "{}:{}",
                n.tag_name().name(),
                n.attribute((W, "val")).unwrap_or("")
            )),
            _ => {}
        }
    }
    out
}
pub(crate) fn seal(b: &mut Baseline, document: &str) -> Result<()> {
    b.layout = layout(&parse(document)?);
    anyhow::ensure!(
        b.fields.len() <= 100000,
        fault("Word round-trip supports at most 100,000 source fields per export")
    );
    Ok(())
}
pub(crate) fn baseline_xml(b: &Baseline) -> Result<String> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(b)?);
    anyhow::ensure!(
        encoded.len() as u64 <= XML_LIMIT,
        fault("Word correction baseline is too large; export a smaller project")
    );
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ng:baseline xmlns:ng=\"{NS}\">{encoded}</ng:baseline>"
    ))
}
fn zip_text(zip: &mut zip::ZipArchive<fs::File>, name: &str) -> Result<String> {
    let mut file = zip.by_name(name)?;
    anyhow::ensure!(
        file.size() <= XML_LIMIT,
        fault("Word XML exceeds the import size limit")
    );
    let mut bytes = vec![];
    (&mut file).take(XML_LIMIT + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= XML_LIMIT,
        fault("Word XML exceeds the import size limit")
    );
    Ok(String::from_utf8(bytes)?)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportField {
    pub tag: String,
    pub page: usize,
    pub block: usize,
    pub field: String,
    pub baseline: String,
    pub current: String,
    pub imported: Option<String>,
    pub status: String,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preview {
    pub schema: u32,
    pub document_sha256: String,
    pub project_digest: String,
    pub fields: Vec<ImportField>,
    pub warnings: Vec<String>,
    pub page_digests: BTreeMap<usize, String>,
}
fn field_text<'a>(b: &'a Block, field: &str) -> Result<&'a str> {
    match field {
        "text" => Ok(&b.text),
        "description" => Ok(&b.description),
        "interpretation" => Ok(&b.interpretation),
        _ => {
            let parts: Vec<_> = field.split(':').collect();
            anyhow::ensure!(
                parts.len() == 3 && parts[0] == "cell",
                fault("Unknown Word correction field")
            );
            let row = parts[1].parse::<usize>()?;
            let col = parts[2].parse::<usize>()?;
            b.rows
                .get(row)
                .and_then(|r| r.get(col))
                .map(String::as_str)
                .ok_or_else(|| fault("Word cell no longer exists"))
        }
    }
}
fn set_field(b: &mut Block, field: &str, value: String) -> Result<()> {
    match field {
        "text" => b.text = value,
        "description" => b.description = value,
        "interpretation" => b.interpretation = value,
        _ => {
            let parts: Vec<_> = field.split(':').collect();
            anyhow::ensure!(
                parts.len() == 3 && parts[0] == "cell",
                fault("Unknown Word correction field")
            );
            let row = parts[1].parse::<usize>()?;
            let col = parts[2].parse::<usize>()?;
            *b.rows
                .get_mut(row)
                .and_then(|r| r.get_mut(col))
                .ok_or_else(|| fault("Word cell no longer exists"))? = value;
        }
    }
    Ok(())
}
pub fn preview(p: &Project, input: &Path, c: &Control) -> Result<Preview> {
    c.check()?;
    let file = fs::File::open(input)?;
    anyhow::ensure!(
        file.metadata()?.len() <= 512_000_000,
        fault("Word input exceeds 512 MB")
    );
    let mut zip = zip::ZipArchive::new(file)?;
    anyhow::ensure!(
        zip.len() <= 10000,
        fault("Word archive has too many entries")
    );
    let mut names = BTreeSet::new();
    let mut size = 0u64;
    for i in 0..zip.len() {
        c.check()?;
        let entry = zip.by_index(i)?;
        anyhow::ensure!(
            entry.enclosed_name().is_some()
                && !entry.name().contains('\\')
                && names.insert(entry.name().to_string()),
            fault("Unsafe or duplicate Word archive entries")
        );
        size = size
            .checked_add(entry.size())
            .ok_or_else(|| fault("Word archive size overflow"))?;
        anyhow::ensure!(
            size <= 512_000_000,
            fault("Expanded Word archive exceeds 512 MB")
        );
    }
    let xml = zip_text(&mut zip, "customXml/item1.xml")?;
    let data = parse(&xml)?;
    let root = data.root_element();
    anyhow::ensure!(
        root.tag_name().namespace() == Some(NS)
            && root.tag_name().name() == "baseline"
            && root.children().filter(|n| n.is_element()).count() == 0,
        fault("Unknown Word correction baseline")
    );
    let bytes = base64::engine::general_purpose::STANDARD.decode(root.text().unwrap_or(""))?;
    anyhow::ensure!(
        bytes.len() as u64 <= XML_LIMIT,
        fault("Word baseline exceeds its size limit")
    );
    let b: Baseline = serde_json::from_slice(&bytes)?;
    let digest = fingerprint(&p.manifest)?;
    anyhow::ensure!(
        b.schema == 1
            && b.project_digest == digest
            && b.input_sha256 == p.manifest.input_sha256
            && b.fields.len() <= 100000
            && b.pages.len() <= 3000,
        fault("Word document belongs to a different source, project or schema")
    );
    let document = zip_text(&mut zip, "word/document.xml")?;
    let doc = parse(&document)?;
    anyhow::ensure!(
        named(doc.root_element(), "document"),
        fault("Invalid Word document namespace")
    );
    let mut exported = BTreeSet::new();
    anyhow::ensure!(
        b.pages
            .iter()
            .all(|(n, shapes)| p.manifest.pages.contains(n) && shapes.len() <= 500),
        fault("Word baseline references invalid source pages")
    );
    for f in &b.fields {
        let saved = b
            .pages
            .get(&f.page)
            .and_then(|shapes| shapes.get(f.block))
            .ok_or_else(|| fault("Word baseline block is absent"))?;
        let allowed = if saved.kind == "table" {
            let parts: Vec<_> = f.field.split(':').collect();
            parts.len() == 3
                && parts[0] == "cell"
                && parts[1].parse::<usize>().is_ok_and(|r| r < saved.rows)
                && parts[2].parse::<usize>().is_ok_and(|c| c < saved.cols)
        } else if saved.kind == "figure" {
            ["description", "interpretation"].contains(&f.field.as_str())
        } else {
            f.field == "text"
        };
        anyhow::ensure!(
            p.manifest.pages.contains(&f.page)
                && f.tag == format!("ng:p{}:b{}:{}", f.page, f.block, f.field)
                && exported.insert(f.tag.clone())
                && f.text.len() <= 120000
                && allowed
                && f.format.as_deref().is_none_or(|format| format == "omml"
                    && saved.kind == "equation"
                    && f.field == "text"),
            fault("Invalid or duplicate Word baseline control")
        );
    }
    let formats: BTreeMap<_, _> = b
        .fields
        .iter()
        .map(|f| (f.tag.as_str(), f.format.as_deref()))
        .collect();
    let mut actual = BTreeMap::new();
    let mut invalid = BTreeSet::new();
    for node in doc.descendants().filter(|n| named(*n, "sdt")) {
        let key = tag(node).ok_or_else(|| fault("Word contains an untagged source control"))?;
        anyhow::ensure!(
            exported.contains(key) && !actual.contains_key(key),
            fault("Unknown or duplicate Word source control")
        );
        let content = node
            .children()
            .find(|n| named(*n, "sdtContent"))
            .ok_or_else(|| fault("Word source control has no content"))?;
        if content.descendants().any(|n| {
            [
                "sdt",
                "tbl",
                "drawing",
                "object",
                "instrText",
                "ins",
                "del",
                "moveFrom",
                "moveTo",
            ]
            .iter()
            .any(|name| named(n, name))
        }) {
            invalid.insert(key.to_string());
        }
        let format = formats.get(key).copied().flatten();
        let math: Vec<_> = content
            .descendants()
            .filter(|n| {
                n.tag_name().namespace()
                    == Some("http://schemas.openxmlformats.org/officeDocument/2006/math")
                    && n.tag_name().name() == "oMath"
            })
            .collect();
        let value = if format == Some("omml") {
            if math.len() == 1 {
                match crate::equations::from_omml(math[0]) {
                    Ok(value) => value,
                    Err(_) => {
                        invalid.insert(key.to_string());
                        "[unsupported edited Word equation]".into()
                    }
                }
            } else {
                invalid.insert(key.to_string());
                "[missing or repeated Word equation]".into()
            }
        } else {
            if !math.is_empty() {
                invalid.insert(key.to_string());
            }
            clean(&text(content))
        };
        anyhow::ensure!(
            value.len() <= 120000,
            fault("Word correction field is too large")
        );
        actual.insert(key.to_string(), value);
    }
    let layout_changed = layout(&doc) != b.layout;
    let mut out = Preview {
        schema: 1,
        document_sha256: hash_file(input, c)?,
        project_digest: digest,
        fields: vec![],
        warnings: vec![],
        page_digests: BTreeMap::new(),
    };
    if layout_changed {
        out.warnings.push("Word page/block/table structure or untracked content changed. Resolve structural changes before importing text corrections.".into());
    }
    let mut pages = BTreeMap::new();
    for n in b.pages.keys() {
        let page = p.load(*n)?;
        out.page_digests.insert(*n, fingerprint(&page)?);
        pages.insert(*n, page);
    }
    for field in &b.fields {
        c.check()?;
        let page = pages
            .get(&field.page)
            .ok_or_else(|| fault("Word baseline page is absent"))?;
        let block = page.content.blocks.get(field.block);
        let shape_changed = b.pages.get(&field.page).is_none_or(|shapes| {
            *shapes != page.content.blocks.iter().map(shape).collect::<Vec<_>>()
        });
        let current = block
            .and_then(|b| field_text(b, &field.field).ok())
            .map(clean)
            .unwrap_or_default();
        let imported = actual.get(&field.tag).cloned();
        let baseline_comparison = if field.format.as_deref() == Some("omml") {
            crate::equations::canonical(&field.text)?
        } else {
            field.text.clone()
        };
        let current_comparison = if field.format.as_deref() == Some("omml") {
            crate::equations::canonical(&current).unwrap_or_else(|_| current.clone())
        } else {
            current.clone()
        };
        let (status, message) = if layout_changed
            || shape_changed
            || invalid.contains(&field.tag)
            || imported.is_none()
        {
            (
                "structural_conflict",
                "Source controls, block geometry or Word structure changed. Review manually.",
            )
        } else if imported.as_ref() == Some(&baseline_comparison) {
            ("unchanged", "No Word correction to import.")
        } else if imported.as_ref() == Some(&current_comparison) {
            (
                "already_applied",
                "This correction is already present in the project.",
            )
        } else if current_comparison != baseline_comparison {
            (
                "conflict",
                "Both Word and the project changed this field. Compare against the source before resolving.",
            )
        } else {
            ("changed", "Ready for explicit selection and import.")
        };
        out.fields.push(ImportField {
            tag: field.tag.clone(),
            page: field.page,
            block: field.block,
            field: field.field.clone(),
            baseline: field.text.clone(),
            current,
            imported,
            status: status.into(),
            message: message.into(),
        });
    }
    Ok(out)
}
/// Apply only selected conflict-free changes from the exact preview, as one review revision.
pub fn apply(
    p: &Project,
    input: &Path,
    inspected: &Preview,
    selected: &[String],
    label: &str,
    c: &Control,
) -> Result<u64> {
    anyhow::ensure!(
        !selected.is_empty()
            && selected.len() <= 100000
            && selected.iter().collect::<BTreeSet<_>>().len() == selected.len(),
        fault("Select distinct Word corrections to import")
    );
    let fresh = preview(p, input, c)?;
    anyhow::ensure!(
        fingerprint(&fresh)? == fingerprint(inspected)?,
        fault("Word document or project changed after preview; preview again")
    );
    let mut pages: BTreeMap<usize, (Page, Page)> = BTreeMap::new();
    for key in selected {
        c.check()?;
        let f = fresh
            .fields
            .iter()
            .find(|f| &f.tag == key)
            .ok_or_else(|| fault("Unknown selected Word correction"))?;
        anyhow::ensure!(
            f.status == "changed",
            fault("Selected Word correction has a conflict or no change")
        );
        if let std::collections::btree_map::Entry::Vacant(entry) = pages.entry(f.page) {
            let before = p.load(f.page)?;
            entry.insert((before.clone(), before));
        }
        let (_, after) = pages
            .get_mut(&f.page)
            .ok_or_else(|| fault("Selected page is absent"))?;
        let block = after
            .content
            .blocks
            .get_mut(f.block)
            .ok_or_else(|| fault("Selected block is absent"))?;
        set_field(
            block,
            &f.field,
            f.imported
                .clone()
                .ok_or_else(|| fault("Selected field is absent"))?,
        )?;
        after.reviewed = false;
        after.edited = true;
    }
    c.check()?;
    crate::review::commit_pages(p, &pages.into_values().collect::<Vec<_>>(), label)
}
