use crate::types::*;
use regex::Regex;
use scraper::{Html, Selector};

pub fn html_table(text: &str) -> Option<Block> {
    let doc = Html::parse_fragment(text);
    let tr = Selector::parse("tr").ok()?;
    let cell = Selector::parse("th,td").ok()?;
    let mut rows: Vec<Vec<String>> = vec![];
    let mut used = vec![vec![false; 40]; 100];
    let mut merges = vec![];
    let mut headers = 0;
    let mut width = 0;
    for (r, row) in doc.select(&tr).take(100).enumerate() {
        while rows.len() <= r {
            rows.push(vec![String::new(); 40]);
        }
        let mut col = 0;
        let mut header = false;
        for c in row.select(&cell) {
            while col < 40 && used[r][col] {
                col += 1;
            }
            if col >= 40 {
                break;
            }
            let rs = c
                .value()
                .attr("rowspan")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1)
                .clamp(1, 100 - r);
            let cs = c
                .value()
                .attr("colspan")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1)
                .clamp(1, 40 - col);
            while rows.len() < r + rs {
                rows.push(vec![String::new(); 40]);
            }
            rows[r][col] = c.text().collect::<Vec<_>>().join(" ").trim().into();
            header |= c.value().name() == "th";
            for line in used.iter_mut().skip(r).take(rs) {
                for v in line.iter_mut().skip(col).take(cs) {
                    *v = true;
                }
            }
            if rs > 1 || cs > 1 {
                merges.push(Merge {
                    row: r,
                    col,
                    rowspan: rs,
                    colspan: cs,
                });
            }
            width = width.max(col + cs);
            col += cs;
        }
        if header && r == headers {
            headers += 1;
        }
    }
    if rows.is_empty() || width == 0 {
        return None;
    }
    for r in &mut rows {
        r.truncate(width);
    }
    let b = Block {
        kind: "table".into(),
        rows,
        merges,
        header_rows: headers,
        ..Default::default()
    };
    if b.validate().is_ok() { Some(b) } else { None }
}
pub fn plain_blocks(text: &str) -> Vec<Block> {
    let table = Regex::new(r"(?is)<table\b.*?</table>").unwrap();
    let tags = Regex::new(r"<[^>]+>").unwrap();
    let mut out = vec![];
    let mut last = 0;
    for m in table.find_iter(text) {
        out.extend(markdown(&text[last..m.start()]));
        if let Some(t) = html_table(m.as_str()) {
            out.push(t);
        } else {
            // Live readers may wrap Markdown tables in HTML tags.
            out.extend(markdown(&tags.replace_all(m.as_str(), "")));
        }
        last = m.end();
    }
    out.extend(markdown(&text[last..]));
    out
}
fn markdown(text: &str) -> Vec<Block> {
    let mut out = vec![];
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let line = line.trim();
        if line.is_empty() || line == "```" || line == "```markdown" {
            continue;
        }
        if line.contains('|')
            && lines
                .peek()
                .is_some_and(|v| v.contains("---") && v.contains('|'))
        {
            let row = |l: &str| {
                l.trim()
                    .trim_matches('|')
                    .split('|')
                    .map(|v| v.trim().to_string())
                    .collect::<Vec<_>>()
            };
            let mut rows = vec![row(line)];
            lines.next();
            while lines.peek().is_some_and(|l| l.contains('|')) {
                rows.push(row(lines.next().unwrap()));
            }
            let cols = rows.iter().map(Vec::len).max().unwrap_or(1).min(40);
            for r in &mut rows {
                r.resize(cols, String::new());
                r.truncate(cols);
            }
            rows.truncate(100);
            out.push(Block {
                kind: "table".into(),
                rows,
                header_rows: 1,
                ..Default::default()
            });
        } else {
            let mut b = Block::text(line.trim_start_matches('#').trim());
            b.kind = if line.starts_with('#') {
                "heading"
            } else if line.starts_with("- ") || line.starts_with("* ") {
                "list"
            } else if line.starts_with("$$") {
                "equation"
            } else {
                "text"
            }
            .into();
            b.uncertain = line.to_lowercase().contains("[illegible]");
            out.push(b);
        }
    }
    out
}
pub fn lighton(text: &str) -> PageContent {
    let re = Regex::new(r"!\[([a-z_+]+)\]\((\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\)").unwrap();
    let hits = re.captures_iter(text).collect::<Vec<_>>();
    if hits.is_empty() {
        return PageContent{blocks:plain_blocks(text),warnings:vec!["OCR returned no source grounding; compare reading order and diagrams with the scan.".into()],summary:text.chars().take(2000).collect()};
    }
    let mut blocks = vec![];
    for (i, c) in hits.iter().enumerate() {
        let body = text[c.get(0).unwrap().end()
            ..hits
                .get(i + 1)
                .map_or(text.len(), |n| n.get(0).unwrap().start())]
            .trim();
        let coords = (1..=4)
            .map(|j| c[j + 1].parse::<f32>().unwrap_or(0.) / 1000.)
            .collect::<Vec<_>>();
        let bbox = BoxRect {
            left: coords[0].clamp(0., 1.),
            top: coords[1].clamp(0., 1.),
            right: coords[2].clamp(0., 1.),
            bottom: coords[3].clamp(0., 1.),
        };
        if !bbox.valid() {
            continue;
        }
        let mut b = if c[1] == *"table" || c[1] == *"chart" {
            html_table(body)
                .or_else(|| plain_blocks(body).into_iter().find(|b| b.kind == "table"))
                .unwrap_or_else(|| Block::text(body))
        } else {
            Block::text(body)
        };
        b.bbox = Some(bbox);
        match &c[1] {
            "title" | "section" => b.kind = "heading".into(),
            "image" => {
                b.kind = "figure".into();
                b.description = body.into();
                b.text.clear();
                b.uncertain = true;
            }
            "formula" => b.kind = "equation".into(),
            "list" => b.kind = "list".into(),
            _ => {}
        }
        blocks.push(b);
    }
    PageContent {
        blocks,
        summary: text.chars().take(2000).collect(),
        ..Default::default()
    }
}
pub fn structured(text: &str) -> Result<PageContent> {
    let text = text
        .trim()
        .trim_start_matches("```json")
        .trim_end_matches("```")
        .trim();
    let p: PageContent = serde_json::from_str(text)
        .map_err(|_| Fault::new("NG201", "Structured model output is not valid page JSON"))?;
    p.validate()?;
    anyhow::ensure!(
        !p.blocks.is_empty(),
        Fault::new("NG201", "Model returned empty page data")
    );
    Ok(p)
}

