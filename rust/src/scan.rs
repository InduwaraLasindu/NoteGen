use crate::{hardware::command, storage::Control, types::*};
use image::{DynamicImage, GrayImage, Luma};
use pdfium_render::prelude::*;
use rayon::prelude::*;
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

pub fn ocr_args(args: &[String], s: &Settings) -> Vec<String> {
    let mut args = args.to_vec();
    if !s.tessdata_dir.as_os_str().is_empty() {
        args.splice(
            2.min(args.len())..2.min(args.len()),
            [
                "--tessdata-dir".into(),
                s.tessdata_dir.to_string_lossy().into_owned(),
            ],
        );
    }
    args
}
pub fn run(
    exe: &Path,
    args: &[String],
    timeout: Duration,
    c: &Control,
) -> Result<(bool, String, String)> {
    run_with_threads(exe, args, timeout, c, 4)
}
pub fn run_with_threads(
    exe: &Path,
    args: &[String],
    timeout: Duration,
    c: &Control,
    threads: usize,
) -> Result<(bool, String, String)> {
    c.check()?;
    let mut out = tempfile::tempfile()?;
    let mut err = tempfile::tempfile()?;
    let mut child = command(exe)
        .args(args)
        .env("OMP_THREAD_LIMIT", threads.clamp(1, 512).to_string())
        .stdin(Stdio::null())
        .stdout(out.try_clone()?)
        .stderr(err.try_clone()?)
        .spawn()
        .map_err(|_| Fault::new("NG110", "Native executable unavailable"))?;
    let now = Instant::now();
    let status = loop {
        if c.cancelled.load(std::sync::atomic::Ordering::Relaxed) || now.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Fault::new(
                if c.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    "NG900"
                } else {
                    "NG110"
                },
                "Native operation cancelled or timed out",
            )
            .into());
        }
        if let Some(s) = child.try_wait()? {
            break s;
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    out.seek(SeekFrom::Start(0))?;
    err.seek(SeekFrom::Start(0))?;
    let mut a = String::new();
    let mut b = String::new();
    out.take(16_000_001).read_to_string(&mut a)?;
    err.take(16000).read_to_string(&mut b)?;
    anyhow::ensure!(
        a.len() <= 16_000_000,
        Fault::new("NG110", "OCR output exceeded size limit")
    );
    Ok((status.success(), a, b))
}
pub fn render_pdf(
    source: &Path,
    library: &Path,
    pages: &str,
    dpi: u32,
    max_pixels: u32,
    c: &Control,
    mut callback: impl FnMut(usize, DynamicImage) -> Result<()>,
) -> Result<Vec<usize>> {
    anyhow::ensure!(
        source.metadata()?.len() <= 2_000_000_000,
        Fault::new("NG100", "PDF exceeds 2 GB limit")
    );
    let engine = pdfium(library)?;
    let doc = engine
        .load_pdf_from_file(source, None)
        .map_err(|_| Fault::new("NG100", "Unreadable or encrypted PDF"))?;
    let total = doc.pages().len() as usize;
    anyhow::ensure!(
        (1..=3000).contains(&total),
        Fault::new("NG100", "PDF page count outside 1–3000")
    );
    let selected = pages_range(pages, total)?;
    for n in &selected {
        c.check()?;
        let p = doc.pages().get((n - 1) as i32)?;
        let w = p.width().value as f64;
        let h = p.height().value as f64;
        anyhow::ensure!(
            w.is_finite() && h.is_finite() && w > 0. && h > 0.,
            Fault::new("NG100", "Invalid page dimensions")
        );
        let scale = (dpi as f64 / 72.)
            .min((max_pixels as f64 / (w * h)).sqrt())
            .min(10000. / w.max(h))
            * 0.998;
        let width = (w * scale).max(1.) as i32;
        let height = (h * scale).max(1.) as i32;
        let im = p
            .render_with_config(
                &PdfRenderConfig::new()
                    .set_target_width(width)
                    .set_target_height(height),
            )?
            .as_image()?;
        callback(*n, im)?;
    }
    Ok(selected)
}
fn pdfium(library: &Path) -> Result<Pdfium> {
    let library = if library.is_file() {
        library.to_path_buf()
    } else {
        library.join(Pdfium::pdfium_platform_library_name())
    };
    let bindings = if library.is_file() {
        Pdfium::bind_to_library(&library)
    } else {
        Pdfium::bind_to_system_library()
    };
    Ok(match bindings {
        Ok(bindings) => Pdfium::new(bindings),
        Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Pdfium::default(),
        Err(e) => return Err(Fault::new("NG100", format!("PDFium load failed: {e}")).into()),
    })
}
/// Read selected page counts without rendering, OCR or loading AI weights.
pub fn selected_page_count(source: &Path, library: &Path, pages: &str) -> Result<usize> {
    anyhow::ensure!(
        source.metadata()?.len() <= 2_000_000_000,
        Fault::new("NG100", "PDF exceeds 2 GB limit")
    );
    let engine = pdfium(library)?;
    let doc = engine
        .load_pdf_from_file(source, None)
        .map_err(|_| Fault::new("NG100", "Unreadable or encrypted PDF"))?;
    let total = doc.pages().len() as usize;
    anyhow::ensure!(
        (1..=3000).contains(&total),
        Fault::new("NG100", "PDF page count outside 1–3000")
    );
    Ok(pages_range(pages, total)?.len())
}
pub fn enhance(im: &DynamicImage) -> GrayImage {
    let mut g = im.to_luma8();
    let mut hist = [0usize; 256];
    for p in g.as_raw() {
        hist[*p as usize] += 1;
    }
    let count = g.width() as usize * g.height() as usize;
    let low = hist
        .iter()
        .scan(0, |a, v| {
            *a += v;
            Some(*a)
        })
        .position(|v| v > count / 100)
        .unwrap_or(0) as u8;
    let high = hist
        .iter()
        .scan(0, |a, v| {
            *a += v;
            Some(*a)
        })
        .position(|v| v >= count - count / 100)
        .unwrap_or(255) as u8;
    if high.saturating_sub(low) > 10 {
        g.as_mut().par_iter_mut().for_each(|p| {
            *p = ((p.saturating_sub(low) as u32 * 255) / (high - low) as u32).min(255) as u8
        });
    }
    g
}
pub fn blur_warning(im: &DynamicImage) -> bool {
    let g = im.thumbnail(900, 1200).to_luma8();
    if g.width() < 3 || g.height() < 3 {
        return true;
    }
    let mut sum = 0.;
    let mut sum2 = 0.;
    let mut n = 0.;
    for y in 1..g.height() - 1 {
        for x in 1..g.width() - 1 {
            let v = 4. * g.get_pixel(x, y)[0] as f64
                - g.get_pixel(x - 1, y)[0] as f64
                - g.get_pixel(x + 1, y)[0] as f64
                - g.get_pixel(x, y - 1)[0] as f64
                - g.get_pixel(x, y + 1)[0] as f64;
            sum += v;
            sum2 += v * v;
            n += 1.;
        }
    }
    sum2 / n - (sum / n).powi(2) < 30.
}
#[derive(Clone, Debug)]
pub struct Word {
    pub text: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub confidence: f32,
    pub group: (u32, u32, u32),
}
pub fn parse_tsv(text: &str) -> Vec<Word> {
    let mut r = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .quoting(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    r.records()
        .filter_map(|v| {
            let r = v.ok()?;
            if r.get(0) != Some("5") {
                return None;
            }
            let text = r.get(11)?.trim().to_owned();
            if text.is_empty() {
                return None;
            }
            let confidence = r.get(10)?.parse::<f32>().ok()? / 100.;
            if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
                return None;
            }
            let word = Word {
                text,
                x: r.get(6)?.parse().ok()?,
                y: r.get(7)?.parse().ok()?,
                w: r.get(8)?.parse().ok()?,
                h: r.get(9)?.parse().ok()?,
                confidence: confidence.clamp(0., 1.),
                group: (
                    r.get(2)?.parse().ok()?,
                    r.get(3)?.parse().ok()?,
                    r.get(4)?.parse().ok()?,
                ),
            };
            (word.w > 0
                && word.h > 0
                && word.x.checked_add(word.w).is_some()
                && word.y.checked_add(word.h).is_some())
            .then_some(word)
        })
        .collect()
}
fn checked_words(text: &str, width: u32, height: u32) -> Result<Vec<Word>> {
    let mut native = 0usize;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .quoting(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let header = reader
        .headers()
        .map_err(|_| Fault::new("NG110", "Malformed Tesseract TSV header"))?;
    anyhow::ensure!(
        header.len() == 12
            && header.get(0) == Some("level")
            && header.get(6) == Some("left")
            && header.get(10) == Some("conf")
            && header.get(11) == Some("text"),
        Fault::new("NG110", "Unrecognized Tesseract TSV header")
    );
    for row in reader.records() {
        let row = row.map_err(|_| Fault::new("NG110", "Malformed Tesseract TSV"))?;
        if row.get(0) == Some("5") && row.get(11).is_some_and(|value| !value.trim().is_empty()) {
            anyhow::ensure!(
                row.len() == 12,
                Fault::new("NG110", "Tesseract TSV word has unexpected fields")
            );
            native += 1;
        }
        anyhow::ensure!(
            native <= 30_000,
            Fault::new("NG110", "Tesseract word count exceeds its geometry budget")
        );
    }
    let words = parse_tsv(text);
    anyhow::ensure!(
        words.len() == native
            && words
                .iter()
                .all(|word| word.x + word.w <= width && word.y + word.h <= height),
        Fault::new(
            "NG110",
            "Tesseract supplied invalid word coordinates or confidence; source retained for review"
        )
    );
    Ok(words)
}
fn word_region(word: &Word, width: u32, height: u32) -> Option<BoxRect> {
    let right = word.x.checked_add(word.w)?;
    let bottom = word.y.checked_add(word.h)?;
    if width == 0 || height == 0 || right > width || bottom > height || word.w == 0 || word.h == 0 {
        return None;
    }
    Some(BoxRect {
        left: word.x as f32 / width as f32,
        top: word.y as f32 / height as f32,
        right: right as f32 / width as f32,
        bottom: bottom as f32 / height as f32,
    })
}
fn observed_geometry(
    words: &[&Word],
    width: u32,
    height: u32,
    grid: Option<crate::geometry::TableGrid>,
) -> crate::geometry::BlockGeometry {
    use crate::geometry::{BlockGeometry, CellAddress, CellBox, LineRegion, Origin, Quad, WordBox};
    let mut groups = BTreeMap::<(u32, u32, u32), Vec<&Word>>::new();
    for word in words {
        if word_region(word, width, height).is_some() {
            groups.entry(word.group).or_default().push(word);
        }
    }
    let mut ordinals = BTreeMap::new();
    let mut lines = vec![];
    for (ordinal, (key, group)) in groups.iter().enumerate() {
        ordinals.insert(*key, ordinal);
        let left = group.iter().map(|word| word.x).min().unwrap();
        let top = group.iter().map(|word| word.y).min().unwrap();
        let right = group.iter().map(|word| word.x + word.w).max().unwrap();
        let bottom = group.iter().map(|word| word.y + word.h).max().unwrap();
        lines.push(LineRegion {
            ordinal,
            shape: Quad::rectangle(&BoxRect {
                left: left as f32 / width as f32,
                top: top as f32 / height as f32,
                right: right as f32 / width as f32,
                bottom: bottom as f32 / height as f32,
            }),
            origin: Origin::TesseractTsv,
        });
    }
    let mut result = BlockGeometry {
        lines,
        grid,
        ..Default::default()
    };
    if let Some(grid) = &result.grid {
        for row in 0..grid.row_edges.len() - 1 {
            for col in 0..grid.column_edges.len() - 1 {
                result.cells.push(CellBox {
                    row,
                    col,
                    region: grid.cell(row, col).expect("validated grid shape"),
                    origin: Origin::DetectedGrid,
                });
            }
        }
    }
    for word in words {
        let Some(region) = word_region(word, width, height) else {
            continue;
        };
        let cell = result.grid.as_ref().and_then(|grid| {
            let x = (region.left + region.right) / 2.;
            let y = (region.top + region.bottom) / 2.;
            Some(CellAddress {
                row: grid
                    .row_edges
                    .windows(2)
                    .position(|v| v[0] <= y && y <= v[1])?,
                col: grid
                    .column_edges
                    .windows(2)
                    .position(|v| v[0] <= x && x <= v[1])?,
            })
        });
        result.words.push(WordBox {
            text: word.text.clone(),
            region,
            confidence: Some(word.confidence),
            line: ordinals.get(&word.group).copied(),
            cell,
            origin: Origin::TesseractTsv,
        });
    }
    result
}
#[derive(Clone, Copy, Debug)]
struct Line {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    horizontal: bool,
}
fn lines(g: &GrayImage) -> Vec<Line> {
    let mut out = vec![];
    for (h, count, len, min) in [
        (true, g.height(), g.width(), (g.width() / 8).max(50)),
        (false, g.width(), g.height(), (g.height() / 30).max(40)),
    ] {
        for a in 0..count {
            let mut start = None;
            for b in 0..=len {
                let dark = b < len
                    && if h {
                        g.get_pixel(b, a)[0] < 110
                    } else {
                        g.get_pixel(a, b)[0] < 110
                    };
                if dark && start.is_none() {
                    start = Some(b);
                }
                if !dark
                    && let Some(first) = start.take()
                    && b - first >= min
                {
                    out.push(if h {
                        Line {
                            x0: first,
                            y0: a,
                            x1: b,
                            y1: a,
                            horizontal: true,
                        }
                    } else {
                        Line {
                            x0: a,
                            y0: first,
                            x1: a,
                            y1: b,
                            horizontal: false,
                        }
                    });
                    if out.len() > 2000 {
                        return out;
                    }
                }
            }
        }
    }
    out
}
fn close(a: Line, b: Line) -> bool {
    a.x0 <= b.x1 + 3 && b.x0 <= a.x1 + 3 && a.y0 <= b.y1 + 3 && b.y0 <= a.y1 + 3
}
fn positions(mut xs: Vec<u32>) -> Vec<u32> {
    xs.sort_unstable();
    let mut out = vec![];
    for x in xs {
        if out.last().is_none_or(|v| x > v + 4) {
            out.push(x);
        }
    }
    out
}
pub fn tables(g: &GrayImage, words: &[Word]) -> Vec<Block> {
    let ls = lines(g);
    let mut used = vec![false; ls.len()];
    let mut out = vec![];
    for i in 0..ls.len() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let mut component = vec![i];
        let mut pos = 0;
        while pos < component.len() {
            let a = ls[component[pos]];
            for j in 0..ls.len() {
                if !used[j] && close(a, ls[j]) {
                    used[j] = true;
                    component.push(j);
                }
            }
            pos += 1;
        }
        let xs = positions(
            component
                .iter()
                .filter(|i| !ls[**i].horizontal)
                .map(|i| ls[*i].x0)
                .collect(),
        );
        let ys = positions(
            component
                .iter()
                .filter(|i| ls[**i].horizontal)
                .map(|i| ls[*i].y0)
                .collect(),
        );
        if !(3..=41).contains(&xs.len()) || !(3..=101).contains(&ys.len()) {
            continue;
        }
        // Solid bars/filled graphics produce many long runs but are not a table grid.
        // Sample interior ink density without scanning a second page-sized buffer.
        let (left, right, top, bottom) = (xs[0], *xs.last().unwrap(), ys[0], *ys.last().unwrap());
        let mut ink = 0;
        for yy in 0..32 {
            for xx in 0..32 {
                let x = left + ((right - left) as u64 * (xx * 2 + 1) as u64 / 64) as u32;
                let y = top + ((bottom - top) as u64 * (yy * 2 + 1) as u64 / 64) as u32;
                if g.get_pixel(x, y)[0] < 120 {
                    ink += 1;
                }
            }
        }
        if ink > 700 {
            continue;
        }
        let mut rows = vec![vec![String::new(); xs.len() - 1]; ys.len() - 1];
        let mut confidences = vec![];
        let mut table_words = vec![];
        for word in words {
            let x = word.x + word.w / 2;
            let y = word.y + word.h / 2;
            if let (Some(col), Some(row)) = (
                xs.windows(2).position(|a| a[0] <= x && x <= a[1]),
                ys.windows(2).position(|a| a[0] <= y && y <= a[1]),
            ) {
                if !rows[row][col].is_empty() {
                    rows[row][col].push(' ');
                }
                rows[row][col].push_str(&word.text);
                confidences.push(word.confidence);
                table_words.push(word);
            }
        }
        out.push(Block {
            kind: "table".into(),
            rows,
            bbox: Some(BoxRect {
                left: xs[0] as f32 / g.width() as f32,
                top: ys[0] as f32 / g.height() as f32,
                right: (xs[xs.len() - 1] + 1).min(g.width()) as f32 / g.width() as f32,
                bottom: (ys[ys.len() - 1] + 1).min(g.height()) as f32 / g.height() as f32,
            }),
            confidence: Some(if confidences.is_empty() {
                0.
            } else {
                confidences.iter().sum::<f32>() / confidences.len() as f32
            }),
            geometry: Some(observed_geometry(
                &table_words,
                g.width(),
                g.height(),
                Some(crate::geometry::TableGrid {
                    row_edges: ys.iter().map(|y| *y as f32 / g.height() as f32).collect(),
                    column_edges: xs.iter().map(|x| *x as f32 / g.width() as f32).collect(),
                    origin: crate::geometry::Origin::DetectedGrid,
                }),
            )),
            ..Default::default()
        });
    }
    out
}
#[derive(Clone, Copy)]
pub enum PrintedLayout {
    Automatic,
    Paragraph,
    Line,
}
pub fn printed(im: &DynamicImage, s: &Settings, c: &Control) -> Result<PageContent> {
    printed_with_layout(im, s, c, PrintedLayout::Automatic)
}
pub fn printed_with_layout(
    im: &DynamicImage,
    s: &Settings,
    c: &Control,
    layout: PrintedLayout,
) -> Result<PageContent> {
    let dir = tempfile::tempdir()?;
    let within_budget_pool =
        rayon::current_thread_index().is_some() && rayon::current_num_threads() <= s.cpu_threads;
    let mut gray = if s.cpu_threads == 0 || within_budget_pool {
        enhance(im)
    } else {
        rayon::ThreadPoolBuilder::new()
            .num_threads(s.cpu_threads.clamp(1, 512))
            .build()?
            .install(|| enhance(im))
    };
    let original = gray.clone();
    for l in lines(&gray) {
        for y in l.y0.saturating_sub(2)..=(l.y1 + 2).min(gray.height() - 1) {
            for x in l.x0.saturating_sub(2)..=(l.x1 + 2).min(gray.width() - 1) {
                gray.put_pixel(x, y, Luma([255]));
            }
        }
    }
    let path = dir.path().join("ocr.png");
    gray.save(&path)?;
    if !s.tessdata_dir.as_os_str().is_empty() {
        crate::languages::verify_selection(&s.tessdata_dir, &s.languages, c)?;
    }
    let (ok, text, _) = run_with_threads(
        &s.tesseract,
        &ocr_args(
            &[
                path.to_string_lossy().into(),
                "stdout".into(),
                "-l".into(),
                s.languages.clone(),
                "--psm".into(),
                match layout {
                    PrintedLayout::Automatic => "3",
                    PrintedLayout::Paragraph => "6",
                    PrintedLayout::Line => "7",
                }
                .into(),
                "tsv".into(),
            ],
            s,
        ),
        Duration::from_secs(90),
        c,
        if s.cpu_threads == 0 {
            4
        } else {
            s.cpu_threads.min(4)
        },
    )?;
    anyhow::ensure!(
        ok,
        Fault::new(
            "NG110",
            "Tesseract failed; check executable and language data"
        )
    );
    let words = checked_words(&text, im.width(), im.height())?;
    let mut blocks = tables(&original, &words);
    let mut groups: BTreeMap<(u32, u32), Vec<&Word>> = BTreeMap::new();
    for w in &words {
        let x = (w.x + w.w / 2) as f32 / im.width() as f32;
        let y = (w.y + w.h / 2) as f32 / im.height() as f32;
        if blocks.iter().any(|b| {
            b.bbox
                .as_ref()
                .is_some_and(|b| b.left <= x && x <= b.right && b.top <= y && y <= b.bottom)
        }) {
            continue;
        }
        groups.entry((w.group.0, w.group.1)).or_default().push(w);
    }
    for group in groups.values() {
        let text = group
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let x = group.iter().map(|w| w.x).min().unwrap();
        let y = group.iter().map(|w| w.y).min().unwrap();
        let r = group.iter().map(|w| w.x + w.w).max().unwrap();
        let b = group.iter().map(|w| w.y + w.h).max().unwrap();
        blocks.push(Block {
            kind: "text".into(),
            text,
            bbox: Some(BoxRect {
                left: x as f32 / im.width() as f32,
                top: y as f32 / im.height() as f32,
                right: (r as f32 / im.width() as f32).min(1.),
                bottom: (b as f32 / im.height() as f32).min(1.),
            }),
            confidence: Some(group.iter().map(|w| w.confidence).sum::<f32>() / group.len() as f32),
            geometry: Some(observed_geometry(group, im.width(), im.height(), None)),
            ..Default::default()
        });
    }
    blocks.extend(graphics(&original, &words, &blocks));
    blocks.sort_by(|a, b| {
        a.bbox
            .as_ref()
            .unwrap()
            .top
            .total_cmp(&b.bbox.as_ref().unwrap().top)
    });
    let mut result = PageContent {
        blocks,
        ..Default::default()
    };
    if words.is_empty() {
        result
            .warnings
            .push("No printed text recognized; AI or manual transcription is required.".into());
    }
    result.summary = result.plain().chars().take(2000).collect();
    Ok(result)
}
pub struct OrientationResult {
    pub image: DynamicImage,
    pub detection: crate::language_routing::ScriptObservation,
}
/// Detect on the supplied (already masked, when applicable) raster. Never rotates.
pub fn detect_script(
    im: &DynamicImage,
    s: &Settings,
    c: &Control,
) -> Result<crate::language_routing::ScriptObservation> {
    detect_script_with_timeout(im, s, c, Duration::from_secs(20))
}
pub fn detect_script_with_timeout(
    im: &DynamicImage,
    s: &Settings,
    c: &Control,
    timeout: Duration,
) -> Result<crate::language_routing::ScriptObservation> {
    anyhow::ensure!(
        !timeout.is_zero() && timeout <= Duration::from_secs(20),
        Fault::new("NG110", "Invalid script detection time budget")
    );
    let d = tempfile::tempdir()?;
    let p = d.path().join("osd.png");
    im.thumbnail(1800, 1800).save(&p)?;
    let r = run_with_threads(
        &s.tesseract,
        &ocr_args(
            &[
                p.to_string_lossy().into(),
                "stdout".into(),
                "-l".into(),
                "osd".into(),
                "--psm".into(),
                "0".into(),
            ],
            s,
        ),
        timeout,
        c,
        if s.cpu_threads == 0 {
            4
        } else {
            s.cpu_threads.min(4)
        },
    );
    c.check()?;
    if let Ok((true, text, _)) = r {
        return Ok(crate::language_routing::parse_osd(&text)
            .unwrap_or_else(|_| crate::language_routing::ScriptObservation::unavailable()));
    }
    Ok(crate::language_routing::ScriptObservation::unavailable())
}
pub fn orient_with_detection(
    im: DynamicImage,
    s: &Settings,
    c: &Control,
) -> Result<OrientationResult> {
    let detection = detect_script(&im, s, c)?;
    let image = if s.auto_rotate {
        match detection.rotate_degrees {
            90 => im.rotate90(),
            180 => im.rotate180(),
            270 => im.rotate270(),
            _ => im,
        }
    } else {
        im
    };
    Ok(OrientationResult { image, detection })
}
pub fn orient(im: DynamicImage, s: &Settings, c: &Control) -> Result<DynamicImage> {
    if !s.auto_rotate {
        return Ok(im);
    }
    Ok(orient_with_detection(im, s, c)?.image)
}
pub fn crop_regions(
    content: &PageContent,
    im: &DynamicImage,
    s: &Settings,
) -> Vec<(BoxRect, DynamicImage)> {
    let mut boxes = content
        .blocks
        .iter()
        .filter(|b| {
            b.kind != "figure"
                && (s.handwriting
                    || b.uncertain
                    || b.confidence.is_none_or(|v| v < s.review_threshold)
                    || b.plain().contains("[illegible]"))
        })
        .filter_map(|b| b.bbox.clone())
        .take(s.crop_retries)
        .collect::<Vec<_>>();
    if boxes.is_empty() && s.preset == Preset::Maximum {
        let overlap = s.tile_overlap;
        for y in [0., 0.5 - overlap / 2.] {
            for x in [0., 0.5 - overlap / 2.] {
                boxes.push(BoxRect {
                    left: x,
                    top: y,
                    right: (x + 0.5 + overlap).min(1.),
                    bottom: (y + 0.5 + overlap).min(1.),
                });
            }
        }
    }
    boxes
        .into_iter()
        .take(s.crop_retries)
        .map(|b| {
            let (l, t, r, d) = b.pixels(im.width(), im.height());
            let crop = im.crop_imm(l, t, r - l, d - t).resize(
                s.tile_edge,
                s.tile_edge,
                image::imageops::FilterType::Lanczos3,
            );
            (b, crop)
        })
        .collect()
}
pub fn save_scan(path: &Path, im: &DynamicImage) -> Result<()> {
    let t = path.with_extension("tmp.png");
    im.save(&t)?;
    fs::rename(&t, path)?;
    Ok(())
}

