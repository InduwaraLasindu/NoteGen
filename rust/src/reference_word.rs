//! Word semantic parts for explicitly resolved source relationships only.
use super::{paragraph, xml};
use crate::{
    references::{Kind, Resolution, ResolvedLink},
    storage::{Control, Project},
    types::*,
};
use std::collections::BTreeMap;

pub(super) struct ReferenceExport {
    pub(super) resolved: Resolution,
    notes: BTreeMap<String, (usize, ResolvedLink)>,
    sources: BTreeMap<String, ResolvedLink>,
}
fn entry_tag(link: &ResolvedLink) -> Result<String> {
    Ok(format!("NG{}", link.entry_id()?))
}
fn runs(text: &str) -> String {
    let mut out = String::new();
    for (i, line) in crate::roundtrip::clean(text).split('\n').enumerate() {
        if i > 0 {
            out.push_str("<w:r><w:br/></w:r>");
        }
        out.push_str(&format!(
            "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>",
            xml(line)
        ));
    }
    out
}
impl ReferenceExport {
    pub(super) fn new(project: &Project, c: &Control) -> Result<Self> {
        let resolved = if crate::redactions::is_export_projection(project) {
            Resolution::default()
        } else {
            crate::references::resolve(project, c)?
        };
        let mut notes = BTreeMap::new();
        let mut sources = BTreeMap::new();
        for link in &resolved.links {
            match link.link.kind {
                Kind::Footnote => {
                    let n = notes.len() + 1;
                    notes.insert(link.link.id.clone(), (n, link.clone()));
                }
                Kind::Citation => {
                    sources
                        .entry(entry_tag(link)?)
                        .or_insert_with(|| link.clone());
                }
            }
        }
        Ok(Self {
            resolved,
            notes,
            sources,
        })
    }
    pub(super) fn active(&self) -> bool {
        !self.resolved.links.is_empty()
    }
    pub(super) fn has_notes(&self) -> bool {
        !self.notes.is_empty()
    }
    pub(super) fn has_sources(&self) -> bool {
        !self.sources.is_empty()
    }
    pub(super) fn relocated(&self, page: usize, block: usize) -> bool {
        self.resolved.relocated(page, block)
    }
    pub(super) fn paragraph(
        &self,
        page: usize,
        block: usize,
        text: &str,
        style: &str,
    ) -> Result<Option<String>> {
        let links = self.resolved.anchors(page, block);
        if links.is_empty() {
            return Ok(None);
        }
        let mut xml_body = String::new();
        let mut cursor = 0;
        for link in links {
            xml_body.push_str(&runs(&text[cursor..link.link.marker_start]));
            match link.link.kind {
                Kind::Footnote => {
                    let id = self.notes[&link.link.id].0;
                    xml_body.push_str(&format!("<w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr><w:footnoteReference w:id=\"{id}\" w:customMarkFollows=\"1\"/><w:t xml:space=\"preserve\">{}</w:t></w:r>",xml(&link.link.marker)));
                }
                Kind::Citation => {
                    xml_body.push_str(&format!(
                        "<w:fldSimple w:instr=\"CITATION {} \\l 1033\">{}</w:fldSimple>",
                        entry_tag(link)?,
                        runs(&link.link.marker)
                    ));
                }
            }
            cursor = link.link.marker_end;
        }
        xml_body.push_str(&runs(&text[cursor..]));
        // Reuse the established paragraph properties, including RTL, while
        // replacing only its text runs with escaped, generated reference runs.
        let empty = paragraph(text, style);
        let start = empty.find("</w:pPr>").expect("paragraph properties") + "</w:pPr>".len();
        Ok(Some(format!("{}{xml_body}</w:p>", &empty[..start])))
    }
    pub(super) fn notes_xml(&self) -> Option<String> {
        if self.notes.is_empty() {
            return None;
        }
        let mut out = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:footnotes xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:footnote w:type=\"separator\" w:id=\"-1\"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>",
        );
        let mut notes = self.notes.values().collect::<Vec<_>>();
        notes.sort_by_key(|(id, _)| *id);
        for (id, link) in notes {
            out.push_str(&format!(
                "<w:footnote w:id=\"{id}\">{}{} </w:footnote>",
                paragraph(
                    &format!("{} {}", link.link.marker, link.target_text),
                    "Normal"
                ),
                link.target_notes
                    .iter()
                    .map(|note| paragraph(note, "Caption"))
                    .collect::<String>()
            ));
        }
        out.push_str("</w:footnotes>");
        Some(out)
    }
    pub(super) fn sources_xml(&self) -> Option<String> {
        if self.sources.is_empty() {
            return None;
        }
        let mut out = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><b:Sources xmlns:b=\"http://schemas.openxmlformats.org/officeDocument/2006/bibliography\">",
        );
        for (tag, link) in &self.sources {
            let b = link
                .link
                .bibliography
                .as_ref()
                .expect("resolved citation metadata");
            out.push_str(&format!("<b:Source><b:Tag>{tag}</b:Tag><b:SourceType>Misc</b:SourceType><b:Title>{}</b:Title>",xml(&b.title)));
            if !b.author.is_empty() {
                out.push_str(&format!(
                    "<b:Author><b:Author><b:Corporate>{}</b:Corporate></b:Author></b:Author>",
                    xml(&b.author)
                ));
            }
            if !b.year.is_empty() {
                out.push_str(&format!("<b:Year>{}</b:Year>", xml(&b.year)));
            }
            out.push_str(&format!(
                "<b:Comments>{}</b:Comments></b:Source>",
                xml(&link.target_text)
            ));
        }
        out.push_str("</b:Sources>");
        Some(out)
    }
    pub(super) fn bibliography(&self) -> Option<String> {
        if self.sources.is_empty() {
            return None;
        }
        let mut out = paragraph("Confirmed bibliography", "Heading1");
        out.push_str("<w:p><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> BIBLIOGRAPHY </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r></w:p>");
        for link in self.sources.values() {
            out.push_str(&paragraph(&link.target_text, "Reference"));
            for note in &link.target_notes {
                out.push_str(&paragraph(note, "Caption"));
            }
        }
        out.push_str("<w:p><w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p>");
        Some(out)
    }
}
