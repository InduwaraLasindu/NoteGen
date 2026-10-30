use crate::{
    storage::{Control, Project},
    types::*,
};
use std::{
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};
use zip::{ZipWriter, write::SimpleFileOptions};

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
    let mut out = format!("<w:p><w:pPr><w:pStyle w:val=\"{}\"/></w:pPr>", xml(style));
    for (i, line) in text.lines().enumerate() {
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
fn table(b: &Block) -> String {
    let mut o = String::from(
        "<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblBorders>",
    );
    for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
        o.push_str(&format!(
            "<w:{side} w:val=\"single\" w:sz=\"4\" w:color=\"8793A1\"/>"
        ));
    }
    o.push_str("</w:tblBorders></w:tblPr><w:tblGrid>");
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
            let text = if let Some(m) = merge {
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
                            (m.col..m.col + m.colspan).map(move |cc| b.rows[rr][cc].as_str())
                        })
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    String::new()
                }
            } else {
                row[c].clone()
            };
            o.push_str("</w:tcPr>");
            o.push_str(&paragraph(&text, "Normal"));
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
    export_inner(project,target,c).map_err(|e| if e.downcast_ref::<Fault>().is_some() {e} else {Fault::new("NG400",format!("Could not save Word document: {e}")).into()})
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
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="styles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    );
    for (index, n) in project.manifest.pages.iter().enumerate() {
        c.check()?;
        let page = project.load(*n)?;
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
        for b in &page.content.blocks {
            b.validate()?;
            match b.kind.as_str() {
                "table" => body.write_all(table(b).as_bytes())?,
                "figure" if project.manifest.settings.preserve_figures => {
                    let path = safe_image(&project.root, &page.image)?;
                    let im = image::open(path)?;
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
                    body.write_all(
                        paragraph(&format!("Observed: {}", b.description), "Normal").as_bytes(),
                    )?;
                    if !b.interpretation.is_empty() {
                        body.write_all(
                            paragraph(
                                &format!("Contextual interpretation: {}", b.interpretation),
                                "Normal",
                            )
                            .as_bytes(),
                        )?;
                    }
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
                    let style = if b.kind == "heading" {
                        "Heading1"
                    } else {
                        "Normal"
                    };
                    let text = format!(
                        "{}{}",
                        b.text,
                        if b.uncertain { " [uncertain]" } else { "" }
                    );
                    body.write_all(paragraph(&text, style).as_bytes())?;
                }
            }
        }
    }
    body.write_all(b"<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1080\" w:right=\"1080\" w:bottom=\"1080\" w:left=\"1080\"/></w:sectPr></w:body></w:document>")?;
    body.flush()?;
    drop(body);
    rels.push_str("</Relationships>");
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut zip = ZipWriter::new(tmp.as_file_mut());
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let s = &project.manifest.settings;
        let styles = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="{}" w:hAnsi="{}" w:cs="Nirmala UI"/><w:sz w:val="{}"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="30"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Caption"><w:name w:val="Caption"/><w:basedOn w:val="Normal"/><w:rPr><w:color w:val="6B7280"/><w:sz w:val="18"/></w:rPr></w:style></w:styles>"#,
            xml(&s.font),
            xml(&s.font),
            (s.font_size * 2.) as u32
        );
        for (name, content) in [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="jpg" ContentType="image/jpeg"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="document" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            ),
            ("word/_rels/document.xml.rels", &rels),
            ("word/styles.xml", &styles),
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