pub fn deskew(im: DynamicImage, enabled: bool, max_pixels: u32) -> DynamicImage {
    if !enabled {
        return im;
    }
    let small = im.thumbnail(800, 1100).to_luma8();
    let mut ink = vec![];
    for y in (0..small.height()).step_by(2) {
        for x in (0..small.width()).step_by(2) {
            if small.get_pixel(x, y)[0] < 100 {
                ink.push((x as f32, y as f32));
            }
        }
    }
    if ink.len() < 50 {
        return im;
    }
    let stride = (ink.len() / 30000).max(1);
    let offset = small.width() as usize / 4 + 10;
    let score = |angle: f32| {
        let mut bins = vec![0u32; small.height() as usize + offset * 2 + 10];
        let tan = angle.to_radians().tan();
        for (x, y) in ink.iter().step_by(stride) {
            let b = (*y - *x * tan + offset as f32) as isize;
            if b >= 0 && (b as usize) < bins.len() {
                bins[b as usize] += 1;
            }
        }
        bins.iter().map(|v| (*v as f64).powi(2)).sum::<f64>()
    };
    let zero = score(0.);
    let mut best = (zero, 0.);
    for i in -10..=10 {
        let angle = i as f32 / 2.;
        let s = score(angle);
        if s > best.0 {
            best = (s, angle);
        }
    }
    if best.1.abs() < 0.4 || best.0 < zero * 1.06 {
        return im;
    }
    let angle = best.1.to_radians();
    let (co, si) = (angle.cos(), angle.sin());
    let w = (im.width() as f32 * co.abs() + im.height() as f32 * si.abs()).ceil() as u32;
    let h = (im.height() as f32 * co.abs() + im.width() as f32 * si.abs()).ceil() as u32;
    let src = im.to_rgb8();
    let mut out = image::RgbImage::from_pixel(w, h, image::Rgb([255, 255, 255]));
    out.as_mut()
        .par_chunks_mut(w as usize * 3)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w as usize {
                let dx = x as f32 - w as f32 / 2.;
                let dy = y as f32 - h as f32 / 2.;
                let sx = co * dx - si * dy + src.width() as f32 / 2.;
                let sy = si * dx + co * dy + src.height() as f32 / 2.;
                if sx >= 0.
                    && sy >= 0.
                    && sx < src.width().saturating_sub(1) as f32
                    && sy < src.height().saturating_sub(1) as f32
                {
                    let a = sx.floor() as u32;
                    let b = sy.floor() as u32;
                    let fx = sx - a as f32;
                    let fy = sy - b as f32;
                    for channel in 0..3 {
                        let p = src.get_pixel(a, b)[channel] as f32 * (1. - fx) * (1. - fy)
                            + src.get_pixel(a + 1, b)[channel] as f32 * fx * (1. - fy)
                            + src.get_pixel(a, b + 1)[channel] as f32 * (1. - fx) * fy
                            + src.get_pixel(a + 1, b + 1)[channel] as f32 * fx * fy;
                        row[x * 3 + channel] = p.round() as u8;
                    }
                }
            }
        });
    let out = DynamicImage::ImageRgb8(out);
    if w as u64 * h as u64 > max_pixels as u64 {
        let scale = (max_pixels as f64 / (w as f64 * h as f64)).sqrt();
        out.resize(
            (w as f64 * scale) as u32,
            (h as f64 * scale) as u32,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        out
    }
}

