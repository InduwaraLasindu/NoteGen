use crate::{hardware::Hardware, types::*};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub family: String,
    pub role: String,
    pub prompt: String,
    pub resident_mib: u64,
    pub weights_mib: u64,
    pub rank: u32,
    pub source: String,
    pub released: String,
    #[serde(default)]
    pub weights: PathBuf,
    #[serde(default)]
    pub projector: PathBuf,
    #[serde(default)]
    pub sha256: String,
}
pub fn catalog() -> Vec<Model> {
    serde_json::from_str(include_str!("../catalog.json")).expect("compiled model catalog")
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub backend: Backend,
    pub gpu_index: Option<u32>,
    pub ram_budget_mib: u64,
    pub vram_budget_mib: u64,
    pub threads: usize,
    pub dpi: u32,
    pub image_edge: u32,
    pub context: u32,
    pub image_tokens: u32,
    pub batch: u32,
    pub micro_batch: u32,
    pub gpu_layers: i32,
    pub models: Vec<Model>,
    pub notes: Vec<String>,
}
pub fn build(hw: &Hardware, s: &Settings, models: &[Model]) -> Result<Plan> {
    s.validate()?;
    let gpu = if let Some(index) = s.gpu_index {
        hw.gpus.iter().find(|g| g.index == index)
    } else {
        hw.gpus.iter().max_by_key(|g| g.free_mib)
    };
    let backend = match s.backend {
        Backend::Auto => {
            if gpu.is_some() {
                if gpu.is_some_and(|g| g.backend == "vulkan") {
                    Backend::Vulkan
                } else {
                    Backend::Cuda
                }
            } else {
                Backend::Cpu
            }
        }
        b => b,
    };
    if s.require_acceleration && (backend == Backend::Cpu || gpu.is_none()) {
        return Err(Fault::new(
            "NG200",
            "Hardware acceleration requested but no compatible GPU was detected",
        )
        .into());
    }
    let ram_budget = ((hw.free_ram_mib as f64 * s.ram_fraction as f64) as u64)
        .min(hw.free_ram_mib.saturating_sub(s.reserve_ram_mib));
    anyhow::ensure!(
        ram_budget >= 768,
        Fault::new("NG300", "Insufficient free system RAM")
    );
    let vram_budget = if backend != Backend::Cpu {
        gpu.map_or(0, |g| {
            ((g.free_mib as f64 * s.vram_fraction as f64) as u64)
                .min(g.free_mib.saturating_sub(s.reserve_vram_mib))
        })
    } else {
        0
    };
    let mut fitting = models
        .iter()
        .filter(|m| {
            m.weights_mib + 512 <= ram_budget
                && if backend == Backend::Cpu {
                    m.weights_mib <= 6500
                } else {
                    m.resident_mib <= vram_budget
                        || s.allow_cpu_offload && m.weights_mib + 2048 <= ram_budget
                }
        })
        .cloned()
        .collect::<Vec<_>>();
    fitting.sort_by_key(|m| std::cmp::Reverse(m.rank));
    let mut chosen = vec![];
    if s.preset != Preset::Fast {
        if let Some(ocr) = fitting.iter().find(|m| m.role == "ocr") {
            chosen.push(ocr.clone());
        }
        if let Some(vision) = fitting.iter().find(|m| m.role == "vision") {
            chosen.push(vision.clone());
        }
        if s.preset == Preset::Maximum {
            let families = chosen.iter().map(|m| m.family.clone()).collect::<Vec<_>>();
            if let Some(other) = fitting.iter().find(|m| !families.contains(&m.family)) {
                chosen.push(other.clone());
            }
        }
    }
    let tier = match vram_budget {
        0..=3500 => 0,
        3501..=7500 => 1,
        7501..=15500 => 2,
        _ => 3,
    };
    let high = matches!(s.preset, Preset::Maximum);
    let choose = |value: u32, auto: u32| if value == 0 { auto } else { value };
    let mut notes=vec!["Memory estimates include weights, projector, context and workspace; runtime admission checks remain required.".into()];
    if chosen.len() < 2 && s.preset != Preset::Fast {
        notes.push("Insufficient compatible model diversity. Results will be flagged for review; no fictitious ensemble agreement.".into());
    }
    if backend == Backend::Cpu {
        notes.push("No NVIDIA GPU detected: CPU inference uses native SIMD. No GPU acceleration is claimed.".into());
    }
    Ok(Plan {
        backend,
        gpu_index: gpu.map(|g| g.index),
        ram_budget_mib: ram_budget,
        vram_budget_mib: vram_budget,
        threads: if s.cpu_threads == 0 {
            hw.logical_cores.saturating_sub(1).max(1)
        } else {
            s.cpu_threads.min(hw.logical_cores.max(1))
        },
        dpi: choose(s.dpi, if high { 400 } else { 300 }),
        image_edge: choose(s.image_edge, [1536, 2048, 2560, 3072][tier]),
        context: choose(
            s.context,
            if tier == 0 {
                12288
            } else if high {
                32768
            } else {
                16384
            },
        ),
        image_tokens: choose(s.image_tokens, [1536, 3072, 4096, 6144][tier]),
        batch: choose(s.batch, if tier == 0 { 256 } else { 1024 }),
        micro_batch: choose(s.micro_batch, [64, 128, 256, 512][tier]),
        gpu_layers: if backend == Backend::Cpu {
            0
        } else if s.gpu_layers < 0 {
            99
        } else {
            s.gpu_layers
        },
        models: chosen,
        notes,
    })
}
pub fn server_args(model: &Model, plan: &Plan, s: &Settings, port: u16) -> Vec<String> {
    let mut args = vec![
        "--model".into(),
        model.weights.to_string_lossy().into(),
        "--mmproj".into(),
        model.projector.to_string_lossy().into(),
        "--alias".into(),
        model.id.clone(),
        "--offline".into(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "--parallel".into(),
        "1".into(),
        "--n-gpu-layers".into(),
        plan.gpu_layers.to_string(),
        "--threads".into(),
        plan.threads.to_string(),
        "--threads-batch".into(),
        plan.threads.to_string(),
        "--ctx-size".into(),
        plan.context.to_string(),
        "--batch-size".into(),
        plan.batch.to_string(),
        "--ubatch-size".into(),
        plan.micro_batch.to_string(),
        "--image-max-tokens".into(),
        plan.image_tokens.to_string(),
        "--flash-attn".into(),
        if s.flash_attention { "on" } else { "off" }.into(),
        "--cache-ram".into(),
        "0".into(),
        "--cache-type-k".into(),
        s.kv_cache.clone(),
        "--cache-type-v".into(),
        s.kv_cache.clone(),
    ];
    if s.projector_on_cpu {
        args.push("--no-mmproj-offload".into());
    }
    if let Some(index) = plan.gpu_index {
        args.extend([
            "--device".into(),
            format!(
                "{}{index}",
                if plan.backend == Backend::Vulkan {
                    "Vulkan"
                } else {
                    "CUDA"
                }
            ),
        ]);
    }
    args
}
