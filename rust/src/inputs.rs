//! Bounded, page-ordered local image imports use the same recognition path as PDFs.
use crate::{
    scan,
    storage::{Control, fingerprint, hash_file},
    types::*,
};
use image::DynamicImage;
use std::{
    fs,
    io::BufReader,
    path::{Path, PathBuf},
};

pub fn supported(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        ["pdf", "png", "jpg", "jpeg", "tif", "tiff"]
            .iter()
            .any(|e| s.eq_ignore_ascii_case(e))
    })
}
fn is_tiff(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("tif") || s.eq_ignore_ascii_case("tiff"))
}
pub fn files(source: &Path) -> Result<Vec<PathBuf>> {
    if source.is_dir() {
        let mut result = vec![];
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            if entry.file_type()?.is_file() && supported(&entry.path()) {
                result.push(entry.path());
                anyhow::ensure!(
                    result.len() <= 3000,
                    Fault::new("NG100", "Image folder exceeds 3000 files")
                );
            }
        }
        result.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
        anyhow::ensure!(
            !result.is_empty(),
            Fault::new("NG100", "Folder contains no supported scans")
        );
        Ok(result)
    } else {
        anyhow::ensure!(
            source.is_file() && supported(source),
            Fault::new("NG100", "Choose a PDF, PNG, JPEG, TIFF or scan folder")
        );
        Ok(vec![source.into()])
    }
}
pub fn hash(source: &Path, c: &Control) -> Result<String> {
    if !source.is_dir() {
        return hash_file(source, c);
    }
    let mut entries = vec![];
    for path in files(source)? {
        c.check()?;
        entries.push((
            path.file_name().unwrap().to_string_lossy().to_string(),
            hash_file(&path, c)?,
        ));
    }
    fingerprint(&entries)
}
fn tiff_decoder(path: &Path) -> Result<tiff::decoder::Decoder<BufReader<fs::File>>> {
    anyhow::ensure!(
        path.metadata()?.len() <= 2_000_000_000,
        Fault::new("NG100", "TIFF exceeds 2 GB")
    );
    let mut limits = tiff::decoder::Limits::default();
    limits.decoding_buffer_size = 160_000_000;
    limits.intermediate_buffer_size = 160_000_000;
    limits.ifd_value_size = 1_000_000;
    Ok(tiff::decoder::Decoder::new(BufReader::new(fs::File::open(path)?))?.with_limits(limits))
}
fn file_count(path: &Path, library: &Path) -> Result<usize> {
    if path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("pdf"))
    {
        scan::selected_page_count(path, library, "")
    } else if is_tiff(path) {
        let mut decoder = tiff_decoder(path)?;
        let mut n = 1;
        while decoder.more_images() {
            anyhow::ensure!(n < 3000, Fault::new("NG100", "TIFF exceeds 3000 pages"));
            decoder.next_image()?;
            n += 1;
        }
        Ok(n)
    } else {
        let (w, h) = image::image_dimensions(path)?;
        anyhow::ensure!(w > 0 && h > 0, Fault::new("NG100", "Empty image"));
        Ok(1)
    }
}
pub fn selected_page_count(source: &Path, library: &Path, spec: &str) -> Result<usize> {
    let mut count = 0;
    for path in files(source)? {
        count += file_count(&path, library)?;
        anyhow::ensure!(
            count <= 3000,
            Fault::new("NG100", "Input exceeds 3000 pages")
        );
    }
    Ok(pages_range(spec, count)?.len())
}
pub fn render(
    source: &Path,
    library: &Path,
    spec: &str,
    dpi: u32,
    max_pixels: u32,
    c: &Control,
    mut callback: impl FnMut(usize, DynamicImage) -> Result<()>,
) -> Result<Vec<usize>> {
    let files = files(source)?;
    let counts: Vec<usize> = files
        .iter()
        .map(|p| file_count(p, library))
        .collect::<Result<_>>()?;
    let total: usize = counts.iter().sum();
    anyhow::ensure!(
        total <= 3000,
        Fault::new("NG100", "Input exceeds 3000 pages")
    );
    let selected = pages_range(spec, total)?;
    let mut offset = 0;
    for (path, count) in files.iter().zip(counts) {
        c.check()?;
        let local: Vec<usize> = selected
            .iter()
            .filter(|n| **n > offset && **n <= offset + count)
            .map(|n| n - offset)
            .collect();
        if local.is_empty() {
            offset += count;
            continue;
        }
        if path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("pdf"))
        {
            let range = local
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(",");
            scan::render_pdf(path, library, &range, dpi, max_pixels, c, |n, im| {
                callback(offset + n, im)
            })?;
        } else if is_tiff(path) {
            let mut decoder = tiff_decoder(path)?;
            for n in 1..=count {
                c.check()?;
                if local.contains(&n) {
                    let (w, h) = decoder.dimensions()?;
                    anyhow::ensure!(
                        w > 0 && h > 0 && u64::from(w) * u64::from(h) <= u64::from(max_pixels),
                        Fault::new(
                            "NG100",
                            "TIFF page exceeds pixel budget; resize before importing"
                        )
                    );
                    let color = decoder.colortype()?;
                    let tiff::decoder::DecodingResult::U8(bytes) = decoder.read_image()? else {
                        return Err(Fault::new(
                            "NG100",
                            "Only 8-bit TIFF samples are supported; convert this scan to PNG",
                        )
                        .into());
                    };
                    let im =
                        match color {
                            tiff::ColorType::Gray(8) => image::GrayImage::from_raw(w, h, bytes)
                                .map(DynamicImage::ImageLuma8),
                            tiff::ColorType::RGB(8) => {
                                image::RgbImage::from_raw(w, h, bytes).map(DynamicImage::ImageRgb8)
                            }
                            tiff::ColorType::RGBA(8) => image::RgbaImage::from_raw(w, h, bytes)
                                .map(DynamicImage::ImageRgba8),
                            _ => None,
                        }
                        .ok_or_else(|| {
                            Fault::new("NG100", "Unsupported TIFF color format; convert to RGB PNG")
                        })?;
                    callback(offset + n, im)?;
                }
                if n < count {
                    decoder.next_image()?;
                }
            }
        } else {
            callback(
                offset + 1,
                crate::writers::open_image(path, u64::from(max_pixels))?,
            )?;
        }
        offset += count;
    }
    Ok(selected)
}
