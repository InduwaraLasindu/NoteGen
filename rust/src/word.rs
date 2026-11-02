use crate::{
    storage::{Control, Project},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};
use zip::{ZipWriter, write::SimpleFileOptions};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Style {
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub space_before: u32,
    pub space_after: u32,
    pub line_height: u32,
    pub color: [u8; 3],
    pub keep_with_next: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StyleProfile {
    pub schema: u32,
    pub styles: BTreeMap<String, Style>,
}
const STYLE_IDS: [&str; 7] = [
    "Normal",
    "Heading1",
    "ListParagraph",
    "Caption",
    "Equation",
    "Reference",
    "Code",
];
impl StyleProfile {
    pub fn defaults(s: &Settings) -> Self {
        let normal = Style {
            font: s.font.clone(),
            size: s.font_size,
            bold: false,
            italic: false,
            space_before: 0,
            space_after: 120,
            line_height: 240,
            color: [0, 0, 0],
            keep_with_next: false,
        };
        let mut styles = BTreeMap::new();
        for id in STYLE_IDS {
            let mut style = normal.clone();
            match id {
                "Heading1" => {
                    style.size = 15.0;
                    style.bold = true;
                    style.space_before = 240;
                    style.keep_with_next = true;
                }
                "Caption" => {
                    style.size = 9.0;
                    style.color = [107, 114, 128];
                }
                "Equation" => {
                    style.font = "Cambria Math".into();
                    style.space_before = 120;
                }
                "Reference" => {
                    style.size = 10.0;
                    style.space_after = 60;
                }
                "Code" => {
                    style.font = "Consolas".into();
                    style.size = 10.0;
                    style.space_before = 120;
                    style.space_after = 120;
                }
                "ListParagraph" => style.space_after = 60,
                _ => {}
            }
            styles.insert(id.into(), style);
        }
        Self { schema: 1, styles }
    }
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.schema == 1
                && self.styles.len() <= STYLE_IDS.len()
                && self.styles.keys().all(|k| STYLE_IDS.contains(&k.as_str())),
            Fault::new("NG102", "Unknown Word style schema or style name")
        );
        for s in self.styles.values() {
            anyhow::ensure!(
                !s.font.trim().is_empty()
                    && s.font.len() <= 100
                    && (6.0..=72.0).contains(&s.size)
                    && s.space_before <= 1440
                    && s.space_after <= 1440
                    && (120..=720).contains(&s.line_height),
                Fault::new("NG102", "Invalid Word font, size or spacing")
            );
        }
        Ok(())
    }
}
pub fn save_style_profile(project: &Project, profile: &StyleProfile) -> Result<()> {
    profile.validate()?;
    crate::storage::atomic_json(&project.root.join("word-styles.json"), profile)
}
pub fn style_profile(project: &Project) -> Result<StyleProfile> {
    let mut result = StyleProfile::defaults(&project.manifest.settings);
    let path = project.root.join("word-styles.json");
    if path.is_file() {
        let custom: StyleProfile = crate::storage::read_json(&path, 100_000)?;
        custom.validate()?;
        result.styles.extend(custom.styles);
    }
    Ok(result)
}
fn styles_xml(profile: &StyleProfile) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
    );
    for id in STYLE_IDS {
        let s = &profile.styles[id];
        let color = hex::encode_upper(s.color);
        out.push_str(&format!("<w:style w:type=\"paragraph\" {} w:styleId=\"{id}\"><w:name w:val=\"{id}\"/>{}<w:pPr><w:spacing w:before=\"{}\" w:after=\"{}\" w:line=\"{}\" w:lineRule=\"auto\"/>{}{}</w:pPr><w:rPr><w:rFonts w:ascii=\"{}\" w:hAnsi=\"{}\" w:cs=\"{}\"/><w:sz w:val=\"{}\"/><w:szCs w:val=\"{}\"/><w:color w:val=\"{color}\"/>{}{}</w:rPr></w:style>",if id=="Normal"{"w:default=\"1\""}else{""},if id=="Normal"{""}else{"<w:basedOn w:val=\"Normal\"/>"},s.space_before,s.space_after,s.line_height,if s.keep_with_next{"<w:keepNext/>"}else{""},if id=="ListParagraph"{"<w:ind w:left=\"360\" w:hanging=\"180\"/>"}else{""},xml(&s.font),xml(&s.font),xml(&s.font),(s.size*2.0)as u32,(s.size*2.0)as u32,if s.bold{"<w:b/>"}else{""},if s.italic{"<w:i/>"}else{""}));
    }
    out.push_str("</w:styles>");
    out
}
fn rtl(c: char) -> bool {
    matches!(c as u32,0x0590..=0x08ff|0xfb1d..=0xfdff|0xfe70..=0xfeff|0x10800..=0x10fff|0x1e800..=0x1eeff)
}

