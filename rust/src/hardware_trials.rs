//! Explicit trials on one copied source page. Timing and reference quality stay separate.
//! Memory figures are sampled highwater observations, not instantaneous process peaks.
//! NVIDIA process memory can be unavailable, including Windows WDDM:
//! https://docs.nvidia.com/deploy/nvidia-smi/#processes
use crate::{
    hardware::{self, Hardware},
    planning, runtime,
    storage::{Activity, Control, Project, fingerprint, hash_file},
    types::*,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, System,
};

fn fault(message: &str) -> anyhow::Error {
    Fault::new("NG720", message).into()
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeviceRequest {
    pub backend: Backend,
    pub gpu_index: Option<u32>,
}
impl DeviceRequest {
    pub fn cpu() -> Self {
        Self {
            backend: Backend::Cpu,
            gpu_index: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Device {
    pub request: DeviceRequest,
    pub name: String,
    pub runtime_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub page: usize,
    pub reader: Option<String>,
    pub devices: Vec<DeviceRequest>,
    pub repetitions: usize,
    pub timeout_secs: u64,
    pub sample_interval_ms: u64,
    pub reference: Option<PageContent>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            page: 1,
            reader: None,
            devices: vec![DeviceRequest::cpu()],
            repetitions: 2,
            timeout_secs: 180,
            sample_interval_ms: 500,
            reference: None,
        }
    }
}
impl Options {
    pub fn validate(&self, p: &Project) -> Result<()> {
        anyhow::ensure!(
            p.manifest.pages.contains(&self.page)
                && (1..=8).contains(&self.devices.len())
                && (1..=3).contains(&self.repetitions)
                && (5..=900).contains(&self.timeout_secs)
                && (250..=5000).contains(&self.sample_interval_ms)
                && self.reader.as_deref().is_none_or(crate::writers::valid_id),
            fault(
                "Choose a project page, 1–8 devices, 1–3 repetitions, 5–900 second timeout and 250–5,000 ms sampling interval"
            )
        );
        anyhow::ensure!(
            self.reader.is_some() || self.devices.iter().all(|d| *d == DeviceRequest::cpu()),
            fault("Printed OCR is a CPU trial. Choose a managed AI reader to compare GPU devices")
        );
        if let Some(reference) = &self.reference {
            reference.validate()?;
        }
        Ok(())
    }
}
/// Validate selections against the actual runtime's detected compatible device list.
pub fn select_devices(h: &Hardware, requests: &[DeviceRequest]) -> Result<Vec<Device>> {
    anyhow::ensure!(
        (1..=8).contains(&requests.len()),
        fault("Select 1–8 explicit devices")
    );
    let mut seen = BTreeSet::new();
    let mut result = vec![];
    for request in requests {
        anyhow::ensure!(
            seen.insert(format!("{:?}:{:?}", request.backend, request.gpu_index)),
            fault("A trial device was selected twice")
        );
        if request.backend == Backend::Cpu {
            anyhow::ensure!(
                request.gpu_index.is_none(),
                fault("CPU trials cannot select a GPU index")
            );
            result.push(Device {
                request: request.clone(),
                name: h.cpu.clone(),
                runtime_id: "CPU".into(),
            });
            continue;
        }
        anyhow::ensure!(
            [Backend::Cuda, Backend::Vulkan].contains(&request.backend),
            fault("Choose CPU, CUDA or Vulkan explicitly for a reproducible trial")
        );
        let index = request
            .gpu_index
            .ok_or_else(|| fault("GPU trials require an explicit detected device index"))?;
        let backend = if request.backend == Backend::Cuda {
            "cuda"
        } else {
            "vulkan"
        };
        let gpu = h
            .gpus
            .iter()
            .find(|gpu| gpu.index == index && gpu.backend == backend)
            .ok_or_else(|| fault("The selected GPU is not exposed by the installed runtime"))?;
        result.push(Device {
            request: request.clone(),
            name: gpu.name.clone(),
            runtime_id: format!(
                "{}{index}",
                if request.backend == Backend::Cuda {
                    "CUDA"
                } else {
                    "Vulkan"
                }
            ),
        });
    }
    Ok(result)
}
pub fn discover(s: &Settings, c: &Control) -> Result<Hardware> {
    c.check()?;
    let mut system = System::new();
    system.refresh_memory_specifics(MemoryRefreshKind::everything());
    system.refresh_cpu_specifics(CpuRefreshKind::everything());
    let mut h = Hardware {
        cpu: system
            .cpus()
            .first()
            .map(|cpu| cpu.brand().to_string())
            .unwrap_or_default(),
        logical_cores: system.cpus().len().max(1),
        physical_cores: System::physical_core_count().unwrap_or(1),
        ram_mib: system.total_memory() / 1048576,
        free_ram_mib: system.available_memory() / 1048576,
        gpus: vec![],
        simd: vec![],
    };
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            h.simd.push("AVX2".into());
        }
        if std::is_x86_feature_detected!("fma") {
            h.simd.push("FMA".into());
        }
    }
    if let Ok((true, stdout, stderr)) = crate::scan::run(
        &s.server,
        &["--list-devices".into()],
        Duration::from_secs(20),
        c,
    ) {
        h.gpus = hardware::parse_devices(&format!("{stdout}\n{stderr}"));
    }
    c.check()?;
    Ok(h)
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VramProcess {
    pub pid: u32,
    pub gpu_uuid: String,
    pub used_mib: Option<u64>,
}
/// Parse --query-compute-apps=pid,gpu_uuid,used_gpu_memory --format=csv,noheader,nounits.
/// Missing/unsupported values remain unknown, including WDDM's N/A.
pub fn parse_nvidia_processes(text: &str) -> Result<Vec<VramProcess>> {
    anyhow::ensure!(
        text.len() <= 1_000_000,
        fault("NVIDIA process metrics exceed 1 MB")
    );
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    let mut result = vec![];
    let mut seen = BTreeSet::new();
    for record in reader.records() {
        let record = record?;
        anyhow::ensure!(
            record.len() == 3 && result.len() < 4096,
            fault("Unexpected NVIDIA process metrics columns or row count")
        );
        let pid = record[0].parse::<u32>()?;
        let uuid = record[1].to_string();
        let raw = record[2].trim();
        let used_mib = if ["N/A", "[N/A]", "[Not Supported]", "Not Supported", "-"].contains(&raw) {
            None
        } else {
            Some(raw.parse::<u64>()?)
        };
        anyhow::ensure!(
            pid > 0
                && !uuid.is_empty()
                && uuid.len() <= 200
                && seen.insert((pid, uuid.clone()))
                && used_mib.is_none_or(|n| n <= 32_000_000),
            fault("Invalid or duplicate NVIDIA process metrics")
        );
        result.push(VramProcess {
            pid,
            gpu_uuid: uuid,
            used_mib,
        });
    }
    Ok(result)
}
pub fn process_vram(rows: &[VramProcess], pid: u32) -> Result<(Option<u64>, Vec<String>)> {
    let selected = rows.iter().filter(|r| r.pid == pid).collect::<Vec<_>>();
    if selected.is_empty() {
        return Ok((None, vec![]));
    }
    let uuids = selected.iter().map(|r| r.gpu_uuid.clone()).collect();
    let mut total = 0u64;
    for row in selected {
        let Some(value) = row.used_mib else {
            return Ok((None, uuids));
        };
        total = total
            .checked_add(value)
            .ok_or_else(|| fault("Process VRAM sample overflow"))?;
    }
    Ok((Some(total), uuids))
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MemorySample {
    pub elapsed_ms: u64,
    pub app_rss_bytes: Option<u64>,
    pub runtime_rss_bytes: Option<u64>,
    pub runtime_vram_mib: Option<u64>,
    pub gpu_uuids: Vec<String>,
    pub vram_query_performed: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MemorySummary {
    pub samples: Vec<MemorySample>,
    pub requested_interval_ms: u64,
    pub app_rss_highwater_bytes: Option<u64>,
    pub runtime_rss_highwater_bytes: Option<u64>,
    pub runtime_vram_highwater_mib: Option<u64>,
    pub truncated: bool,
    pub notes: Vec<String>,
}
pub fn summarize(samples: Vec<MemorySample>, interval: u64, truncated: bool) -> MemorySummary {
    MemorySummary{app_rss_highwater_bytes:samples.iter().filter_map(|s|s.app_rss_bytes).max(),runtime_rss_highwater_bytes:samples.iter().filter_map(|s|s.runtime_rss_bytes).max(),runtime_vram_highwater_mib:samples.iter().filter_map(|s|s.runtime_vram_mib).max(),samples,requested_interval_ms:interval,truncated,notes:vec!["Periodic RSS/VRAM highwater observations are lower bounds on instantaneous peaks. Startup allocations and unobserved intervals are not measured.".into(),"RSS is resident process memory and may include shared pages. OCR child-process RSS is not included; only NoteGen and an owned managed AI runtime are sampled.".into(),"NVIDIA process VRAM is queried at most once per second and can be unavailable, including Windows WDDM. Missing values are unknown, never zero estimates.".into()]}
}
struct Collector {
    system: System,
    runtime: Option<u32>,
    runtime_start: Option<u64>,
    gpu: bool,
    started: Instant,
    last_vram: Option<Instant>,
    notes: Vec<String>,
    query_enabled: bool,
}
impl Collector {
    fn collect(&mut self, stop: &Control, query_vram: bool) -> MemorySample {
        let mut pids = vec![Pid::from_u32(std::process::id())];
        if let Some(pid) = self.runtime {
            pids.push(Pid::from_u32(pid));
        }
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing().with_memory().without_tasks(),
        );
        let app = self
            .system
            .process(Pid::from_u32(std::process::id()))
            .map(|p| p.memory())
            .filter(|memory| *memory > 0);
        let runtime = self
            .runtime
            .and_then(|pid| self.system.process(Pid::from_u32(pid)))
            .and_then(|process| {
                let start = process.start_time();
                if let Some(previous) = self.runtime_start
                    && previous != start
                {
                    return None;
                }
                self.runtime_start = Some(start);
                (process.memory() > 0).then_some(process.memory())
            });
        let mut sample = MemorySample {
            elapsed_ms: self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            app_rss_bytes: app,
            runtime_rss_bytes: runtime,
            ..Default::default()
        };
        if self.gpu
            && self.query_enabled
            && query_vram
            && self.runtime.is_some()
            && runtime.is_some()
            && self
                .last_vram
                .is_none_or(|last| last.elapsed() >= Duration::from_secs(1))
        {
            self.last_vram = Some(Instant::now());
            sample.vram_query_performed = true;
            let result = (|| -> Result<(Option<u64>, Vec<String>)> {
                let (ok, text, _) = crate::scan::run(
                    Path::new("nvidia-smi"),
                    &[
                        "--query-compute-apps=pid,gpu_uuid,used_gpu_memory".into(),
                        "--format=csv,noheader,nounits".into(),
                    ],
                    Duration::from_secs(2),
                    stop,
                )?;
                anyhow::ensure!(ok, fault("NVIDIA process metrics unavailable"));
                process_vram(
                    &parse_nvidia_processes(&text)?,
                    self.runtime.unwrap_or_default(),
                )
            })();
            match result {
                Ok((memory, uuids)) => {
                    sample.runtime_vram_mib = memory;
                    sample.gpu_uuids = uuids;
                }
                Err(_) => {
                    self.query_enabled = false;
                    if !stop.cancelled.load(Ordering::Relaxed) {
                        self.notes.push("nvidia-smi did not provide usable process memory; this trial has no NVIDIA VRAM observation.".into());
                    }
                }
            }
        }
        sample
    }
}
pub struct MemorySampler {
    stop: Arc<(Mutex<bool>, Condvar)>,
    control: Control,
    thread: Option<thread::JoinHandle<MemorySummary>>,
}
impl MemorySampler {
    pub fn start(runtime: Option<u32>, gpu: bool, interval_ms: u64) -> Result<Self> {
        anyhow::ensure!(
            (250..=5000).contains(&interval_ms),
            fault("Sampling interval must be 250–5,000 ms")
        );
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let signal = stop.clone();
        let control = Control::default();
        let worker_control = control.clone();
        let mut collector = Collector {
            system: System::new(),
            runtime,
            runtime_start: None,
            gpu,
            started: Instant::now(),
            last_vram: None,
            notes: vec![],
            query_enabled: true,
        };
        let initial = collector.collect(&control, false);
        let handle = thread::spawn(move || {
            let mut samples = vec![initial];
            let mut truncated = false;
            loop {
                if worker_control.cancelled.load(Ordering::Relaxed) {
                    break;
                }
                if samples.len() >= 4096 {
                    truncated = true;
                    break;
                }
                samples.push(collector.collect(&worker_control, true));
                let (lock, wake) = &*signal;
                let stopped = lock.lock().unwrap_or_else(|e| e.into_inner());
                if *stopped {
                    break;
                }
                let (stopped, _) = wake
                    .wait_timeout(stopped, Duration::from_millis(interval_ms))
                    .unwrap_or_else(|e| e.into_inner());
                if *stopped {
                    break;
                }
            }
            let mut summary = summarize(samples, interval_ms, truncated);
            summary.notes.extend(collector.notes);
            summary
        });
        Ok(Self {
            stop,
            control,
            thread: Some(handle),
        })
    }
    fn stop(&self) {
        self.control.cancelled.store(true, Ordering::Relaxed);
        let (lock, wake) = &*self.stop;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
        wake.notify_all();
    }
    pub fn finish(mut self) -> MemorySummary {
        self.stop();
        self.thread
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_else(|| MemorySummary {
                notes: vec![
                    "Memory sampling stopped unexpectedly; metrics are unavailable.".into(),
                ],
                ..Default::default()
            })
    }
}
impl Drop for MemorySampler {
    fn drop(&mut self) {
        self.stop();
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BudgetSuggestion {
    pub runtime_ram_mib: Option<u64>,
    pub vram_mib: Option<u64>,
    pub note: String,
}
pub fn conservative_budget(
    original_ram_mib: u64,
    original_vram_mib: u64,
    memory: &MemorySummary,
) -> BudgetSuggestion {
    let margin =
        |mib: u64, reserve: u64| mib.saturating_add(mib.div_ceil(4)).saturating_add(reserve);
    BudgetSuggestion{runtime_ram_mib:memory.runtime_rss_highwater_bytes.map(|bytes|original_ram_mib.max(margin(bytes.div_ceil(1048576),512))),vram_mib:memory.runtime_vram_highwater_mib.map(|mib|original_vram_mib.max(margin(mib,256))),note:"Optional non-decreasing margin over sampled highwater. This is not a validated peak requirement; model admission must still check current free memory and preserve existing headroom.".into()}
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trial {
    pub device: Device,
    pub repetition: usize,
    pub elapsed_seconds: f64,
    pub startup_seconds: Option<f64>,
    pub read_seconds: Option<f64>,
    pub success: bool,
    pub acceleration_verified: bool,
    pub managed_pid: Option<u32>,
    pub memory: MemorySummary,
    pub review_regions: usize,
    pub content: Option<PageContent>,
    pub scorecard: Option<crate::evaluation::Scorecard>,
    pub quality_error: Option<String>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceSummary {
    pub device: Device,
    pub successes: usize,
    pub failures: usize,
    pub median_seconds: Option<f64>,
    pub output_varied: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub schema: u32,
    pub created: String,
    pub page: usize,
    pub source_sha256: String,
    pub copied_source: bool,
    pub image_size: [u32; 2],
    pub reader: Option<String>,
    pub reader_revision: Option<String>,
    pub reader_sha256: Option<String>,
    pub runtime_sha256: Option<String>,
    pub reference_digest: Option<String>,
    pub writer_reference_digest: Option<String>,
    pub settings_digest: String,
    pub hardware: Hardware,
    pub trials: Vec<Trial>,
    pub devices: Vec<DeviceSummary>,
    pub recommended: Option<DeviceRequest>,
    pub reference_supplied: bool,
    pub notes: Vec<String>,
}
pub fn device_summaries(trials: &[Trial], devices: &[Device]) -> Result<Vec<DeviceSummary>> {
    let mut summaries = vec![];
    for device in devices {
        let selected = trials
            .iter()
            .filter(|t| t.device.request == device.request)
            .collect::<Vec<_>>();
        let mut times = selected
            .iter()
            .filter(|t| t.success && t.elapsed_seconds.is_finite() && t.elapsed_seconds > 0.0)
            .map(|t| t.elapsed_seconds)
            .collect::<Vec<_>>();
        times.sort_by(f64::total_cmp);
        let hashes = selected
            .iter()
            .filter_map(|t| t.content.as_ref())
            .map(fingerprint)
            .collect::<Result<BTreeSet<_>>>()?;
        let median = if times.is_empty() {
            None
        } else {
            Some(if times.len().is_multiple_of(2) {
                (times[times.len() / 2 - 1] + times[times.len() / 2]) / 2.0
            } else {
                times[times.len() / 2]
            })
        };
        summaries.push(DeviceSummary {
            device: device.clone(),
            successes: times.len(),
            failures: selected.len() - times.len(),
            median_seconds: median,
            output_varied: hashes.len() > 1,
        });
    }
    Ok(summaries)
}
struct Deadline {
    done: Arc<AtomicBool>,
    control: Control,
    thread: Option<thread::JoinHandle<()>>,
}
impl Deadline {
    fn start(parent: &Control, seconds: u64) -> Self {
        let control = Control {
            paused: parent.paused.clone(),
            ..Default::default()
        };
        let done = Arc::new(AtomicBool::new(false));
        let closed = done.clone();
        let inner = control.clone();
        let outer = parent.cancelled.clone();
        let handle = thread::spawn(move || {
            let start = Instant::now();
            while !closed.load(Ordering::Relaxed) {
                if outer.load(Ordering::Relaxed) || start.elapsed() >= Duration::from_secs(seconds)
                {
                    inner.cancelled.store(true, Ordering::Relaxed);
                    break;
                }
                thread::sleep(Duration::from_millis(30));
            }
        });
        Self {
            done,
            control,
            thread: Some(handle),
        }
    }
}
impl Drop for Deadline {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
struct PreviewGuard<'a>(&'a Activity);
impl Drop for PreviewGuard<'_> {
    fn drop(&mut self) {
        self.0.preview(crate::telemetry::Preview::clear());
    }
}
fn quality(
    reference: &PageContent,
    predicted: &PageContent,
) -> Result<crate::evaluation::Scorecard> {
    crate::evaluation::score(
        &crate::evaluation::Corpus {
            schema: 1,
            version: "one-page-hardware-trial-v1".into(),
            permission: "Reference explicitly supplied for this local source-page trial.".into(),
            samples: vec![crate::evaluation::Sample {
                id: "source-page".into(),
                writer: "explicit-reference".into(),
                split: "test".into(),
                reference: reference.clone(),
            }],
        },
        &[crate::evaluation::Prediction {
            id: "source-page".into(),
            content: predicted.clone(),
            uncertainty: 1.0,
        }],
        "test",
    )
}
/// Work on a verified copy of one saved source page; project recognition/settings remain untouched.
pub fn run(
    project: &Project,
    options: &Options,
    settings: &Settings,
    c: &Control,
    log: &Activity,
) -> Result<Report> {
    options.validate(project)?;
    settings.validate()?;
    c.check()?;
    let source = project.load(options.page)?;
    let source_path = crate::word::safe_image(&project.root, &source.image)?;
    anyhow::ensure!(
        fs::metadata(&source_path)?.len() <= 320_000_000,
        fault("Source page PNG exceeds the 320 MB trial-copy limit")
    );
    let source_hash = hash_file(&source_path, c)?;
    let source_identity = crate::redactions::source_identity(project, &source, c)?;
    let work = tempfile::tempdir()?;
    let copy = work.path().join("source-page.png");
    if source.confirmed_redactions.is_empty() {
        fs::copy(&source_path, &copy)?;
        anyhow::ensure!(
            hash_file(&copy, c)? == source_hash,
            fault("Source changed while copying; retry the trial")
        );
    } else {
        let (_, image, _) = crate::redactions::effective_page(project, &source, c)?;
        image.save(&copy)?;
        anyhow::ensure!(
            hash_file(&source_path, c)? == source_hash,
            fault("Source changed while masking the trial")
        );
    }
    let image = crate::writers::open_image(&copy, u64::from(settings.max_pixels))?;
    let h = discover(settings, c)?;
    let devices = select_devices(&h, &options.devices)?;
    let mut common =
        crate::redactions::recognition_settings(settings, &source.confirmed_redactions)?;
    common.backend = Backend::Cpu;
    common.gpu_index = None;
    common.require_acceleration = false;
    common.external_server = false;
    let base = planning::build(&h, &common, &[])?;
    common.cpu_threads = base.threads.min(512);
    common.image_edge = base.image_edge;
    common.context = base.context;
    common.image_tokens = base.image_tokens;
    common.batch = base.batch;
    common.micro_batch = base.micro_batch;
    common.automatic_models = false;
    common.model_ids = options.reader.iter().cloned().collect();
    common.preset = Preset::Accuracy;
    if common.writer.use_examples
        && common.writer_reference.is_none()
        && let Some(id) = common.writer.id.as_deref()
    {
        common.writer_reference = crate::writers::reference(&common.writer.root, id)?;
    }
    let model = if let Some(id) = &options.reader {
        let mut model = runtime::inventory(&common.models_dir)?
            .into_iter()
            .find(|m| &m.id == id)
            .ok_or_else(|| fault("The trial reader is not installed"))?;
        anyhow::ensure!(
            model.endpoint.is_none(),
            fault(
                "Device trials require an owned managed reader; external services cannot verify the selected device"
            )
        );
        let digest = fingerprint(&(
            hash_file(&model.weights, c)?,
            hash_file(&model.projector, c)?,
        ))?;
        anyhow::ensure!(
            model.sha256.is_empty() || model.sha256 == digest,
            fault("Trial reader checksums changed; repair or register the model again")
        );
        model.sha256 = digest;
        Some(model)
    } else {
        None
    };
    let executable = if model.is_some() {
        &common.server
    } else {
        &common.tesseract
    };
    let runtime_hash = if executable.is_file() {
        Some(hash_file(executable, c)?)
    } else {
        None
    };
    let mut report=Report{schema:1,created:chrono::Utc::now().to_rfc3339(),page:options.page,source_sha256:source_hash,copied_source:true,image_size:[image.width(),image.height()],reader:options.reader.clone(),reader_revision:model.as_ref().and_then(|m|(!m.revision.is_empty()).then(||m.revision.clone())),reader_sha256:model.as_ref().map(|m|m.sha256.clone()),runtime_sha256:runtime_hash,reference_digest:options.reference.as_ref().map(fingerprint).transpose()?,writer_reference_digest:common.writer_reference.as_ref().map(|r|fingerprint(&(&r.description,&r.png))).transpose()?,settings_digest:fingerprint(&common)?,hardware:h.clone(),trials:vec![],devices:vec![],recommended:None,reference_supplied:options.reference.is_some(),notes:vec!["All trials use the same verified copied source pixels, reader revision and fixed inference profile. Only backend/device and necessary CPU/GPU offload differ.".into(),"Each AI repetition starts a new managed runtime; operating-system disk caches are not flushed. Startup/read times are separated, and reference evaluation is outside the timer.".into(),"A single-page timing and successful requests do not establish broad recognition quality or hardware stability. Review flags are uncalibrated.".into()]};
    if options.reference.is_none() {
        report.notes.push("No reference was supplied. Accuracy is unmeasured; only runtime success, elapsed time, source output and review flags are reported.".into());
    }
    let _preview = PreviewGuard(log);
    for device in &devices {
        for repetition in 1..=options.repetitions {
            c.check()?;
            let deadline = Deadline::start(c, options.timeout_secs);
            let inner = &deadline.control;
            let started = Instant::now();
            let mut trial = Trial {
                device: device.clone(),
                repetition,
                elapsed_seconds: 0.0,
                startup_seconds: None,
                read_seconds: None,
                success: false,
                acceleration_verified: false,
                managed_pid: None,
                memory: MemorySummary::default(),
                review_regions: 0,
                content: None,
                scorecard: None,
                quality_error: None,
                error: None,
            };
            let mut selected = common.clone();
            selected.backend = device.request.backend;
            selected.gpu_index = device.request.gpu_index;
            selected.require_acceleration = device.request.backend != Backend::Cpu;
            log.emit(
                "hardware-trial",
                format!(
                    "Copied page {} · {} · repetition {repetition}/{}",
                    options.page, device.runtime_id, options.repetitions
                ),
                options.page,
                report.trials.len(),
                devices.len() * options.repetitions,
                "",
            );
            let outcome = (|| -> Result<PageContent> {
                if selected.experience.live_preview {
                    log.preview(crate::telemetry::Preview::source(
                        copy.clone(),
                        options.page,
                        options.reader.as_deref().unwrap_or("Printed OCR"),
                        None,
                    ));
                }
                if let Some(model) = &model {
                    let plan = planning::build(&h, &selected, std::slice::from_ref(model))?;
                    anyhow::ensure!(
                        plan.models.iter().any(|m| m.id == model.id),
                        fault(
                            "The identical reader/profile does not fit this device's current memory budget"
                        )
                    );
                    let startup = Instant::now();
                    let server =
                        runtime::Server::start(model, &plan, &selected, inner, log, work.path())?;
                    trial.startup_seconds = Some(startup.elapsed().as_secs_f64());
                    trial.managed_pid = server.managed_pid();
                    trial.acceleration_verified = server.acceleration_verified;
                    let sampler = MemorySampler::start(
                        trial.managed_pid,
                        selected.backend != Backend::Cpu,
                        options.sample_interval_ms,
                    )?;
                    let read = Instant::now();
                    let result = if source.confirmed_redactions.is_empty() {
                        server.read(
                            &image,
                            "",
                            &selected.subject,
                            &selected,
                            inner,
                            log,
                            options.page,
                        )
                    } else {
                        server.read_masked(
                            &image,
                            &source_identity,
                            &source.confirmed_redactions,
                            None,
                            &selected,
                            inner,
                            log,
                            options.page,
                        )
                    };
                    let result = result.and_then(|mut content| {
                        crate::geometry::bind_reader_content(&mut content, &source_identity)?;
                        Ok(content)
                    });
                    trial.read_seconds = Some(read.elapsed().as_secs_f64());
                    trial.elapsed_seconds = started.elapsed().as_secs_f64();
                    trial.memory = sampler.finish();
                    result
                } else {
                    let sampler = MemorySampler::start(None, false, options.sample_interval_ms)?;
                    let read = Instant::now();
                    let result =
                        crate::scan::printed(&image, &selected, inner).and_then(|mut content| {
                            crate::geometry::bind_content(&mut content, &source_identity)?;
                            crate::redactions::enforce_view(
                                &content,
                                &source_identity,
                                &source.confirmed_redactions,
                                None,
                                inner,
                            )
                            .map(|(content, _)| content)
                        });
                    trial.read_seconds = Some(read.elapsed().as_secs_f64());
                    trial.elapsed_seconds = started.elapsed().as_secs_f64();
                    trial.memory = sampler.finish();
                    result
                }
            })();
            if trial.elapsed_seconds == 0.0 {
                trial.elapsed_seconds = started.elapsed().as_secs_f64();
            }
            let deadline_cancelled = inner.cancelled.load(Ordering::Relaxed);
            drop(deadline);
            c.check()?;
            match outcome {
                Ok(content) => {
                    content.validate()?;
                    trial.success = true;
                    trial.review_regions = content
                        .blocks
                        .iter()
                        .filter(|b| {
                            b.uncertain
                                || b.kind == "figure"
                                || b.confidence.is_none_or(|v| v < settings.review_threshold)
                                || b.plain().to_lowercase().contains("[illegible]")
                        })
                        .count();
                    if let Some(reference) = &options.reference {
                        match quality(reference, &content) {
                            Ok(score) => trial.scorecard = Some(score),
                            Err(e) => trial.quality_error = Some(e.to_string()),
                        }
                    }
                    trial.content = Some(content);
                }
                Err(error) => {
                    trial.error = Some(if deadline_cancelled {
                        format!(
                            "Trial exceeded its {} second limit or was cancelled: {error}",
                            options.timeout_secs
                        )
                    } else {
                        error.to_string()
                    });
                }
            }
            log.emit("hardware-memory",format!("{} · sampled runtime RSS {:?} bytes · sampled NVIDIA process VRAM {:?} MiB · {} observations",device.runtime_id,trial.memory.runtime_rss_highwater_bytes,trial.memory.runtime_vram_highwater_mib,trial.memory.samples.len()),options.page,report.trials.len()+1,devices.len()*options.repetitions,"");
            report.trials.push(trial);
        }
    }
    if let Some(model) = &model {
        let digest = fingerprint(&(
            hash_file(&model.weights, c)?,
            hash_file(&model.projector, c)?,
        ))?;
        anyhow::ensure!(
            digest == model.sha256,
            fault("Reader files changed during the comparison; results are not comparable")
        );
    }
    if let Some(expected) = &report.runtime_sha256 {
        anyhow::ensure!(
            hash_file(executable, c)? == *expected,
            fault("Runtime executable changed during the comparison; repeat with a fixed version")
        );
    }
    report.devices = device_summaries(&report.trials, &devices)?;
    let outputs = report
        .trials
        .iter()
        .filter_map(|t| t.content.as_ref())
        .map(fingerprint)
        .collect::<Result<BTreeSet<_>>>()?;
    if report.devices.len() > 1
        && outputs.len() == 1
        && report
            .devices
            .iter()
            .all(|d| d.successes == options.repetitions && d.failures == 0 && !d.output_varied)
    {
        report.recommended = report
            .devices
            .iter()
            .filter_map(|d| d.median_seconds.map(|seconds| (d, seconds)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(d, _)| d.device.request.clone());
    } else if report.devices.len() > 1 {
        report.notes.push("No automatic device recommendation: some requests failed or observed reader outputs differed. Compare source quality and per-device results before choosing.".into());
    }
    Ok(report)
}
