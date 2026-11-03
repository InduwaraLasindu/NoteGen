//! Literal code OCR and explicit, source-bound review. Never executes recognized code.
use crate::{
    geometry,
    redactions::{self, SourceIdentity},
    storage::{Control, Project, fingerprint},
    types::*,
};
use image::DynamicImage;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::Read, time::Duration};

fn fail(message: &str) -> anyhow::Error {
    Fault::new("NG201", message).into()
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Indentation {
    /// Retain native TXT whitespace exactly; leading indentation may be missed by OCR.
    #[default]
    Native,
    /// Explicit source-pixel calibration, relative to the selected crop.
    Monospace {
        left_margin_pixels: u32,
        space_width_pixels: f32,
    },
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum LineNumbers {
    #[default]
    Keep,
    /// Every nonblank line must match this consecutive gutter. Exact recognized
    /// prefixes remain in visible metadata and the immutable raw OCR candidate.
    SeparateSequential { first: u32, separator: String },
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CodeOptions {
    pub indentation: Indentation,
    pub line_numbers: LineNumbers,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeCodeReading {
    pub content: PageContent,
    pub raw_content: PageContent,
    pub separated_line_numbers: Vec<String>,
    pub note: String,
}
fn read_text(path: &std::path::Path, max: u64, c: &Control) -> Result<String> {
    crate::watch::no_links(path)?;
    let file = fs::File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= max,
        fail("Native code OCR exceeded its output budget")
    );
    let mut bytes = vec![];
    let mut reader = file.take(max + 1);
    let mut chunk = [0u8; 65536];
    loop {
        c.check()?;
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        anyhow::ensure!(
            bytes.len() as u64 + n as u64 <= max,
            fail("Native code OCR output grew beyond its budget")
        );
        bytes.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8(bytes).map_err(|_| fail("Native code OCR returned non-UTF-8 output"))
}
fn parts(line: &str) -> (&str, &str) {
    if let Some(body) = line.strip_suffix("\r\n") {
        (body, "\r\n")
    } else if let Some(body) = line.strip_suffix('\n') {
        (body, "\n")
    } else {
        (line, "")
    }
}
fn format_code(
    raw: &str,
    words: &[crate::scan::Word],
    width: u32,
    options: &CodeOptions,
) -> Result<(String, Vec<String>)> {
    let mut lines = BTreeMap::<(u32, u32, u32), Vec<&crate::scan::Word>>::new();
    for word in words {
        lines.entry(word.group).or_default().push(word);
    }
    let lines = lines.into_values().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut next_number = match &options.line_numbers {
        LineNumbers::SeparateSequential { first, .. } => *first,
        LineNumbers::Keep => 0,
    };
    if let LineNumbers::SeparateSequential { separator, .. } = &options.line_numbers {
        anyhow::ensure!(
            !separator.is_empty()
                && separator.len() <= 8
                && separator.bytes().all(|b| b" \t:.)|".contains(&b)),
            fail("Confirm a short literal line-number separator")
        );
    }
    let mut output = String::new();
    let mut numbers = vec![];
    for segment in raw.split_inclusive('\n') {
        let (body, ending) = parts(segment);
        if body.trim().is_empty() {
            output.push_str(segment);
            continue;
        }
        let mut line = body.to_owned();
        if let Indentation::Monospace {
            left_margin_pixels,
            space_width_pixels,
        } = options.indentation
        {
            anyhow::ensure!(
                left_margin_pixels < width
                    && space_width_pixels.is_finite()
                    && (1.0..=512.0).contains(&space_width_pixels),
                fail("Confirm a finite 1–512 pixel space width and source margin inside the crop")
            );
            let observed = lines.get(index).ok_or_else(|| fail("Native TXT and TSV lines disagree; retain native whitespace or repair this code manually"))?;
            let txt = body
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            let tsv = observed
                .iter()
                .flat_map(|w| w.text.chars().filter(|c| !c.is_whitespace()))
                .collect::<String>();
            anyhow::ensure!(
                txt == tsv,
                fail("Native TXT and TSV content disagree; calibrated indentation was not applied")
            );
            let first = observed.iter().map(|w| w.x).min().unwrap();
            anyhow::ensure!(
                first >= left_margin_pixels,
                fail("A recognized line begins before the confirmed code margin")
            );
            let columns =
                ((first - left_margin_pixels) as f32 / space_width_pixels).round() as usize;
            anyhow::ensure!(
                columns <= 512,
                fail("Code indentation exceeds 512 calibrated columns")
            );
            line = format!(
                "{}{}",
                " ".repeat(columns),
                body.trim_start_matches([' ', '\t'])
            );
        }
        if let LineNumbers::SeparateSequential { separator, .. } = &options.line_numbers {
            let leading = line.len() - line.trim_start_matches([' ', '\t']).len();
            let rest = &line[leading..];
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            anyhow::ensure!(
                (1..=10).contains(&digits) && rest[digits..].starts_with(separator),
                fail(
                    "Every nonblank line must contain the explicitly confirmed sequential number and separator; no prefixes were removed"
                )
            );
            let number = &rest[..digits];
            anyhow::ensure!(
                number.parse::<u32>().ok() == Some(next_number),
                fail(
                    "Line-number sequence is missing, repeated or ambiguous; no prefixes were removed"
                )
            );
            numbers.push(number.to_owned());
            line = format!("{}{}", &line[..leading], &rest[digits + separator.len()..]);
            next_number = next_number
                .checked_add(1)
                .ok_or_else(|| fail("Line-number sequence overflow"))?;
        }
        output.push_str(&line);
        output.push_str(ending);
        anyhow::ensure!(
            output.len() < 120_000,
            fail("Code text exceeds its 120 KB review budget")
        );
        index += 1;
    }
    if matches!(options.indentation, Indentation::Monospace { .. }) {
        anyhow::ensure!(
            index == lines.len(),
            fail("Native TXT and TSV line counts disagree; calibrated indentation was not applied")
        );
    }
    Ok((output, numbers))
}
/// One native invocation produces literal TXT plus actual TSV geometry. No table
/// line removal, spelling correction, smart quotes, syntax fixing or execution.
pub fn recognize_image(
    image: &DynamicImage,
    settings: &Settings,
    options: &CodeOptions,
    c: &Control,
) -> Result<NativeCodeReading> {
    c.check()?;
    anyhow::ensure!(
        image.width() > 0
            && image.height() > 0
            && image.width() <= 20000
            && image.height() <= 20000
            && u64::from(image.width()) * u64::from(image.height()) <= 40_000_000,
        fail("Code source exceeds its pixel budget")
    );
    if !settings.tessdata_dir.as_os_str().is_empty() {
        crate::languages::verify_selection(&settings.tessdata_dir, &settings.languages, c)?;
    }
    let directory = crate::storage::temporary_dir(c, "notegen-code-")?;
    let input = directory.path().join("source.png");
    let output = directory.path().join("reading");
    image.save(&input)?;
    let args = crate::scan::ocr_args(
        &[
            input.to_string_lossy().into_owned(),
            output.to_string_lossy().into_owned(),
            "-l".into(),
            settings.languages.clone(),
            "--psm".into(),
            "6".into(),
            "-c".into(),
            "preserve_interword_spaces=1".into(),
            "-c".into(),
            "load_system_dawg=0".into(),
            "-c".into(),
            "load_freq_dawg=0".into(),
            "-c".into(),
            "page_separator=".into(),
            "txt".into(),
            "tsv".into(),
        ],
        settings,
    );
    let (ok, _, _) = crate::scan::run_with_threads(
        &settings.tesseract,
        &args,
        Duration::from_secs(90),
        c,
        if settings.cpu_threads == 0 {
            4
        } else {
            settings.cpu_threads.min(4)
        },
    )?;
    anyhow::ensure!(
        ok,
        fail("Native code OCR failed; check the executable and selected languages")
    );
    let raw = read_text(&output.with_extension("txt"), 119_999, c)?;
    let tsv = read_text(&output.with_extension("tsv"), 16_000_000, c)?;
    reading_from_outputs(&raw, &tsv, image.width(), image.height(), options)
}
fn reading_from_outputs(
    raw: &str,
    tsv: &str,
    width: u32,
    height: u32,
    options: &CodeOptions,
) -> Result<NativeCodeReading> {
    anyhow::ensure!(
        raw.len() < 120_000,
        fail("Code text exceeds its 120 KB review budget")
    );
    let words = crate::scan::checked_words(tsv, width, height)?;
    let (text, numbers) = format_code(raw, &words, width, options)?;
    let refs = words.iter().collect::<Vec<_>>();
    let geometry = crate::scan::observed_geometry(&refs, width, height, None);
    let raw_content = PageContent {
        blocks: vec![Block {
            kind: "code".into(),
            text: raw.into(),
            bbox: Some(BoxRect::full()),
            geometry: Some(geometry.clone()),
            uncertain: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut content = PageContent { blocks:vec![Block { kind:"code".into(),text,bbox:Some(BoxRect::full()),geometry:Some(geometry),uncertain:true,..Default::default() }],warnings:vec!["Code OCR is unverified. Compare punctuation, indentation, quotes and line numbers with the source; no recognized code is executed. Native whitespace cannot distinguish tabs from spaces or recover every blank line.".into()],..Default::default() };
    if !numbers.is_empty() {
        content.blocks.push(Block { kind:"caption".into(),text:format!("Explicitly separated source line numbers: {}. Original numbered OCR remains reader evidence.",numbers.join(", ")),bbox:Some(BoxRect::full()),..Default::default() });
    }
    raw_content.validate()?;
    content.validate()?;
    Ok(NativeCodeReading { content,raw_content,separated_line_numbers:numbers,note:"Native text is retained exactly as OCR supplied it. Only explicitly selected indentation calibration and verified line-number separation change the code preview. OCR does not establish syntactic or semantic correctness; handwritten code still requires source review.".into() })
}
#[derive(Clone, Debug, Serialize)]
pub struct CodePreview {
    pub source: SourceIdentity,
    pub before_fingerprint: String,
    pub block: usize,
    pub options: CodeOptions,
    pub after: Page,
    pub token: String,
    pub note: String,
}
fn token(
    source: &SourceIdentity,
    before: &str,
    block: usize,
    options: &CodeOptions,
    after: &Page,
) -> Result<String> {
    fingerprint(&(1, source, before, block, options, after))
}
pub fn preview(
    project: &Project,
    expected: &Page,
    block: usize,
    options: &CodeOptions,
    settings: &Settings,
    c: &Control,
) -> Result<CodePreview> {
    let before = fingerprint(expected)?;
    anyhow::ensure!(
        fingerprint(&project.load(expected.page)?)? == before && expected.candidates.len() <= 1022,
        fail("Page changed or its reader evidence budget is full")
    );
    let selected = expected
        .content
        .blocks
        .get(block)
        .ok_or_else(|| fail("Select an existing code/text source block"))?;
    anyhow::ensure!(
        matches!(selected.kind.as_str(), "text" | "code"),
        fail("Code OCR replaces only an explicitly selected whole text/code block")
    );
    let region = selected
        .bbox
        .as_ref()
        .filter(|r| r.valid())
        .ok_or_else(|| fail("Code OCR requires a source-grounded block"))?;
    let (source, image) = redactions::recognition_view(project, expected, c)?;
    let (l, t, r, b) = region.pixels(image.width(), image.height());
    let view = BoxRect {
        left: l as f32 / image.width() as f32,
        top: t as f32 / image.height() as f32,
        right: r as f32 / image.width() as f32,
        bottom: b as f32 / image.height() as f32,
    };
    let settings = redactions::recognition_settings(settings, &expected.confirmed_redactions)?;
    let reading = recognize_image(&image.crop_imm(l, t, r - l, b - t), &settings, options, c)?;
    anyhow::ensure!(
        reading
            .content
            .blocks
            .iter()
            .any(|b| b.kind == "code" && !b.text.trim().is_empty()),
        fail("No usable code was recognized; the existing reading was preserved")
    );
    let mut contents = [reading.raw_content, reading.content];
    for content in &mut contents {
        if !expected.confirmed_redactions.is_empty() {
            *content = redactions::enforce_view(
                content,
                &source,
                &expected.confirmed_redactions,
                Some(&view),
                c,
            )?
            .0;
        }
        geometry::map_content(content, &view, &source)?;
        if !expected.confirmed_redactions.is_empty() {
            *content = redactions::enforce_view(
                content,
                &source,
                &expected.confirmed_redactions,
                None,
                c,
            )?
            .0;
            content.blocks.retain_mut(|block| {
                let Some(bounds) = &block.bbox else {
                    return false;
                };
                let clipped = BoxRect {
                    left: bounds.left.max(view.left),
                    top: bounds.top.max(view.top),
                    right: bounds.right.min(view.right),
                    bottom: bounds.bottom.min(view.bottom),
                };
                if !clipped.valid() {
                    return false;
                }
                block.bbox = Some(clipped);
                true
            });
        }
    }
    let [raw, processed] = contents;
    let mut after = expected.clone();
    after
        .content
        .blocks
        .splice(block..=block, processed.blocks.clone());
    after.content.warnings.extend(processed.warnings.clone());
    for (view_name, content) in [("raw", raw), ("preview", processed)] {
        after.candidates.push(Candidate {
            model_id: format!("tesseract:{}", settings.languages),
            family: "tesseract".into(),
            view: format!("code_{view_name}:block-{block}"),
            region: Some(view.clone()),
            content,
            reliability: 0.65,
        });
    }
    if !expected.confirmed_redactions.is_empty() {
        after.content = redactions::enforce_view(
            &after.content,
            &source,
            &expected.confirmed_redactions,
            None,
            c,
        )?
        .0;
    }
    after.content.summary = after.content.plain().chars().take(2000).collect();
    after.reviewed = false;
    after.edited = true;
    after.agreement = 0.;
    after.validate()?;
    anyhow::ensure!(
        redactions::source_identity(project, expected, c)? == source
            && fingerprint(&project.load(expected.page)?)? == before,
        fail("Page/source changed during code OCR")
    );
    let token = token(&source, &before, block, options, &after)?;
    Ok(CodePreview {
        source,
        before_fingerprint: before,
        block,
        options: options.clone(),
        after,
        token,
        note: reading.note,
    })
}
pub fn apply_preview(
    project: &Project,
    expected: &Page,
    preview: &CodePreview,
    confirmation: &str,
    c: &Control,
) -> Result<u64> {
    let (source, _) = redactions::recognition_view(project, expected, c)?;
    anyhow::ensure!(
        source == preview.source
            && fingerprint(expected)? == preview.before_fingerprint
            && confirmation == preview.token
            && confirmation
                == token(
                    &source,
                    &preview.before_fingerprint,
                    preview.block,
                    &preview.options,
                    &preview.after
                )?
            && preview.after.page == expected.page
            && preview.after.image == expected.image
            && preview.after.confirmed_redactions == expected.confirmed_redactions
            && fingerprint(&preview.after.manual_layers)? == fingerprint(&expected.manual_layers)?,
        fail("Confirm the exact fresh code OCR preview")
    );
    anyhow::ensure!(
        preview.after.candidates.len() >= expected.candidates.len(),
        fail("Code OCR cannot discard existing reader evidence")
    );
    let retained = &preview.after.candidates[..expected.candidates.len()];
    anyhow::ensure!(
        fingerprint(&retained)? == fingerprint(&expected.candidates)?,
        fail("Code OCR cannot discard existing reader evidence")
    );
    preview.after.validate()?;
    c.check()?;
    crate::review::commit(project, expected, &preview.after, "Apply literal code OCR")
}

#[cfg(test)]
mod tests {
    use super::*;
    const HEADER: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n";
    fn tsv(lines: &[(&str, u32)]) -> String {
        let mut text = HEADER.to_owned();
        for (line, (value, x)) in lines.iter().enumerate() {
            text.push_str(&format!(
                "5\t1\t1\t1\t{}\t1\t{x}\t{}\t100\t15\t95\t{value}\n",
                line + 1,
                10 + line * 25
            ));
        }
        text
    }
    #[test]
    fn default_keeps_literal_code_whitespace_punctuation_and_numbered_lines() {
        let raw =
            "001: if (x == 1250) {\r\n\t002:     print(\"a  b\\t\");  // !?\r\n\r\n003: }\r\n";
        let native = tsv(&[("001:", 10), ("002:", 20), ("003:", 10)]);
        let read = reading_from_outputs(raw, &native, 500, 200, &CodeOptions::default()).unwrap();
        assert_eq!(read.content.blocks[0].text, raw);
        assert_eq!(read.raw_content.blocks[0].text, raw);
        assert_eq!(read.content.blocks[0].kind, "code");
        assert!(read.content.blocks[0].uncertain);
        assert!(read.separated_line_numbers.is_empty());
        assert_eq!(read.content.blocks.len(), 1);
    }
    #[test]
    fn explicit_number_separation_is_strict_reversible_evidence_and_preserves_other_bytes() {
        let raw = "001: if (x == 1250) {\r\n002:     x += 1;  // keep  spaces\r\n\r\n003: }\r\n";
        let options = CodeOptions {
            line_numbers: LineNumbers::SeparateSequential {
                first: 1,
                separator: ": ".into(),
            },
            ..Default::default()
        };
        let native = tsv(&[("001:", 10), ("002:", 10), ("003:", 10)]);
        let read = reading_from_outputs(raw, &native, 500, 200, &options).unwrap();
        assert_eq!(
            read.content.blocks[0].text,
            "if (x == 1250) {\r\n    x += 1;  // keep  spaces\r\n\r\n}\r\n"
        );
        assert_eq!(read.separated_line_numbers, vec!["001", "002", "003"]);
        assert_eq!(read.raw_content.blocks[0].text, raw);
        assert!(read.content.blocks[1].text.contains("001, 002, 003"));
        for bad in [
            "001: a\n003: b\n",
            "001: a\n001: b\n",
            "001: a\nx += 1;\n",
            "001. a\n",
        ] {
            assert!(reading_from_outputs(bad, &native, 500, 200, &options).is_err());
        }
    }
    #[test]
    fn calibrated_indent_uses_source_pixels_only_when_native_lines_match_exactly() {
        let raw = "if (x == 1250) {\nreturn x + 1;  \n\n}\n";
        let native = tsv(&[("if (x == 1250) {", 20), ("return x + 1;", 60), ("}", 20)]);
        let options = CodeOptions {
            indentation: Indentation::Monospace {
                left_margin_pixels: 20,
                space_width_pixels: 10.,
            },
            ..Default::default()
        };
        let read = reading_from_outputs(raw, &native, 500, 200, &options).unwrap();
        assert_eq!(
            read.content.blocks[0].text,
            "if (x == 1250) {\n    return x + 1;  \n\n}\n"
        );
        assert_eq!(read.raw_content.blocks[0].text, raw);
        assert!(
            reading_from_outputs(
                "if (x != 1250) {\nreturn x + 1;\n}\n",
                &native,
                500,
                200,
                &options
            )
            .is_err()
        );
        for pitch in [f32::NAN, 0., 513.] {
            let invalid = CodeOptions {
                indentation: Indentation::Monospace {
                    left_margin_pixels: 20,
                    space_width_pixels: pitch,
                },
                ..Default::default()
            };
            assert!(reading_from_outputs(raw, &native, 500, 200, &invalid).is_err());
        }
    }
}