pub fn xml(text: &str) -> String {
    text.chars()
        .filter(|c| {
            matches!(*c, '\t' | '\n' | '\r')
                || (*c >= ' ' && *c <= '\u{d7ff}')
                || (*c >= '\u{e000}' && *c <= '\u{fffd}')
                || *c >= '\u{10000}'
        })
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&apos;".into(),
            _ => c.to_string(),
        })
        .collect()
}
pub fn paragraph(text: &str, style: &str) -> String {
    let clean = crate::roundtrip::clean(text);
    let direction = style != "Code" && clean.chars().find(|c| c.is_alphabetic()).is_some_and(rtl);
    let mut out = format!(
        "<w:p><w:pPr><w:pStyle w:val=\"{}\"/>{}</w:pPr>",
        xml(style),
        if direction { "<w:bidi/>" } else { "" }
    );
    for (i, line) in clean.split('\n').enumerate() {
        if i > 0 {
            out.push_str("<w:r><w:br/></w:r>");
        }
        out.push_str(&format!(
            "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>",
            xml(line)
        ));
    }
    out.push_str("</w:p>");
    out
}
fn table(
    b: &Block,
    page: usize,
    index: usize,
    baseline: &mut crate::roundtrip::Baseline,
) -> String {
    let mut o = String::from(
        "<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblBorders>",
    );
    for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
        o.push_str(&format!(
            "<w:{side} w:val=\"single\" w:sz=\"4\" w:color=\"8793A1\"/>"
        ));
    }
    o.push_str("</w:tblBorders>");
    if b.rows
        .iter()
        .flatten()
        .find_map(|s| s.chars().find(|c| c.is_alphabetic()))
        .is_some_and(rtl)
    {
        o.push_str("<w:bidiVisual/>");
    }
    o.push_str("</w:tblPr><w:tblGrid>");
    let cols = b.rows[0].len();
    for _ in 0..cols {
        o.push_str(&format!("<w:gridCol w:w=\"{}\"/>", 9300 / cols));
    }
    o.push_str("</w:tblGrid>");
    for (r, row) in b.rows.iter().enumerate() {
        o.push_str("<w:tr>");
        if r < b.header_rows {
            o.push_str("<w:trPr><w:tblHeader/></w:trPr>");
        }
        let mut c = 0;
        while c < cols {
            let merge = b.merges.iter().find(|m| {
                r >= m.row && r < m.row + m.rowspan && c >= m.col && c < m.col + m.colspan
            });
            if let Some(m) = merge
                && c != m.col
            {
                c += 1;
                continue;
            }
            o.push_str("<w:tc><w:tcPr>");
            let mut advance = 1;
            let fields = if let Some(m) = merge {
                advance = m.colspan;
                if m.colspan > 1 {
                    o.push_str(&format!("<w:gridSpan w:val=\"{}\"/>", m.colspan));
                }
                if m.rowspan > 1 {
                    o.push_str(if r == m.row {
                        "<w:vMerge w:val=\"restart\"/>"
                    } else {
                        "<w:vMerge/>"
                    });
                }
                if r == m.row {
                    (m.row..m.row + m.rowspan)
                        .flat_map(|rr| {
                            (m.col..m.col + m.colspan)
                                .map(move |cc| (rr, cc, b.rows[rr][cc].as_str()))
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![]
                }
            } else {
                vec![(r, c, row[c].as_str())]
            };
            o.push_str("</w:tcPr>");
            if fields.is_empty() {
                o.push_str(&paragraph("", "Normal"));
            }
            for (row, col, text) in fields {
                o.push_str(&crate::roundtrip::tagged(
                    baseline,
                    page,
                    index,
                    &format!("cell:{row}:{col}"),
                    text,
                    &paragraph(text, "Normal"),
                ));
            }
            o.push_str("</w:tc>");
            c += advance;
        }
        o.push_str("</w:tr>");
    }
    o.push_str("</w:tbl>");
    o
}
pub fn safe_image(root: &Path, name: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        !name.contains(['/', '\\', ':']) && name.ends_with(".png"),
        Fault::new("NG500", "Unsafe source image path")
    );
    let p = root.join(name).canonicalize()?;
    anyhow::ensure!(
        p.parent() == Some(root.canonicalize()?.as_path()),
        Fault::new("NG500", "Source image escaped project folder")
    );
    Ok(p)
}
pub fn export(project: &Project, target: &Path, c: &Control) -> Result<()> {
    export_inner(project, target, c).map_err(|e| {
        if e.downcast_ref::<Fault>().is_some() {
            e
        } else {
            Fault::new("NG400", format!("Could not save Word document: {e}")).into()
        }
    })
}
fn export_inner(project: &Project, target: &Path, c: &Control) -> Result<()> {
    anyhow::ensure!(
        target
            .extension()
            .is_some_and(|v| v.eq_ignore_ascii_case("docx")),
        Fault::new("NG400", "Choose a .docx output")
    );
    anyhow::ensure!(
        !target.exists(),
        Fault::new("NG401", "Output already exists")
    );
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let assets = tempfile::tempdir_in(parent)?;
    let document = assets.path().join("document.xml");
    let mut body = BufWriter::new(fs::File::create(&document)?);
    body.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body>"#)?;
    let mut media = vec![];
    let mut rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="styles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="notegenBaseline" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.xml"/>"#,
    );
    let mut baseline = crate::roundtrip::baseline(project)?;
    let profile = style_profile(project)?;
    for (index, n) in project.manifest.pages.iter().enumerate() {
        c.check()?;
        let page = project.load(*n)?;
        crate::roundtrip::record_page(&mut baseline, &page);
        if index > 0 && project.manifest.settings.keep_page_breaks {
            body.write_all(b"<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>")?;
        }
        body.write_all(
            paragraph(
                &format!(
                    "Source page {}{}",
                    page.page,
                    if page.reviewed { " · reviewed" } else { "" }
                ),
                "Caption",
            )
            .as_bytes(),
        )?;
        if page.needs_review(project.manifest.settings.review_threshold) {
            body.write_all(
                paragraph(
                    "REVIEW NEEDED — compare this content with the scan.",
                    "Caption",
                )
                .as_bytes(),
            )?;
        }
        for warning in &page.content.warnings {
            body.write_all(paragraph(warning, "Caption").as_bytes())?;
        }
        for (block_index, b) in page.content.blocks.iter().enumerate() {
            b.validate()?;
            match b.kind.as_str() {
                "table" => body.write_all(table(b, *n, block_index, &mut baseline).as_bytes())?,
                "figure" if project.manifest.settings.preserve_figures => {
                    let path = safe_image(&project.root, &page.image)?;
                    let im = crate::writers::open_image(&path, 40_000_000)?;
                    let (l, t, r, d) = b.bbox.as_ref().unwrap().pixels(im.width(), im.height());
                    let crop = im
                        .crop_imm(l, t, r - l, d - t)
                        .thumbnail(2000, 2400)
                        .to_rgb8();
                    let id = media.len() + 1;
                    let path = assets.path().join(format!("image{id}.jpg"));
                    let mut imagefile = fs::File::create(&path)?;
                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut imagefile, 95)
                        .encode_image(&crop)?;
                    let width = (crop.width() as f64 / 160.).min(6.45);
                    let cx = (width * 914400.) as u64;
                    let cy = (cx as f64 * crop.height() as f64 / crop.width() as f64) as u64;
                    rels.push_str(&format!(r#"<Relationship Id="image{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image{id}.jpg"/>"#));
                    body.write_all(format!(r#"<w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{id}" name="Figure {id}" descr="{}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="image{id}.jpg"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="image{id}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,xml(&b.description)).as_bytes())?;
                    body.write_all(paragraph("Observed:", "Caption").as_bytes())?;
                    body.write_all(
                        crate::roundtrip::tagged(
                            &mut baseline,
                            *n,
                            block_index,
                            "description",
                            &b.description,
                            &paragraph(&b.description, "Caption"),
                        )
                        .as_bytes(),
                    )?;
                    body.write_all(paragraph("Contextual interpretation:", "Caption").as_bytes())?;
                    body.write_all(
                        crate::roundtrip::tagged(
                            &mut baseline,
                            *n,
                            block_index,
                            "interpretation",
                            &b.interpretation,
                            &paragraph(&b.interpretation, "Normal"),
                        )
                        .as_bytes(),
                    )?;
                    body.write_all(
                        paragraph(
                            "AI diagram interpretations require source verification.",
                            "Caption",
                        )
                        .as_bytes(),
                    )?;
                    media.push(path);
                }
                "figure" => {}
                _ => {
                    let style = match b.kind.as_str() {
                        "heading" => "Heading1",
                        "list" => "ListParagraph",
                        "caption" => "Caption",
                        "equation" => "Equation",
                        "reference" => "Reference",
                        "code" => "Code",
                        _ => "Normal",
                    };
                    body.write_all(
                        crate::roundtrip::tagged(
                            &mut baseline,
                            *n,
                            block_index,
                            "text",
                            &b.text,
                            &paragraph(&b.text, style),
                        )
                        .as_bytes(),
                    )?;
                    if b.uncertain {
                        body.write_all(
                            paragraph("[uncertain] — verify this source region.", "Caption")
                                .as_bytes(),
                        )?;
                    }
                }
            }
        }
    }
    body.write_all(b"<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1080\" w:right=\"1080\" w:bottom=\"1080\" w:left=\"1080\"/></w:sectPr></w:body></w:document>")?;
    body.flush()?;
    drop(body);
    let document_text = fs::read_to_string(&document)?;
    crate::roundtrip::seal(&mut baseline, &document_text)?;
    let baseline_xml = crate::roundtrip::baseline_xml(&baseline)?;
    rels.push_str("</Relationships>");
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut zip = ZipWriter::new(tmp.as_file_mut());
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let styles = styles_xml(&profile);
        for (name, content) in [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="jpg" ContentType="image/jpeg"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/customXml/itemProps1.xml" ContentType="application/vnd.openxmlformats-officedocument.customXmlProperties+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="document" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            ),
            ("word/_rels/document.xml.rels", &rels),
            ("word/styles.xml", &styles),
            ("customXml/item1.xml", &baseline_xml),
            (
                "customXml/itemProps1.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><ds:datastoreItem ds:itemID="{7D87000D-A59F-4AF8-BE3C-9166A3DC0350}" xmlns:ds="http://schemas.openxmlformats.org/officeDocument/2006/customXml"><ds:schemaRefs><ds:schemaRef ds:uri="urn:notegen:roundtrip:1"/></ds:schemaRefs></ds:datastoreItem>"#,
            ),
            (
                "customXml/_rels/item1.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="properties" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXmlProps" Target="itemProps1.xml"/></Relationships>"#,
            ),
        ] {
            zip.start_file(name, opts)?;
            zip.write_all(content.as_bytes())?;
        }
        zip.start_file("word/document.xml", opts)?;
        std::io::copy(&mut fs::File::open(document)?, &mut zip)?;
        for (index, path) in media.iter().enumerate() {
            c.check()?;
            zip.start_file(format!("word/media/image{}.jpg", index + 1), opts)?;
            std::io::copy(&mut fs::File::open(path)?, &mut zip)?;
        }
        zip.finish()?;
    }
    tmp.as_file_mut().flush()?;
    tmp.as_file().sync_all()?;
    tmp.persist_noclobber(target)
        .map_err(|_| Fault::new("NG401", "Output collision or unavailable output folder"))?;
    Ok(())
}
