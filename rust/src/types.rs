use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};

pub type Result<T> = anyhow::Result<T>;

#[derive(Debug, thiserror::Error)]
#[error("{code}: {detail}")]
pub struct Fault {
    pub code: &'static str,
    pub detail: String,
}
impl Fault {
    pub fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
    pub fn remedy(&self) -> &'static str {
        match self.code {
            "NG100" => "Choose a readable, unlocked PDF or valid page range.",
            "NG102" => "Check Advanced settings and restore defaults for invalid values.",
            "NG110" => {
                "Install Tesseract and the chosen language data, or configure a local vision model."
            }
            "NG200" => {
                "Configure a compatible llama-server executable and local model/projector files."
            }
            "NG201" => {
                "Review preserved candidates; the model returned invalid or truncated output."
            }
            "NG202" | "NG300" => {
                "Free RAM/VRAM or reduce model, image/context size. Resume saved work."
            }
            "NG203" => {
                "Use a literal loopback endpoint. Redirects and remote conversion servers are blocked."
            }
            "NG400" => {
                "Choose a writable output folder; close any open Word file and check disk space."
            }
            "NG401" => "Choose a new output filename; existing files are protected.",
            "NG500" => "Restore matching input/settings/models or start a new project.",
            "NG610" => {
                "Choose a valid writer and a tight crop containing visible handwriting. Label only confirmed examples."
            }
            "NG620" => {
                "Label individual English letters first, then export to a new font-pack filename."
            }
            "NG630" => {
                "Reopen the source block, confirm its text, and save a shorter clarification."
            }
            "NG640" => {
                "Enable public lookup and enter a topic; check the connection or continue offline."
            }
            "NG650" => {
                "Check model metadata, available disk space and the publisher revision. The active version is preserved."
            }
            "NG900" => "Completed work is saved. Resume the same input and settings.",
            _ => "Open the local activity log for the stage and error code.",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Fast,
    Balanced,
    #[default]
    Accuracy,
    Maximum,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    #[default]
    Auto,
    Cuda,
    Vulkan,
    Cpu,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub preset: Preset,
    pub automatic_models: bool,
    pub model_ids: Vec<String>,
    pub handwriting: bool,
    pub languages: String,
    pub page_range: String,
    pub dpi: u32,
    pub max_pixels: u32,
    pub image_edge: u32,
    pub tile_edge: u32,
    pub tile_overlap: f32,
    pub crop_retries: usize,
    pub consensus_min: usize,
    pub agreement_threshold: f64,
    pub review_threshold: f32,
    pub backend: Backend,
    pub gpu_index: Option<u32>,
    pub ram_fraction: f32,
    pub vram_fraction: f32,
    pub reserve_ram_mib: u64,
    pub reserve_vram_mib: u64,
    pub cpu_threads: usize,
    pub context: u32,
    pub max_tokens: u32,
    pub batch: u32,
    pub micro_batch: u32,
    pub image_tokens: u32,
    pub gpu_layers: i32,
    pub flash_attention: bool,
    pub projector_on_cpu: bool,
    pub kv_cache: String,
    pub temperature: f32,
    pub top_p: f32,
    pub seed: u64,
    pub request_timeout_secs: u64,
    pub allow_cpu_offload: bool,
    pub require_acceleration: bool,
    pub tesseract: PathBuf,
    pub pdfium: PathBuf,
    pub server: PathBuf,
    pub models_dir: PathBuf,
    pub endpoint: String,
    pub external_server: bool,
    pub subject: String,
    pub writer: crate::assistance::WriterOptions,
    pub assistance: crate::assistance::Options,
    #[serde(skip)]
    pub writer_reference: Option<crate::writers::Reference>,
    pub deskew: bool,
    pub auto_rotate: bool,
    pub preserve_figures: bool,
    pub keep_page_breaks: bool,
    pub font: String,
    pub font_size: f32,
    pub ui: Appearance,
    pub experience: ExperienceOptions,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            preset: Preset::Accuracy,
            automatic_models: true,
            model_ids: vec![],
            handwriting: false,
            languages: "eng".into(),
            page_range: String::new(),
            dpi: 0,
            max_pixels: 20_000_000,
            image_edge: 0,
            tile_edge: 1536,
            tile_overlap: 0.12,
            crop_retries: 8,
            consensus_min: 2,
            agreement_threshold: 0.92,
            review_threshold: 0.90,
            backend: Backend::Auto,
            gpu_index: None,
            ram_fraction: 0.85,
            vram_fraction: 0.92,
            reserve_ram_mib: 1024,
            reserve_vram_mib: 512,
            cpu_threads: 0,
            context: 0,
            max_tokens: 8192,
            batch: 0,
            micro_batch: 0,
            image_tokens: 0,
            gpu_layers: -1,
            flash_attention: true,
            projector_on_cpu: false,
            kv_cache: "q8_0".into(),
            temperature: 0.1,
            top_p: 0.9,
            seed: 42,
            request_timeout_secs: 180,
            allow_cpu_offload: false,
            require_acceleration: false,
            tesseract: if PathBuf::from("runtime/tesseract/tesseract.exe").is_file() {
                "runtime/tesseract/tesseract.exe".into()
            } else {
                "tesseract".into()
            },
            pdfium: "runtime".into(),
            server: crate::setup::saved_runtime(std::path::Path::new("runtime"))
                .map(|r| r.server)
                .unwrap_or_else(|| "runtime/llama-server.exe".into()),
            models_dir: "models".into(),
            endpoint: "http://127.0.0.1:18745".into(),
            external_server: false,
            subject: String::new(),
            writer: Default::default(),
            assistance: Default::default(),
            writer_reference: None,
            deskew: true,
            auto_rotate: true,
            preserve_figures: true,
            keep_page_breaks: true,
            font: "Calibri".into(),
            font_size: 11.0,
            ui: Appearance::default(),
            experience: ExperienceOptions::default(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExperienceOptions {
    pub live_preview: bool,
    pub check_updates: bool,
}
impl Default for ExperienceOptions {
    fn default() -> Self {
        Self {
            live_preview: true,
            check_updates: true,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Appearance {
    pub dark: bool,
    pub accent: [u8; 3],
    pub scale: f32,
    pub motion: f32,
    pub scan_animation: bool,
    pub intro_animation: bool,
    pub hover_effects: bool,
    pub transition_ms: u32,
    pub ambient_density: u8,
    pub idle_fps: u32,
    pub reduced_motion: bool,
    pub particles: bool,
    pub glow: bool,
    pub rounding: u8,
    pub fps: u32,
    pub compact: bool,
    pub background: [u8; 3],
    pub autosave: bool,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            dark: true,
            accent: [100, 226, 204],
            scale: 1.0,
            motion: 1.0,
            scan_animation: true,
            intro_animation: true,
            hover_effects: true,
            transition_ms: 220,
            ambient_density: 3,
            idle_fps: 30,
            reduced_motion: false,
            particles: true,
            glow: true,
            rounding: 16,
            fps: 60,
            compact: false,
            background: [12, 17, 28],
            autosave: true,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        let bad = |message| Err(Fault::new("NG102", message).into());
        if self.handwriting && self.preset == Preset::Fast {
            return bad(
                "Handwriting requires Balanced, Accuracy or Maximum Accuracy. Fast is printed OCR only.",
            );
        }
        if self.dpi != 0 && !(100..=600).contains(&self.dpi) {
            return bad("DPI must be 0 (auto) or 100–600");
        }
        if self.image_edge != 0 && !(512..=8192).contains(&self.image_edge) {
            return bad("Image edge must be auto or 512–8192");
        }
        if !(1_000_000..=40_000_000).contains(&self.max_pixels) {
            return bad("Image memory limit outside supported range");
        }
        if !(0.1..=0.98).contains(&self.ram_fraction) || !(0.1..=0.98).contains(&self.vram_fraction)
        {
            return bad("Memory fractions must be 0.1–0.98");
        }
        if !(0.0..=0.4).contains(&self.tile_overlap)
            || !(512..=4096).contains(&self.tile_edge)
            || self.crop_retries > 64
        {
            return bad("Invalid crop settings");
        }
        if !(0.5..=1.0).contains(&self.agreement_threshold)
            || !(0.0..=1.0).contains(&self.review_threshold)
        {
            return bad("Invalid review/consensus threshold");
        }
        if self.context != 0 && !(4096..=131072).contains(&self.context) {
            return bad("Context must be auto or 4096–131072");
        }
        if !(256..=32768).contains(&self.max_tokens)
            || self.cpu_threads > 512
            || self.batch > 8192
            || self.micro_batch > 2048
            || self.image_tokens > 16384
        {
            return bad("Invalid inference limits");
        }
        if self.consensus_min < 2
            || self.consensus_min > 8
            || self.gpu_layers < -1
            || !(0.0..=1.5).contains(&self.temperature)
            || !(0.0..=1.0).contains(&self.top_p)
        {
            return bad("Invalid inference/consensus configuration");
        }
        if !["f16", "q8_0", "q4_0"].contains(&self.kv_cache.as_str()) {
            return bad("Unsupported KV cache precision");
        }
        if !(5..=900).contains(&self.request_timeout_secs)
            || !(0.7..=2.0).contains(&self.ui.scale)
            || !(0.0..=3.0).contains(&self.ui.motion)
            || !(15..=144).contains(&self.ui.fps)
            || !(1..=60).contains(&self.ui.idle_fps)
            || self.ui.transition_ms > 1000
            || self.ui.ambient_density > 8
        {
            return bad("Invalid timeout or UI settings");
        }
        if !self
            .languages
            .split('+')
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        {
            return bad("Invalid Tesseract language codes");
        }
        if self.subject.len() > 8000
            || self.font.len() > 100
            || !(6.0..=32.0).contains(&self.font_size)
        {
            return bad("Invalid document settings");
        }
        if self.model_ids.len() > 32
            || self.model_ids.iter().any(|s| {
                s.is_empty()
                    || s.len() > 100
                    || !s
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            })
        {
            return bad("Invalid model allowlist");
        }
        self.assistance.validate()?;
        anyhow::ensure!(
            self.writer
                .id
                .as_deref()
                .is_none_or(crate::writers::valid_id),
            Fault::new("NG102", "Invalid writer profile ID")
        );
        local_endpoint(&self.endpoint)?;
        Ok(())
    }
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_secs)
    }
}
pub fn local_endpoint(endpoint: &str) -> Result<reqwest::Url> {
    let url =
        reqwest::Url::parse(endpoint).map_err(|_| Fault::new("NG203", "Invalid local endpoint"))?;
    let host = url.host_str().unwrap_or("").trim_matches(['[', ']']);
    let ip = host
        .parse::<std::net::IpAddr>()
        .map_err(|_| Fault::new("NG203", "Use a literal loopback IP"))?;
    anyhow::ensure!(
        url.scheme() == "http"
            && ip.is_loopback()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/",
        Fault::new("NG203", "Remote or ambiguous endpoint blocked")
    );
    Ok(url)
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct BoxRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}
impl BoxRect {
    pub fn valid(&self) -> bool {
        [self.left, self.top, self.right, self.bottom]
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && self.left < self.right
            && self.top < self.bottom
    }
    pub fn full() -> Self {
        Self {
            left: 0.,
            top: 0.,
            right: 1.,
            bottom: 1.,
        }
    }
    pub fn pixels(&self, w: u32, h: u32) -> (u32, u32, u32, u32) {
        if w == 0 || h == 0 {
            return (0, 0, 0, 0);
        }
        let x = (self.left * w as f32) as u32;
        let y = (self.top * h as f32) as u32;
        (
            x.min(w - 1),
            y.min(h - 1),
            ((self.right * w as f32).ceil() as u32).min(w).max(x + 1),
            ((self.bottom * h as f32).ceil() as u32).min(h).max(y + 1),
        )
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Merge {
    pub row: usize,
    pub col: usize,
    pub rowspan: usize,
    pub colspan: usize,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Block {
    pub kind: String,
    pub text: String,
    #[serde(rename = "box")]
    pub bbox: Option<BoxRect>,
    pub confidence: Option<f32>,
    pub rows: Vec<Vec<String>>,
    pub merges: Vec<Merge>,
    pub header_rows: usize,
    pub description: String,
    pub interpretation: String,
    pub uncertain: bool,
    pub alternatives: Vec<String>,
}
impl Block {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            kind: "text".into(),
            text: text.into(),
            ..Self::default()
        }
    }
    pub fn plain(&self) -> String {
        if self.kind == "table" {
            self.rows
                .iter()
                .map(|r| r.join("\t"))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            self.text.clone()
        }
    }
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            ["text", "heading", "list", "table", "figure", "equation"]
                .contains(&self.kind.as_str()),
            Fault::new("NG201", "Unknown block kind")
        );
        anyhow::ensure!(
            self.text.len() < 120000
                && self.description.len() < 24000
                && self.interpretation.len() < 24000
                && self.alternatives.len() <= 64
                && self.alternatives.iter().all(|s| s.len() < 120000)
                && self.rows.iter().flatten().map(String::len).sum::<usize>() < 2_000_000,
            Fault::new("NG201", "Oversized content")
        );
        if let Some(b) = &self.bbox {
            anyhow::ensure!(b.valid(), Fault::new("NG201", "Invalid region box"));
        }
        if let Some(c) = self.confidence {
            anyhow::ensure!(
                c.is_finite() && (0.0..=1.0).contains(&c),
                Fault::new("NG201", "Invalid confidence")
            );
        }
        if self.kind == "figure" {
            anyhow::ensure!(
                self.bbox.is_some(),
                Fault::new("NG201", "Figure requires a source box")
            );
        }
        if self.kind == "table" {
            let cols = self.rows.first().map_or(0, Vec::len);
            anyhow::ensure!(
                (1..=100).contains(&self.rows.len())
                    && (1..=40).contains(&cols)
                    && self.rows.iter().all(|r| r.len() == cols)
                    && self.header_rows <= self.rows.len(),
                Fault::new("NG201", "Invalid table dimensions")
            );
            let mut seen = std::collections::HashSet::new();
            for m in &self.merges {
                anyhow::ensure!(
                    m.rowspan > 0
                        && m.colspan > 0
                        && m.row
                            .checked_add(m.rowspan)
                            .is_some_and(|n| n <= self.rows.len())
                        && m.col.checked_add(m.colspan).is_some_and(|n| n <= cols),
                    Fault::new("NG201", "Invalid table merge")
                );
                for r in m.row..m.row + m.rowspan {
                    for c in m.col..m.col + m.colspan {
                        anyhow::ensure!(
                            seen.insert((r, c)),
                            Fault::new("NG201", "Overlapping table merges")
                        );
                    }
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PageContent {
    pub blocks: Vec<Block>,
    pub warnings: Vec<String>,
    pub summary: String,
}
impl PageContent {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.summary.len() <= 24000
                && self.warnings.len() <= 1000
                && self.warnings.iter().all(|v| v.len() <= 8000),
            Fault::new("NG201", "Oversized reader summary or warnings")
        );
        anyhow::ensure!(
            self.blocks.len() <= 500 && self.warnings.len() <= 200,
            Fault::new("NG201", "Excessive page data")
        );
        for b in &self.blocks {
            b.validate()?;
        }
        Ok(())
    }
    pub fn plain(&self) -> String {
        self.blocks
            .iter()
            .filter(|b| b.kind != "figure")
            .map(Block::plain)
            .collect::<Vec<_>>()
            .join("\n")
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    #[serde(default)]
    pub region: Option<BoxRect>,
    pub model_id: String,
    pub family: String,
    pub view: String,
    pub content: PageContent,
    pub reliability: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Page {
    pub edited: bool,
    pub page: usize,
    pub image: String,
    pub content: PageContent,
    pub candidates: Vec<Candidate>,
    pub source_warnings: Vec<String>,
    pub agreement: f64,
    pub reviewed: bool,
    pub seconds: f64,
}
impl Page {
    pub fn needs_review(&self, threshold: f32) -> bool {
        !self.reviewed
            && (self.agreement < 0.92
                || !self.content.warnings.is_empty()
                || !self.source_warnings.is_empty()
                || self.content.blocks.is_empty()
                || self.content.blocks.iter().any(|b| {
                    b.uncertain
                        || b.kind == "figure"
                        || b.confidence.is_none_or(|v| v < threshold)
                        || b.plain().to_lowercase().contains("[illegible]")
                }))
    }
}
pub fn pages_range(spec: &str, total: usize) -> Result<Vec<usize>> {
    if spec.trim().is_empty() {
        return Ok((1..=total).collect());
    }
    let mut out = std::collections::BTreeSet::new();
    for part in spec.split(',') {
        let parts = part.trim().split('-').collect::<Vec<_>>();
        anyhow::ensure!(
            (1..=2).contains(&parts.len()),
            Fault::new("NG100", "Invalid page range")
        );
        let a = parts[0].parse::<usize>()?;
        let b = parts.last().unwrap().parse::<usize>()?;
        anyhow::ensure!(
            a > 0 && a <= b && b <= total,
            Fault::new("NG100", "Page range outside document")
        );
        out.extend(a..=b);
    }
    Ok(out.into_iter().collect())
}
