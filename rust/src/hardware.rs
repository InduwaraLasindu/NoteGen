use serde::{Deserialize, Serialize};
use std::process::Command;
use sysinfo::System;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Gpu {
    pub index: u32,
    pub name: String,
    pub uuid: String,
    pub total_mib: u64,
    pub free_mib: u64,
    pub backend: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Hardware {
    pub cpu: String,
    pub logical_cores: usize,
    pub physical_cores: usize,
    pub ram_mib: u64,
    pub free_ram_mib: u64,
    pub gpus: Vec<Gpu>,
    pub simd: Vec<String>,
}
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
    }
    c
}
pub fn parse_nvidia(text: &str) -> Vec<Gpu> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    rdr.records()
        .filter_map(|r| {
            let r = r.ok()?;
            Some(Gpu {
                index: r.get(0)?.parse().ok()?,
                name: r.get(1)?.into(),
                uuid: r.get(2)?.into(),
                total_mib: r.get(3)?.parse().ok()?,
                free_mib: r.get(4)?.parse().ok()?,
                backend: "cuda".into(),
            })
        })
        .collect()
}
pub fn detect() -> Hardware {
    let s = System::new_all();
    let mut gpus = command("nvidia-smi")
        .args([
            "--query-gpu=index,name,uuid,memory.total,memory.free",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_nvidia(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    gpus.sort_by_key(|g| std::cmp::Reverse(g.free_mib));
    let mut simd = vec![];
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            simd.push("AVX2".into());
        }
        if std::is_x86_feature_detected!("fma") {
            simd.push("FMA".into());
        }
        if std::is_x86_feature_detected!("avx512f") {
            simd.push("AVX512".into());
        }
    }
    Hardware {
        cpu: s
            .cpus()
            .first()
            .map(|c| c.brand().to_owned())
            .unwrap_or_default(),
        logical_cores: s.cpus().len().max(1),
        physical_cores: System::physical_core_count().unwrap_or(1),
        ram_mib: s.total_memory() / 1048576,
        free_ram_mib: s.available_memory() / 1048576,
        gpus,
        simd,
    }
}

pub fn parse_devices(text: &str) -> Vec<Gpu> {
    let re = regex::Regex::new(
        r"(?im)^\s*(CUDA|Vulkan)(\d+):\s*(.*?)\s*\((\d+)\s*MiB,\s*(\d+)\s*MiB\s*free\)",
    )
    .unwrap();
    re.captures_iter(text)
        .filter_map(|c| {
            Some(Gpu {
                index: c[2].parse().ok()?,
                name: c[3].into(),
                uuid: format!("{}{}", &c[1], &c[2]),
                total_mib: c[4].parse().ok()?,
                free_mib: c[5].parse().ok()?,
                backend: c[1].to_lowercase(),
            })
        })
        .collect()
}
pub fn detect_for(s: &crate::types::Settings, c: &crate::storage::Control) -> Hardware {
    let mut h = detect();
    if s.server.is_file()
        && let Ok((true, a, b)) = crate::scan::run(
            &s.server,
            &["--list-devices".into()],
            std::time::Duration::from_secs(20),
            c,
        )
    {
        let mut devices = parse_devices(&format!("{a}\n{b}"));
        if !devices.is_empty() {
            if s.backend == crate::types::Backend::Vulkan {
                devices.retain(|g| g.backend == "vulkan");
            } else if devices.iter().any(|g| g.backend == "cuda") {
                devices.retain(|g| g.backend == "cuda");
            }
            h.gpus = devices;
        }
    }
    h
}
