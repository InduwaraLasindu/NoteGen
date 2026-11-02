//! Local, review-aware exports. Source regions are never inferred from text layout.
use crate::{
    storage::{Control, Project},
    types::*,
    word::xml,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, io::Write, path::Path};
use zip::{ZipWriter, write::SimpleFileOptions};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Markdown,
    Html,
    Latex,
    Spreadsheet,
    Traceability,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Html => "html",
            Self::Latex => "tex",
            Self::Spreadsheet => "xlsx",
            Self::Traceability => "json",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Evidence {
    pub model_id: String,
    pub family: String,
    pub view: String,
    pub checkpoint_uri: String,
    pub json_pointer: String,
    /// An exact recorded value match; this is not independent accuracy evidence.
    pub relation: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CellTrace {
    pub id: String,
    pub row: usize,
    pub column: usize,
    pub value: String,
    pub source_uri: String,
    /// Individual cell boxes are unavailable: use the containing table region.
    pub localization: String,
    pub evidence: Vec<Evidence>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockTrace {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub source_uri: String,
    pub source_region: BoxRect,
    pub localization: String,
    pub reviewed: bool,
    pub uncertain: bool,
    pub confidence: Option<f32>,
    pub alternatives: Vec<String>,
    pub evidence: Vec<Evidence>,
    pub cells: Vec<CellTrace>,
    pub manual_layers: Vec<crate::layers::ResolvedLayer>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageTrace {
    pub page: usize,
    pub source_uri: String,
    pub reviewed: bool,
    pub requires_review: bool,
    pub warnings: Vec<String>,
    pub blocks: Vec<BlockTrace>,
    /// Includes stale assignments without attaching them to a different current block.
    pub manual_layers: Vec<crate::layers::ResolvedLayer>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Traceability {
    pub schema: u32,
    pub input_sha256: String,
    pub project_fingerprint: String,
    pub evidence_note: String,
    pub pages: Vec<PageTrace>,
}

fn file_uri(path: &Path) -> Result<String> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    reqwest::Url::from_file_path(absolute)
        .map(String::from)
        .map_err(|_| Fault::new("NG400", "Cannot construct a local source link").into())
}
fn source_uri(project: &Project, page: &Page, region: &BoxRect) -> Result<String> {
    let mut uri = file_uri(&project.root.canonicalize()?.join(&page.image))?;
    uri.push_str(&format!(
        "#xywh=percent,{:.4},{:.4},{:.4},{:.4}",
        region.left * 100.,
        region.top * 100.,
        (region.right - region.left) * 100.,
        (region.bottom - region.top) * 100.
    ));
    Ok(uri)
}
fn checkpoint_uri(project: &Project, page: &Page) -> Result<String> {
    let edits = project
        .root
        .join(format!("page-{:05}.edits.json", page.page));
    file_uri(&if edits.is_file() {
        edits
    } else {
        project.page_path(page.page)
    })
}
fn evidence_index(page: &Page, checkpoint: &str) -> Result<HashMap<String, Vec<Evidence>>> {
    let mut found: HashMap<String, Vec<Evidence>> = HashMap::new();
    for (ci, candidate) in page.candidates.iter().enumerate() {
        anyhow::ensure!(
            candidate.model_id.len() <= 100
                && candidate.family.len() <= 100
                && candidate.view.len() <= 100,
            Fault::new(
                "NG400",
                "Reader evidence metadata exceeds the export size limit"
            )
        );
        candidate.content.validate()?;
        for (bi, block) in candidate.content.blocks.iter().enumerate() {
            let base = format!("/candidates/{ci}/content/blocks/{bi}");
            let value = block.plain();
            if !value.is_empty() && found.len() < 100_000 {
                let matches = found.entry(value).or_default();
                if matches.len() < 4 {
                    matches.push(Evidence {
                        model_id: candidate.model_id.clone(),
                        family: candidate.family.clone(),
                        view: candidate.view.clone(),
                        checkpoint_uri: checkpoint.into(),
                        json_pointer: base.clone(),
                        relation: "exact_recorded_block_value".into(),
                    });
                }
            }
            for (r, row) in block.rows.iter().enumerate() {
                for (col, cell) in row.iter().enumerate() {
                    if !cell.is_empty() && found.len() < 100_000 {
                        let matches = found.entry(cell.clone()).or_default();
                        if matches.len() < 4 {
                            matches.push(Evidence {
                                model_id: candidate.model_id.clone(),
                                family: candidate.family.clone(),
                                view: candidate.view.clone(),
                                checkpoint_uri: checkpoint.into(),
                                json_pointer: format!("{base}/rows/{r}/{col}"),
                                relation: "exact_recorded_cell_value".into(),
                            });
                        }
                    }
                }
            }
        }
    }
    Ok(found)
}
pub fn traceability(project: &Project, c: &Control) -> Result<Traceability> {
    c.check()?;
    let mut result = Traceability {
        schema: 1,
        input_sha256: project.manifest.input_sha256.clone(),
        project_fingerprint: project.manifest.fingerprint.clone(),
        evidence_note: "Source coordinates refer to the saved scan, not an inferred cell grid. Exact reader-value matches are candidates, not proof of correctness; repeated values may match multiple regions. At most four exact matches per value and 100,000 distinct evidence values per page are retained. Missing evidence means no exact match was retained. Confidence is uncalibrated. Source links depend on the local project remaining available.".into(),
        pages: vec![],
    };
    let mut total_cells = 0usize;
    let mut total_blocks = 0usize;
    let mut total_layers = 0usize;
    for n in &project.manifest.pages {
        c.check()?;
        let page = project.load(*n)?;
        let checkpoint = checkpoint_uri(project, &page)?;
        let evidence = evidence_index(&page, &checkpoint)?;
        let layers = crate::layers::report(&page)?;
        total_layers += layers.len();
        anyhow::ensure!(
            total_layers <= 10_000,
            Fault::new(
                "NG400",
                "Traceability exceeds 10,000 manual layers; export fewer pages"
            )
        );
        let mut p = PageTrace {
            page: page.page,
            source_uri: source_uri(project, &page, &BoxRect::full())?,
            reviewed: page.reviewed,
            requires_review: page.needs_review(project.manifest.settings.review_threshold),
            warnings: page.source_warnings.clone(),
            blocks: vec![],
            manual_layers: layers.clone(),
        };
        p.warnings.extend(page.content.warnings.clone());
        for (bi, b) in page.content.blocks.iter().enumerate() {
            c.check()?;
            total_blocks += 1;
            total_cells += b.rows.iter().map(Vec::len).sum::<usize>();
            anyhow::ensure!(
                total_blocks <= 50_000 && total_cells <= 100_000,
                Fault::new(
                    "NG400",
                    "Traceability export exceeds its bounded block/cell budget; split the project into smaller exports"
                )
            );
            let id = format!("page-{}-block-{}", page.page, bi + 1);
            let region = b.bbox.clone().unwrap_or_else(BoxRect::full);
            let uri = source_uri(project, &page, &region)?;
            let mut block = BlockTrace {
                id,
                kind: b.kind.clone(),
                text: if b.kind == "figure" {
                    format!(
                        "Observed: {}\nInterpretation: {}",
                        b.description, b.interpretation
                    )
                } else {
                    b.plain()
                },
                source_uri: uri.clone(),
                source_region: region,
                localization: if b.bbox.is_some() {
                    "block_region"
                } else {
                    "page_fallback"
                }
                .into(),
                reviewed: page.reviewed,
                uncertain: b.uncertain
                    || b.confidence
                        .is_none_or(|v| v < project.manifest.settings.review_threshold),
                confidence: b.confidence,
                alternatives: b.alternatives.clone(),
                evidence: evidence.get(&b.plain()).cloned().unwrap_or_default(),
                cells: vec![],
                manual_layers: layers
                    .iter()
                    .filter(|a| a.confirmed() && a.assignment.block == bi)
                    .cloned()
                    .collect(),
            };
            for (r, row) in b.rows.iter().enumerate() {
                for (col, value) in row.iter().enumerate() {
                    block.cells.push(CellTrace {
                        id: format!("{}-r{}c{}", block.id, r + 1, col + 1),
                        row: r + 1,
                        column: col + 1,
                        value: value.clone(),
                        source_uri: uri.clone(),
                        localization: if b.bbox.is_some() {
                            "table_region"
                        } else {
                            "page_fallback"
                        }
                        .into(),
                        evidence: evidence.get(value).cloned().unwrap_or_default(),
                    });
                }
            }
            p.blocks.push(block);
        }
        result.pages.push(p);
    }
    Ok(result)
}

fn markdown(text: &str) -> String {
    let mut out = String::new();
    let mut prose_start = true;
    for ch in text.chars() {
        // A hyphen inside a source label is ordinary prose. Escape a leading
        // hyphen (including after indentation) so document data cannot form a list.
        if "\\`*_{}[]<>()#+.!|~".contains(ch) || ch == '-' && prose_start {
            out.push('\\');
        }
        match ch {
            '&' => out.push_str("&amp;"),
            '\n' => out.push_str("<br>"),
            '\r' => {}
            _ => out.push(ch),
        }
        if !matches!(ch, ' ' | '\t' | '\r') {
            prose_start = false;
        }
    }
    out
}
fn latex(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\\' => "\\textbackslash{}".into(),
            '{' => "\\{".into(),
            '}' => "\\}".into(),
            '$' => "\\$".into(),
            '&' => "\\&".into(),
            '#' => "\\#".into(),
            '%' => "\\%".into(),
            '_' => "\\_".into(),
            '~' => "\\textasciitilde{}".into(),
            '^' => "\\textasciicircum{}".into(),
            '\n' => "\\newline{} ".into(),
            '\r' => String::new(),
            _ => ch.to_string(),
        })
        .collect()
}
fn text_export(project: &Project, format: Format, out: &mut impl Write, c: &Control) -> Result<()> {
    match format {
        Format::Html => out.write_all(b"<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>NoteGen export</title><style>body{font:1rem/1.6 system-ui;max-width:80ch;margin:2rem auto;padding:1rem}table{border-collapse:collapse}th,td{border:1px solid #777;padding:.4rem;white-space:pre-wrap}.review{border-left:4px solid #a65b00;padding-left:1rem}pre{white-space:pre-wrap}</style></head><body><h1>NoteGen transcription</h1><p>Confidence is uncalibrated. Compare uncertain content with its source scan. Local source links require the project files.</p>\n")?,
        Format::Latex => out.write_all(b"% Compile with XeLaTeX or LuaLaTeX for Unicode text.\n\\documentclass{article}\n\\usepackage{fontspec}\n\\usepackage{hyperref}\n\\usepackage{longtable}\n\\begin{document}\n\\section*{NoteGen transcription}\nConfidence is uncalibrated. Compare uncertain content with the source scan. Local links require the project files.\n\n")?,
        _ => out.write_all(b"# NoteGen transcription\n\nConfidence is uncalibrated. Compare uncertain content with its source scan. Local links require the project files.\n\n")?,
    }
    for n in &project.manifest.pages {
        c.check()?;
        let page = project.load(*n)?;
        let state = if page.reviewed {
            "reviewed"
        } else {
            "not reviewed"
        };
        let mut warnings = page.source_warnings.clone();
        warnings.extend(page.content.warnings.clone());
        let layers = crate::layers::report(&page)?;
        warnings.extend(
            layers
                .iter()
                .filter(|a| !a.confirmed())
                .map(crate::layers::ResolvedLayer::caption),
        );
        if page.needs_review(project.manifest.settings.review_threshold) {
            warnings.insert(0, "REVIEW NEEDED: compare with the scan".into());
        }
        match format {
            Format::Html => writeln!(
                out,
                "<section id=\"page-{n}\"><h2>Source page {n} · {state}</h2>"
            )?,
            Format::Latex => writeln!(out, "\\section*{{Source page {n} -- {state}}}")?,
            _ => writeln!(out, "## Source page {n} · {state}\n")?,
        }
        for warning in warnings {
            match format {
                Format::Html => writeln!(out, "<p class=\"review\">{}</p>", xml(&warning))?,
                Format::Latex => writeln!(out, "\\textbf{{{}}}\\par", latex(&warning))?,
                _ => writeln!(out, "> {}\n", markdown(&warning))?,
            }
        }
        for (index, b) in page.content.blocks.iter().enumerate() {
            c.check()?;
            let uri = source_uri(
                project,
                &page,
                &b.bbox.clone().unwrap_or_else(BoxRect::full),
            )?;
            let status = format!(
                "page {n}, block {}{}{}",
                index + 1,
                if b.bbox.is_none() {
                    "; page-level location only: no measured source box"
                } else {
                    ""
                },
                if b.uncertain
                    || b.confidence
                        .is_none_or(|v| v < project.manifest.settings.review_threshold)
                {
                    "; uncertain"
                } else {
                    ""
                }
            );
            match format {
                Format::Html => {
                    writeln!(
                        out,
                        "<article id=\"page-{n}-block-{}\" dir=\"auto\"><p><a href=\"{}\">Source: {}</a></p>",
                        index + 1,
                        xml(&uri),
                        xml(&status)
                    )?;
                    for layer in layers
                        .iter()
                        .filter(|a| a.confirmed() && a.assignment.block == index)
                    {
                        writeln!(
                            out,
                            "<p class=\"manual-layer\">{}</p>",
                            xml(&layer.caption())
                        )?;
                    }
                    match b.kind.as_str() {
                        "table" => {
                            writeln!(
                                out,
                                "<table><caption>Source table; cell links identify the table region</caption>"
                            )?;
                            for (ri, row) in b.rows.iter().enumerate() {
                                write!(out, "<tr>")?;
                                for cell in row {
                                    let tag = if ri < b.header_rows { "th" } else { "td" };
                                    write!(
                                        out,
                                        "<{tag}><a href=\"{}\">{}</a></{tag}>",
                                        xml(&uri),
                                        xml(cell)
                                    )?;
                                }
                                writeln!(out, "</tr>")?;
                            }
                            writeln!(out, "</table>")?;
                        }
                        "heading" => writeln!(out, "<h3>{}</h3>", xml(&b.text))?,
                        "figure" => writeln!(
                            out,
                            "<p>Observed: {}</p><p>Contextual interpretation (verify against source): {}</p>",
                            xml(&b.description),
                            xml(&b.interpretation)
                        )?,
                        _ => writeln!(out, "<pre>{}</pre>", xml(&b.text))?,
                    }
                    writeln!(out, "</article>")?;
                }
                Format::Latex => {
                    // All document text is escaped, including equations. Never execute untrusted TeX.
                    writeln!(
                        out,
                        "\\noindent Source: \\href{{{}}}{{{}}}\\par",
                        latex(&uri),
                        latex(&status)
                    )?;
                    for layer in layers
                        .iter()
                        .filter(|a| a.confirmed() && a.assignment.block == index)
                    {
                        writeln!(out, "\\textit{{{}}}\\par", latex(&layer.caption()))?;
                    }
                    if b.kind == "table" {
                        writeln!(
                            out,
                            "\\begin{{longtable}}{{{}}}",
                            "l".repeat(b.rows[0].len())
                        )?;
                        for row in &b.rows {
                            writeln!(
                                out,
                                "{} \\\\",
                                row.iter().map(|v| latex(v)).collect::<Vec<_>>().join(" & ")
                            )?;
                        }
                        writeln!(out, "\\end{{longtable}}")?;
                    } else if b.kind == "figure" {
                        writeln!(
                            out,
                            "Observed: {}\\par Contextual interpretation (verify against source): {}\\par",
                            latex(&b.description),
                            latex(&b.interpretation)
                        )?;
                    } else if b.kind == "heading" {
                        writeln!(out, "\\subsection*{{{}}}", latex(&b.text))?;
                    } else {
                        writeln!(out, "{}\\par", latex(&b.text))?;
                    }
                }
                _ => {
                    writeln!(out, "[Source: {}](<{uri}>)\n", markdown(&status))?;
                    for layer in layers
                        .iter()
                        .filter(|a| a.confirmed() && a.assignment.block == index)
                    {
                        writeln!(out, "> {}\n", markdown(&layer.caption()))?;
                    }
                    if b.kind == "table" {
                        let columns = b.rows[0].len();
                        // A neutral header avoids falsely promoting a data row to a heading.
                        writeln!(
                            out,
                            "|{}|",
                            (1..=columns)
                                .map(|n| format!(" Column {n} "))
                                .collect::<Vec<_>>()
                                .join("|")
                        )?;
                        writeln!(out, "|{}|", vec![" --- "; columns].join("|"))?;
                        for row in &b.rows {
                            writeln!(
                                out,
                                "|{}|",
                                row.iter()
                                    .map(|v| format!(" {} ", markdown(v)))
                                    .collect::<Vec<_>>()
                                    .join("|")
                            )?;
                        }
                        writeln!(out)?;
                    } else if b.kind == "figure" {
                        writeln!(
                            out,
                            "Observed: {}\n\nContextual interpretation (verify against source): {}\n",
                            markdown(&b.description),
                            markdown(&b.interpretation)
                        )?;
                    } else {
                        writeln!(
                            out,
                            "{}{}\n",
                            if b.kind == "heading" { "### " } else { "" },
                            markdown(&b.text)
                        )?;
                    }
                }
            }
        }
        if format == Format::Html {
            writeln!(out, "</section>")?;
        }
    }
    if format == Format::Html {
        out.write_all(b"</body></html>\n")?;
    } else if format == Format::Latex {
        out.write_all(b"\\end{document}\n")?;
    }
    Ok(())
}

/// One-based Excel column name, supporting the full XLSX limit.
pub fn column_name(mut n: usize) -> Result<String> {
    anyhow::ensure!(
        (1..=16384).contains(&n),
        Fault::new("NG400", "Spreadsheet column outside A:XFD")
    );
    let mut out = vec![];
    while n > 0 {
        n -= 1;
        out.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    out.reverse();
    Ok(String::from_utf8(out)?)
}
fn inline_cell(reference: &str, value: &str, style: usize) -> Result<String> {
    anyhow::ensure!(
        value.encode_utf16().count() <= 32767,
        Fault::new(
            "NG400",
            "A table cell exceeds Excel's 32,767 UTF-16 unit limit; choose a text export"
        )
    );
    // SpreadsheetML reserves _xHHHH_ escape sequences inside strings. Protect
    // literal sequences and encode XML-forbidden controls without losing data.
    let mut escaped = String::new();
    for (index, ch) in value.char_indices() {
        let suffix = &value.as_bytes()[index..];
        if ch == '_'
            && suffix.len() >= 7
            && suffix[1] == b'x'
            && suffix[2..6].iter().all(u8::is_ascii_hexdigit)
            && suffix[6] == b'_'
        {
            escaped.push_str("_x005F_");
        } else if (ch < ' ' && !matches!(ch, '\t' | '\n' | '\r'))
            || matches!(ch, '\u{fffe}' | '\u{ffff}')
        {
            escaped.push_str(&format!("_x{:04X}_", ch as u32));
        } else {
            escaped.push(ch);
        }
    }
    Ok(format!(
        "<c r=\"{reference}\" t=\"inlineStr\" s=\"{style}\"><is><t xml:space=\"preserve\">{}</t></is></c>",
        xml(&escaped)
    ))
}
const SHEET_START: &str = r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheetViews><sheetView workbookViewId="0"/></sheetViews><cols><col min="1" max="40" width="24" customWidth="1"/></cols><sheetData>"#;
fn zip_text<W: Write + std::io::Seek>(
    zip: &mut ZipWriter<W>,
    name: &str,
    value: &str,
) -> Result<()> {
    zip.start_file(
        name,
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
    )?;
    zip.write_all(value.as_bytes())?;
    Ok(())
}
fn spreadsheet(project: &Project, file: &mut fs::File, c: &Control) -> Result<()> {
    let mut zip = ZipWriter::new(file);
    let mut sheets = vec![("Review".to_string(), String::new())];
    let mut review = String::from(SHEET_START);
    let headers = [
        "Page",
        "Table",
        "Cell",
        "Value",
        "Page reviewed",
        "Uncertain",
        "Warnings",
        "Source (table region)",
    ];
    review.push_str("<row r=\"1\">");
    for (i, h) in headers.iter().enumerate() {
        review.push_str(&inline_cell(&format!("{}1", column_name(i + 1)?), h, 1)?);
    }
    review.push_str("</row>");
    let mut review_links = String::new();
    let mut review_rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    let mut review_row = 1usize;
    for n in &project.manifest.pages {
        c.check()?;
        let page = project.load(*n)?;
        for (bi, block) in page.content.blocks.iter().enumerate() {
            if block.kind != "table" {
                continue;
            }
            c.check()?;
            anyhow::ensure!(
                sheets.len() < 16000,
                Fault::new("NG400", "Too many spreadsheet tables; split the project")
            );
            let id = sheets.len() + 1;
            let table_no = sheets.len();
            let uri = source_uri(
                project,
                &page,
                &block.bbox.clone().unwrap_or_else(BoxRect::full),
            )?;
            let uncertain = block.uncertain
                || block
                    .confidence
                    .is_none_or(|v| v < project.manifest.settings.review_threshold);
            let style = if uncertain {
                2
            } else if page.reviewed {
                3
            } else {
                0
            };
            let mut sheet = String::from(SHEET_START);
            let mut links = String::new();
            let mut rels = String::from(
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
            );
            rels.push_str(&format!(r#"<Relationship Id="source" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="{}" TargetMode="External"/></Relationships>"#, xml(&uri)));
            let warning = format!(
                "{}{}{}",
                if uncertain { "Uncertain content. " } else { "" },
                if page.reviewed {
                    "Page marked reviewed. "
                } else {
                    "Page not reviewed. "
                },
                "Source link identifies the table region, not an individual cell. All values are text; no formulas are executed."
            );
            for (ri, row) in block.rows.iter().enumerate() {
                c.check()?;
                sheet.push_str(&format!("<row r=\"{}\">", ri + 1));
                for (ci, value) in row.iter().enumerate() {
                    let address = format!("{}{}", column_name(ci + 1)?, ri + 1);
                    sheet.push_str(&inline_cell(
                        &address,
                        value,
                        if ri < block.header_rows && !uncertain {
                            1
                        } else {
                            style
                        },
                    )?);
                    links.push_str(&format!(
                        "<hyperlink ref=\"{address}\" r:id=\"source\" tooltip=\"{}\"/>",
                        xml(&warning)
                    ));
                    review_row += 1;
                    anyhow::ensure!(
                        review_row <= 1_048_576,
                        Fault::new(
                            "NG400",
                            "Spreadsheet review exceeds Excel's row limit; split the project"
                        )
                    );
                    review.push_str(&format!("<row r=\"{review_row}\">"));
                    let all_warnings = page
                        .source_warnings
                        .iter()
                        .chain(&page.content.warnings)
                        .cloned()
                        .chain([warning.clone()])
                        .collect::<Vec<_>>()
                        .join("\n");
                    for (col, v) in [
                        page.page.to_string(),
                        format!("{table_no} (source block {})", bi + 1),
                        address,
                        value.clone(),
                        page.reviewed.to_string(),
                        uncertain.to_string(),
                        all_warnings,
                        uri.clone(),
                    ]
                    .iter()
                    .enumerate()
                    {
                        review.push_str(&inline_cell(
                            &format!("{}{review_row}", column_name(col + 1)?),
                            v,
                            style,
                        )?);
                    }
                    review.push_str("</row>");
                    review_links.push_str(&format!(
                        "<hyperlink ref=\"H{review_row}\" r:id=\"link{review_row}\"/>"
                    ));
                    review_rels.push_str(&format!(r#"<Relationship Id="link{review_row}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="{}" TargetMode="External"/>"#, xml(&uri)));
                }
                sheet.push_str("</row>");
            }
            sheet.push_str("</sheetData>");
            if !block.merges.is_empty() {
                sheet.push_str(&format!("<mergeCells count=\"{}\">", block.merges.len()));
                for m in &block.merges {
                    sheet.push_str(&format!(
                        "<mergeCell ref=\"{}{}:{}{}\"/>",
                        column_name(m.col + 1)?,
                        m.row + 1,
                        column_name(m.col + m.colspan)?,
                        m.row + m.rowspan
                    ));
                }
                sheet.push_str("</mergeCells>");
            }
            sheet.push_str(&format!("<hyperlinks>{links}</hyperlinks></worksheet>"));
            zip_text(&mut zip, &format!("xl/worksheets/sheet{id}.xml"), &sheet)?;
            zip_text(
                &mut zip,
                &format!("xl/worksheets/_rels/sheet{id}.xml.rels"),
                &rels,
            )?;
            sheets.push((
                format!("Table {table_no}"),
                format!("Page {n}, block {}", bi + 1),
            ));
        }
    }
    // A separate visible sheet preserves every manual label, including stale and non-table ones.
    let annotation_id = sheets.len() + 1;
    let mut annotation_sheet = String::from(SHEET_START);
    annotation_sheet.push_str("<row r=\"1\">");
    for (col, header) in [
        "Page",
        "Assigned block",
        "Layer",
        "Status",
        "Label",
        "Note",
        "Full block fingerprint",
        "Review note",
        "Source",
    ]
    .iter()
    .enumerate()
    {
        annotation_sheet.push_str(&inline_cell(
            &format!("{}1", column_name(col + 1)?),
            header,
            1,
        )?);
    }
    annotation_sheet.push_str("</row>");
    let mut annotation_row = 1usize;
    for n in &project.manifest.pages {
        c.check()?;
        let page = project.load(*n)?;
        for layer in crate::layers::report(&page)? {
            annotation_row += 1;
            anyhow::ensure!(
                annotation_row <= 1_048_576,
                Fault::new(
                    "NG400",
                    "Too many annotation layers for Excel; split the project"
                )
            );
            annotation_sheet.push_str(&format!("<row r=\"{annotation_row}\">"));
            let a = &layer.assignment;
            for (col, value) in [
                page.page.to_string(),
                (a.block + 1).to_string(),
                a.kind.name().into(),
                layer.status.clone(),
                a.label.clone(),
                a.note.clone(),
                a.block_fingerprint.clone(),
                layer.message.clone(),
                source_uri(
                    project,
                    &page,
                    &layer.source_region.clone().unwrap_or_else(BoxRect::full),
                )?,
            ]
            .iter()
            .enumerate()
            {
                annotation_sheet.push_str(&inline_cell(
                    &format!("{}{annotation_row}", column_name(col + 1)?),
                    value,
                    if layer.confirmed() { 0 } else { 2 },
                )?);
            }
            annotation_sheet.push_str("</row>");
            anyhow::ensure!(
                annotation_row <= 10_001 && annotation_sheet.len() <= 16_000_000,
                Fault::new(
                    "NG400",
                    "Spreadsheet annotation metadata exceeds its bounded budget; export fewer pages"
                )
            );
        }
    }
    if annotation_row > 1 {
        annotation_sheet.push_str("</sheetData></worksheet>");
        zip_text(
            &mut zip,
            &format!("xl/worksheets/sheet{annotation_id}.xml"),
            &annotation_sheet,
        )?;
        sheets.push(("Annotations".into(), String::new()));
    }
    if review_row == 1 {
        review.push_str("<row r=\"2\">");
        review.push_str(&inline_cell(
            "A2",
            "No recognized tables in this project.",
            0,
        )?);
        review.push_str("</row>");
    }
    review.push_str("</sheetData>");
    if !review_links.is_empty() {
        review.push_str(&format!("<hyperlinks>{review_links}</hyperlinks>"));
    }
    review.push_str("</worksheet>");
    review_rels.push_str("</Relationships>");
    zip_text(&mut zip, "xl/worksheets/sheet1.xml", &review)?;
    zip_text(
        &mut zip,
        "xl/worksheets/_rels/sheet1.xml.rels",
        &review_rels,
    )?;
    let mut workbook = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><bookViews><workbookView/></bookViews><sheets>"#,
    );
    let mut rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="styles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    );
    let mut content = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="json" ContentType="application/json"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#,
    );
    for (i, (name, _)) in sheets.iter().enumerate() {
        let id = i + 1;
        workbook.push_str(&format!(
            "<sheet name=\"{}\" sheetId=\"{id}\" r:id=\"sheet{id}\"/>",
            xml(name)
        ));
        rels.push_str(&format!(r#"<Relationship Id="sheet{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{id}.xml"/>"#));
        content.push_str(&format!(r#"<Override PartName="/xl/worksheets/sheet{id}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#));
    }
    workbook.push_str("</sheets></workbook>");
    rels.push_str("</Relationships>");
    content.push_str("</Types>");
    zip_text(&mut zip, "[Content_Types].xml", &content)?;
    zip_text(&mut zip, "xl/workbook.xml", &workbook)?;
    zip_text(&mut zip, "xl/_rels/workbook.xml.rels", &rels)?;
    zip_text(
        &mut zip,
        "_rels/.rels",
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="workbook" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
    )?;
    zip_text(
        &mut zip,
        "xl/styles.xml",
        r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="5"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFDCE8F7"/></patternFill></fill><fill><patternFill patternType="solid"><fgColor rgb="FFFFE3B0"/></patternFill></fill><fill><patternFill patternType="solid"><fgColor rgb="FFD9F0DD"/></patternFill></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="4"><xf numFmtId="49" fontId="0" fillId="0" borderId="0" xfId="0" applyAlignment="1"><alignment wrapText="1" vertical="top"/></xf><xf numFmtId="49" fontId="0" fillId="2" borderId="0" xfId="0" applyFill="1" applyAlignment="1"><alignment wrapText="1"/></xf><xf numFmtId="49" fontId="0" fillId="3" borderId="0" xfId="0" applyFill="1" applyAlignment="1"><alignment wrapText="1"/></xf><xf numFmtId="49" fontId="0" fillId="4" borderId="0" xfId="0" applyFill="1" applyAlignment="1"><alignment wrapText="1"/></xf></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#,
    )?;
    c.check()?;
    zip_text(
        &mut zip,
        "notegen/traceability.json",
        &serde_json::to_string_pretty(&traceability(project, c)?)?,
    )?;
    zip.finish()?;
    Ok(())
}

/// Export atomically, without replacing any existing output (including raced creations).
pub fn export(project: &Project, target: &Path, format: Format, c: &Control) -> Result<()> {
    let operation = || -> Result<()> {
        c.check()?;
        anyhow::ensure!(
            target
                .extension()
                .is_some_and(|v| v.eq_ignore_ascii_case(format.extension())),
            Fault::new("NG400", format!("Choose a .{} output", format.extension()))
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
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        match format {
            Format::Spreadsheet => spreadsheet(project, tmp.as_file_mut(), c)?,
            Format::Traceability => {
                serde_json::to_writer_pretty(&mut tmp, &traceability(project, c)?)?
            }
            _ => text_export(project, format, &mut tmp, c)?,
        }
        c.check()?;
        tmp.flush()?;
        tmp.as_file().sync_all()?;
        tmp.persist_noclobber(target)
            .map_err(|_| Fault::new("NG401", "Output collision or unavailable output folder"))?;
        Ok(())
    };
    operation().map_err(|e| {
        if e.downcast_ref::<Fault>().is_some() {
            e
        } else {
            Fault::new("NG400", format!("Could not export document: {e}")).into()
        }
    })
}
