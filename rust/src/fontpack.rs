//! Small, deterministic TrueType exporter for explicitly labelled English glyphs.
use crate::{storage::Control, types::*, writers};
use image::{DynamicImage, GrayImage, Luma};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::Path,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct FontOptions {
    pub family: String,
    pub spacing: u16,
    pub threshold: u8,
}
impl Default for FontOptions {
    fn default() -> Self {
        Self {
            family: "My handwriting".into(),
            spacing: 90,
            threshold: 128,
        }
    }
}
#[derive(Clone)]
struct Glyph {
    contours: Vec<Vec<(i16, i16)>>,
    advance: u16,
}
fn u16b(v: &mut Vec<u8>, n: u16) {
    v.extend(n.to_be_bytes());
}
fn i16b(v: &mut Vec<u8>, n: i16) {
    v.extend(n.to_be_bytes());
}
fn u32b(v: &mut Vec<u8>, n: u32) {
    v.extend(n.to_be_bytes());
}
fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0u32, |s, c| {
        let mut b = [0; 4];
        b[..c.len()].copy_from_slice(c);
        s.wrapping_add(u32::from_be_bytes(b))
    })
}
fn pad(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
}
fn contours(im: &GrayImage, threshold: u8) -> Vec<Vec<(i16, i16)>> {
    type Point = (i16, i16);
    let mut edges: BTreeMap<Point, Vec<Point>> = BTreeMap::new();
    let black = |x: i32, y: i32| {
        x >= 0
            && y >= 0
            && x < im.width() as i32
            && y < im.height() as i32
            && im[(x as u32, y as u32)][0] < threshold
    };
    for y in 0..im.height() as i32 {
        for x in 0..im.width() as i32 {
            if !black(x, y) {
                continue;
            }
            let (x, y) = (x as i16, y as i16);
            for (outside, a, b) in [
                (!black(x as i32, y as i32 - 1), (x, y), (x + 1, y)),
                (!black(x as i32 + 1, y as i32), (x + 1, y), (x + 1, y + 1)),
                (!black(x as i32, y as i32 + 1), (x + 1, y + 1), (x, y + 1)),
                (!black(x as i32 - 1, y as i32), (x, y + 1), (x, y)),
            ] {
                if outside {
                    edges.entry(a).or_default().push(b);
                }
            }
        }
    }
    let direction = |a: Point, b: Point| match (b.0 - a.0, b.1 - a.1) {
        (1, 0) => 0,
        (0, 1) => 1,
        (-1, 0) => 2,
        _ => 3,
    };
    let mut result = vec![];
    while let Some((&start, _)) = edges.first_key_value() {
        let mut at = start;
        let mut previous = None;
        let mut points = vec![];
        loop {
            points.push(at);
            let Some(choices) = edges.get_mut(&at) else {
                break;
            };
            let index = if let Some(dir) = previous {
                choices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, b)| match (direction(at, **b) + 4 - dir) % 4 {
                        1 => 0,
                        0 => 1,
                        3 => 2,
                        _ => 3,
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            } else {
                0
            };
            let next = choices.swap_remove(index);
            let empty = choices.is_empty();
            if empty {
                edges.remove(&at);
            }
            previous = Some(direction(at, next));
            at = next;
            if at == start {
                break;
            }
            if points.len() > 25000 {
                break;
            }
        }
        if points.len() < 4 {
            continue;
        }
        let n = points.len();
        let simplified = (0..n)
            .filter(|i| {
                let a = points[(i + n - 1) % n];
                let b = points[*i];
                let c = points[(i + 1) % n];
                (b.0 - a.0) * (c.1 - b.1) != (b.1 - a.1) * (c.0 - b.0)
            })
            .map(|i| points[i])
            .collect::<Vec<_>>();
        if simplified.len() >= 3 {
            result.push(simplified);
        }
    }
    result
}
fn glyph(im: &GrayImage, ch: char, o: &FontOptions) -> Result<Glyph> {
    let mut cs = contours(im, o.threshold);
    anyhow::ensure!(
        !cs.is_empty(),
        Fault::new(
            "NG620",
            format!(
                "Averaging removed all ink for {ch}. Add consistent samples or change the font threshold."
            )
        )
    );
    let left = cs.iter().flatten().map(|p| p.0).min().unwrap();
    let right = cs.iter().flatten().map(|p| p.0).max().unwrap();
    let top = cs.iter().flatten().map(|p| p.1).min().unwrap();
    let bottom = cs.iter().flatten().map(|p| p.1).max().unwrap();
    let desc = "gjpqy,;".contains(ch);
    let target = if ch.is_ascii_punctuation() {
        200.
    } else if ch.is_ascii_lowercase() {
        if desc { 750. } else { 550. }
    } else {
        800.
    };
    let scale = (target / (bottom - top).max(1) as f32).min(1600. / (right - left).max(1) as f32);
    let base = if desc { -200. } else { 0. };
    for c in &mut cs {
        for p in c.iter_mut() {
            *p = (
                ((p.0 - left) as f32 * scale + 45.).round() as i16,
                ((bottom - p.1) as f32 * scale + base).round() as i16,
            );
        }
        c.reverse();
    }
    let points = cs.iter().map(Vec::len).sum::<usize>();
    anyhow::ensure!(
        points < 32760 && cs.len() < 32760,
        "Glyph outline is too complex"
    );
    Ok(Glyph {
        contours: cs,
        advance: ((right - left) as f32 * scale + 45. + o.spacing as f32)
            .round()
            .clamp(150., 2000.) as u16,
    })
}
fn encode_glyph(g: &Glyph) -> Vec<u8> {
    if g.contours.is_empty() {
        return vec![];
    }
    let points = g.contours.iter().flatten().copied().collect::<Vec<_>>();
    let mut v = vec![];
    i16b(&mut v, g.contours.len() as i16);
    for n in [
        points.iter().map(|p| p.0).min().unwrap_or(0),
        points.iter().map(|p| p.1).min().unwrap_or(0),
        points.iter().map(|p| p.0).max().unwrap_or(0),
        points.iter().map(|p| p.1).max().unwrap_or(0),
    ] {
        i16b(&mut v, n);
    }
    let mut end = 0;
    for c in &g.contours {
        end += c.len();
        u16b(&mut v, (end - 1) as u16);
    }
    u16b(&mut v, 0);
    v.extend(std::iter::repeat_n(1, points.len()));
    let mut last = 0;
    for p in &points {
        i16b(&mut v, p.0 - last);
        last = p.0;
    }
    last = 0;
    for p in &points {
        i16b(&mut v, p.1 - last);
        last = p.1;
    }
    pad(&mut v);
    v
}
fn name_table(family: &str) -> Vec<u8> {
    let ps = family
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(55)
        .collect::<String>();
    let records = [
        (1, family.to_string()),
        (2, "Regular".into()),
        (3, format!("NoteGen:{ps}:1")),
        (4, format!("{family} Regular")),
        (5, "Version 1.000; NoteGen confirmed samples".into()),
        (6, format!("{ps}-Regular")),
    ];
    let mut v = vec![];
    u16b(&mut v, 0);
    u16b(&mut v, records.len() as u16);
    u16b(&mut v, (6 + 12 * records.len()) as u16);
    let mut strings = vec![];
    for (id, s) in records {
        let bytes = s
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        for n in [3, 1, 0x409, id, bytes.len() as u16, strings.len() as u16] {
            u16b(&mut v, n);
        }
        strings.extend(bytes);
    }
    v.extend(strings);
    v
}
fn ttf(glyphs: &[(char, Glyph)], family: &str) -> Vec<u8> {
    let mut tables: BTreeMap<[u8; 4], Vec<u8>> = BTreeMap::new();
    let fallback = Glyph {
        contours: vec![
            vec![(50, 0), (50, 700), (450, 700), (450, 0)],
            vec![(100, 50), (400, 50), (400, 650), (100, 650)],
        ],
        advance: 500,
    };
    let blank = Glyph {
        contours: vec![],
        advance: 300,
    };
    let all = std::iter::once(&fallback)
        .chain(std::iter::once(&blank))
        .chain(glyphs.iter().map(|(_, g)| g))
        .collect::<Vec<_>>();
    let mut glyf = vec![];
    let mut loca = vec![];
    let mut hmtx = vec![];
    for g in &all {
        u32b(&mut loca, glyf.len() as u32);
        glyf.extend(encode_glyph(g));
        u16b(&mut hmtx, g.advance);
        i16b(
            &mut hmtx,
            if g.contours.is_empty() {
                0
            } else {
                g.contours.iter().flatten().map(|p| p.0).min().unwrap_or(0)
            },
        );
    }
    u32b(&mut loca, glyf.len() as u32);
    tables.insert(*b"glyf", glyf);
    tables.insert(*b"loca", loca);
    tables.insert(*b"hmtx", hmtx);
    let mut head = vec![];
    u32b(&mut head, 0x10000);
    u32b(&mut head, 0x10000);
    u32b(&mut head, 0);
    u32b(&mut head, 0x5f0f3cf5);
    u16b(&mut head, 3);
    u16b(&mut head, 1000);
    head.extend(3_850_070_400i64.to_be_bytes());
    head.extend(3_850_070_400i64.to_be_bytes());
    for n in [0, -200, 2000, 800, 0, 8, 2, 1, 0] {
        i16b(&mut head, n);
    }
    tables.insert(*b"head", head);
    let mut hhea = vec![];
    u32b(&mut hhea, 0x10000);
    for n in [850, -250, 100] {
        i16b(&mut hhea, n);
    }
    u16b(
        &mut hhea,
        all.iter().map(|g| g.advance).max().unwrap_or(500),
    );
    for n in [0, 0, 2000, 1, 0, 0, 0, 0, 0, 0, 0] {
        i16b(&mut hhea, n);
    }
    u16b(&mut hhea, all.len() as u16);
    tables.insert(*b"hhea", hhea);
    let mut maxp = vec![];
    u32b(&mut maxp, 0x10000);
    u16b(&mut maxp, all.len() as u16);
    u16b(
        &mut maxp,
        all.iter()
            .map(|g| g.contours.iter().map(Vec::len).sum::<usize>())
            .max()
            .unwrap_or(0) as u16,
    );
    u16b(
        &mut maxp,
        all.iter().map(|g| g.contours.len()).max().unwrap_or(0) as u16,
    );
    for n in [0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        u16b(&mut maxp, n);
    }
    tables.insert(*b"maxp", maxp);
    let mut mappings = vec![(32u16, 1u16)];
    mappings.extend(
        glyphs
            .iter()
            .enumerate()
            .map(|(i, (c, _))| (*c as u16, i as u16 + 2)),
    );
    mappings.push((0xffff, 0));
    let n = mappings.len() as u16;
    let pow = 1u16 << (15 - n.leading_zeros());
    let mut sub = vec![];
    for value in [
        4,
        16 + 8 * n,
        0,
        2 * n,
        2 * pow,
        pow.trailing_zeros() as u16,
        2 * n - 2 * pow,
    ] {
        u16b(&mut sub, value);
    }
    for (c, _) in &mappings {
        u16b(&mut sub, *c);
    }
    u16b(&mut sub, 0);
    for (c, _) in &mappings {
        u16b(&mut sub, *c);
    }
    for (c, g) in &mappings {
        u16b(&mut sub, g.wrapping_sub(*c));
    }
    for _ in &mappings {
        u16b(&mut sub, 0);
    }
    let mut cmap = vec![];
    for n in [0, 1, 3, 1] {
        u16b(&mut cmap, n);
    }
    u32b(&mut cmap, 12);
    cmap.extend(sub);
    tables.insert(*b"cmap", cmap);
    tables.insert(*b"name", name_table(family));
    let mut post = vec![];
    u32b(&mut post, 0x30000);
    u32b(&mut post, 0);
    i16b(&mut post, -100);
    i16b(&mut post, 50);
    post.extend([0; 20]);
    tables.insert(*b"post", post);
    let mut os = vec![];
    u16b(&mut os, 4);
    i16b(&mut os, 500);
    for n in [400, 5, 0] {
        u16b(&mut os, n);
    }
    for n in [650, 600, 0, 75, 650, 600, 0, 350, 50, 300, 0] {
        i16b(&mut os, n);
    }
    os.extend([0; 10]);
    u32b(&mut os, 1);
    for _ in 0..3 {
        u32b(&mut os, 0);
    }
    os.extend(b"NTGN");
    u16b(&mut os, 0x40);
    u16b(&mut os, 32);
    u16b(&mut os, glyphs.last().map_or(32, |(c, _)| *c as u16));
    for n in [850, -250, 100] {
        i16b(&mut os, n);
    }
    u16b(&mut os, 850);
    u16b(&mut os, 250);
    u32b(&mut os, 1);
    u32b(&mut os, 0);
    for n in [550, 800, 0, 32, 2] {
        u16b(&mut os, n);
    }
    tables.insert(*b"OS/2", os);
    let count = tables.len() as u16;
    let pow = 1u16 << (15 - count.leading_zeros());
    let mut output = vec![];
    u32b(&mut output, 0x10000);
    for n in [
        count,
        pow * 16,
        pow.trailing_zeros() as u16,
        count * 16 - pow * 16,
    ] {
        u16b(&mut output, n);
    }
    let mut offset = 12 + count as usize * 16;
    let mut head_offset = 0;
    for (tag, data) in &tables {
        output.extend(tag);
        u32b(&mut output, checksum(data));
        u32b(&mut output, offset as u32);
        u32b(&mut output, data.len() as u32);
        if tag == b"head" {
            head_offset = offset;
        }
        offset += data.len().div_ceil(4) * 4;
    }
    for data in tables.values() {
        output.extend(data);
        pad(&mut output);
    }
    let adjustment = 0xb1b0afbau32.wrapping_sub(checksum(&output));
    output[head_offset + 8..head_offset + 12].copy_from_slice(&adjustment.to_be_bytes());
    output
}
pub fn export(root: &Path, id: &str, output: &Path, o: &FontOptions, c: &Control) -> Result<usize> {
    anyhow::ensure!(
        !output.exists(),
        Fault::new("NG401", "Font pack already exists")
    );
    anyhow::ensure!(
        !o.family.trim().is_empty()
            && o.family.len() <= 100
            && o.family.chars().any(|c| c.is_ascii_alphanumeric())
            && o.spacing <= 500
            && (32..=224).contains(&o.threshold),
        Fault::new("NG620", "Invalid font family, spacing or threshold")
    );
    let mut p = writers::read(root, id)?;
    p.samples.retain(|sample| sample.active());
    let chars = p
        .samples
        .iter()
        .filter(|s| s.glyph && s.active())
        .filter_map(|s| s.label.chars().next())
        .filter(char::is_ascii_graphic)
        .collect::<BTreeSet<_>>();
    anyhow::ensure!(
        !chars.is_empty(),
        Fault::new(
            "NG620",
            "Label individual letters in Review → Teach handwriting before exporting a font"
        )
    );
    let mut glyphs = vec![];
    let mut images = vec![];
    for ch in chars {
        c.check()?;
        let im = writers::average_glyph(root, &p, ch)?;
        glyphs.push((ch, glyph(&im, ch, o)?));
        images.push((ch, im));
    }
    let bytes = ttf(&glyphs, &o.family);
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut zip = zip::ZipWriter::new(&mut temp);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("Handwriting.ttf", options)?;
        zip.write_all(&bytes)?;
        zip.start_file("profile.json", options)?;
        zip.write_all(&serde_json::to_vec_pretty(&p)?)?;
        let missing = ('A'..='Z')
            .chain('a'..='z')
            .chain('0'..='9')
            .filter(|c| !glyphs.iter().any(|(g, _)| c == g))
            .collect::<String>();
        zip.start_file("README.txt", options)?;
        write!(
            zip,
            "{} — NoteGen handwriting font\n{} confirmed glyphs; averaged up to 32 examples each.\nMissing letters/numbers: {}\nOnly explicitly labelled glyphs are mapped. Install Handwriting.ttf in Windows Settings > Fonts.\nThis is a personal print-style outline font; cursive joining and kerning are not inferred.\nYour profile and source examples are private. Share this pack only if you intend to share your handwriting.\n",
            o.family,
            glyphs.len(),
            missing
        )?;
        let mut preview =
            GrayImage::from_pixel(640, (images.len() as u32).div_ceil(10) * 96, Luma([245]));
        for (i, (ch, im)) in images.iter().enumerate() {
            image::imageops::replace(&mut preview, im, (i % 10 * 64) as i64, (i / 10 * 96) as i64);
            let g = &glyphs[i].1;
            let path = g
                .contours
                .iter()
                .map(|c| {
                    format!(
                        "M {} Z",
                        c.iter()
                            .map(|(x, y)| format!("{x},{}", 800 - y))
                            .collect::<Vec<_>>()
                            .join(" L ")
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            zip.start_file(format!("svg/U{:04X}.svg", *ch as u32), options)?;
            write!(
                zip,
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} 1100\"><path d=\"{}\"/></svg>",
                g.advance, path
            )?;
        }
        let mut png = std::io::Cursor::new(vec![]);
        DynamicImage::ImageLuma8(preview).write_to(&mut png, image::ImageFormat::Png)?;
        zip.start_file("coverage.png", options)?;
        zip.write_all(&png.into_inner())?;
        for sample in &p.samples {
            c.check()?;
            let source = root.join(&p.id).join(&sample.image);
            let data = fs::read(source)?;
            anyhow::ensure!(data.len() <= 1_000_000, "Oversized sample");
            zip.start_file(format!("samples/{}", sample.image), options)?;
            zip.write_all(&data)?;
        }
        zip.finish()?;
    }
    temp.as_file().sync_all()?;
    temp.persist_noclobber(output).map_err(|e| e.error)?;
    Ok(glyphs.len())
}