/// Keep useful reader content when optional region or merge claims are invalid.
/// Never guess a crop or table topology: preserve cells in their reported places
/// and require review when an entire table's merge claims must be omitted.
pub fn structured_reader(text: &str) -> Result<PageContent> {
    let text = text
        .trim()
        .trim_start_matches("```json")
        .trim_end_matches("```")
        .trim();
    let mut page: PageContent = serde_json::from_str(text)
        .map_err(|_| Fault::new("NG201", "Structured model output is not valid page JSON"))?;
    let mut repaired = 0;
    let mut unmerged = vec![];
    for (index, block) in page.blocks.iter_mut().enumerate() {
        let invalid = block.bbox.as_ref().is_some_and(|b| !b.valid());
        if invalid || (block.kind == "figure" && block.bbox.is_none()) {
            block.bbox = (block.kind == "figure").then(BoxRect::full);
            block.uncertain = true;
            repaired += 1;
        }
        if block.kind == "table" && !block.merges.is_empty() && block.validate().is_err() {
            // Do not retain an apparently valid subset: an in-bounds span can
            // still cover a distinct cell when the model's topology conflicts.
            // Recovery is allowed only if every other field validates unchanged.
            block.merges.clear();
            block.validate()?;
            block.uncertain = true;
            unmerged.push((index + 1).to_string());
        }
    }
    if repaired > 0 {
        page.warnings.push(format!("{repaired} source regions could not be located. Uncertain figures retain the full source page; verify their boundaries in Review."));
    }
    if !unmerged.is_empty() {
        page.warnings.push(format!("Table blocks {} contain invalid or overlapping merge metadata. All merge claims for those tables were omitted; every cell remains in its original row and column. Verify the unmerged layout against the scan and raw reader response in Review.", unmerged.join(", ")));
    }
    page.validate()?;
    anyhow::ensure!(
        !page.blocks.is_empty(),
        Fault::new("NG201", "Model returned empty page data")
    );
    Ok(page)
}