/// Conservative residual-ink grouping: preserve graphics that OCR cannot transcribe.
/// This detector labels visible regions only; contextual meaning comes from a vision reader.
pub fn graphics(source: &GrayImage, words: &[Word], blocks: &[Block]) -> Vec<Block> {
    let mut g = image::imageops::thumbnail(source, 1000, 1400);
    let sx = g.width() as f32 / source.width() as f32;
    let sy = g.height() as f32 / source.height() as f32;
    let mut erase = |x: u32, y: u32, r: u32, b: u32| {
        let (l, t, r, b) = (
            (x as f32 * sx).floor() as u32,
            (y as f32 * sy).floor() as u32,
            (r as f32 * sx).ceil() as u32,
            (b as f32 * sy).ceil() as u32,
        );
        for yy in t.saturating_sub(2)..(b + 3).min(g.height()) {
            for xx in l.saturating_sub(2)..(r + 3).min(g.width()) {
                g.put_pixel(xx, yy, Luma([255]));
            }
        }
    };
    for w in words {
        erase(w.x, w.y, w.x + w.w, w.y + w.h);
    }
    for b in blocks.iter().filter(|b| b.kind == "table") {
        if let Some(b) = &b.bbox {
            let (x, y, r, b) = b.pixels(source.width(), source.height());
            erase(x, y, r, b);
        }
    }
    let cell = 20u32;
    let cols = g.width().div_ceil(cell) as usize;
    let rows = g.height().div_ceil(cell) as usize;
    let mut active = vec![false; cols * rows];
    for y in 0..g.height() {
        for x in 0..g.width() {
            if g.get_pixel(x, y)[0] < 150 {
                active[y as usize / cell as usize * cols + x as usize / cell as usize] = true;
            }
        }
    }
    let mut out = vec![];
    for i in 0..active.len() {
        if !active[i] {
            continue;
        }
        active[i] = false;
        let mut stack = vec![i];
        let (mut l, mut r, mut t, mut b, mut count) = (cols, 0, rows, 0, 0);
        while let Some(j) = stack.pop() {
            let (x, y) = (j % cols, j / cols);
            l = l.min(x);
            r = r.max(x);
            t = t.min(y);
            b = b.max(y);
            count += 1;
            for yy in y.saturating_sub(1)..=(y + 1).min(rows - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(cols - 1) {
                    let n = yy * cols + xx;
                    if active[n] {
                        active[n] = false;
                        stack.push(n);
                    }
                }
            }
        }
        let area = (r - l + 1) * (b - t + 1);
        if count < 8 || area < cols * rows / 100 || r - l < 2 || b - t < 2 {
            continue;
        }
        out.push(Block {
            kind: "figure".into(),
            bbox: Some(BoxRect {
                left: l as f32 * cell as f32 / g.width() as f32,
                top: t as f32 * cell as f32 / g.height() as f32,
                right: ((r + 1) as f32 * cell as f32 / g.width() as f32).min(1.),
                bottom: ((b + 1) as f32 * cell as f32 / g.height() as f32).min(1.),
            }),
            description: "Visual region preserved from the scan. Verify its boundaries and labels."
                .into(),
            uncertain: true,
            ..Default::default()
        });
        if out.len() >= 32 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    const HEADER: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n";
    #[test]
    fn native_tsv_keeps_actual_word_line_geometry_and_rejects_corrupt_precision() {
        let text = format!(
            "{HEADER}5\t1\t2\t3\t4\t1\t10\t20\t30\t40\t92.5\t1250\n5\t1\t2\t3\t4\t2\t50\t20\t20\t40\t40\tword\n"
        );
        let words = checked_words(&text, 100, 100).unwrap();
        let refs = words.iter().collect::<Vec<_>>();
        let data = observed_geometry(&refs, 100, 100, None);
        assert_eq!(data.words.len(), 2);
        assert_eq!(data.lines.len(), 1);
        assert_eq!(data.words[0].text, "1250");
        assert_eq!(data.words[0].confidence, Some(0.925));
        assert_eq!(
            data.words[0].region,
            BoxRect {
                left: 0.1,
                top: 0.2,
                right: 0.4,
                bottom: 0.6
            }
        );
        assert_eq!(
            data.lines[0].shape.bounds(),
            BoxRect {
                left: 0.1,
                top: 0.2,
                right: 0.7,
                bottom: 0.6
            }
        );
        for record in [
            "5\t1\t1\t1\t1\t1\t90\t0\t30\t20\t90\toutside\n",
            "5\t1\t1\t1\t1\t1\t0\t0\t10\t20\tNaN\tnan\n",
            "5\t1\t1\t1\t1\t1\t4294967295\t0\t20\t20\t90\toverflow\n",
        ] {
            assert!(checked_words(&format!("{HEADER}{record}"), 100, 100).is_err());
        }
        assert!(checked_words("not a TSV header\n", 100, 100).is_err());
    }
}
