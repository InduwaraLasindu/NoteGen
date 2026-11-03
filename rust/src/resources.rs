//! Reversible application budgets. These are scheduling limits, not OS power settings.
use crate::{hardware::Hardware, planning, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerProfile {
    Quiet,
    Battery,
    Balanced,
    Throughput,
}
impl PowerProfile {
    pub fn label(self) -> &'static str {
        match self {
            Self::Quiet => "Quiet",
            Self::Battery => "Battery",
            Self::Balanced => "Balanced",
            Self::Throughput => "Maximum throughput",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Reservations {
    pub cpu_cores: usize,
    pub ram_mib: Option<u64>,
    pub vram_mib: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceValues {
    pub cpu_threads: usize,
    pub ram_fraction: f32,
    pub vram_fraction: f32,
    pub reserve_ram_mib: u64,
    pub reserve_vram_mib: u64,
    pub fps: u32,
    pub idle_fps: u32,
}
impl ResourceValues {
    fn validate(&self) -> Result<()> {
        // Keep persisted previews subject to the same numeric bounds as ordinary settings.
        let mut candidate = Settings::default();
        self.write(&mut candidate);
        candidate.validate()
    }
    fn capture(s: &Settings) -> Self {
        Self {
            cpu_threads: s.cpu_threads,
            ram_fraction: s.ram_fraction,
            vram_fraction: s.vram_fraction,
            reserve_ram_mib: s.reserve_ram_mib,
            reserve_vram_mib: s.reserve_vram_mib,
            fps: s.ui.fps,
            idle_fps: s.ui.idle_fps,
        }
    }
    fn write(&self, s: &mut Settings) {
        s.cpu_threads = self.cpu_threads;
        s.ram_fraction = self.ram_fraction;
        s.vram_fraction = self.vram_fraction;
        s.reserve_ram_mib = self.reserve_ram_mib;
        s.reserve_vram_mib = self.reserve_vram_mib;
        s.ui.fps = self.fps;
        s.ui.idle_fps = self.idle_fps;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourcePreview {
    pub profile: PowerProfile,
    pub before: ResourceValues,
    pub after: ResourceValues,
    pub ram_budget_mib: u64,
    pub vram_budget_mib: u64,
    pub notes: Vec<String>,
}
impl ResourcePreview {
    /// Reject stale previews and preserve unrelated edits made after the preview.
    pub fn apply(&self, s: &mut Settings) -> Result<()> {
        self.replace(s, &self.before, &self.after)
    }
    pub fn undo(&self, s: &mut Settings) -> Result<()> {
        self.replace(s, &self.after, &self.before)
    }
    fn replace(
        &self,
        s: &mut Settings,
        expected: &ResourceValues,
        next: &ResourceValues,
    ) -> Result<()> {
        // Validate both directions before mutating anything, including the future undo state.
        self.before.validate()?;
        self.after.validate()?;
        anyhow::ensure!(
            ResourceValues::capture(s) == *expected,
            Fault::new(
                "NG720",
                "Resource settings changed; create a new preview before applying or undoing"
            )
        );
        let mut candidate = s.clone();
        next.write(&mut candidate);
        candidate.validate()?;
        next.write(s);
        Ok(())
    }
}

pub fn preview(
    s: &Settings,
    hw: &Hardware,
    profile: PowerProfile,
    reserve: &Reservations,
) -> Result<ResourcePreview> {
    s.validate()?;
    anyhow::ensure!(
        hw.logical_cores > 0 && reserve.cpu_cores < hw.logical_cores,
        Fault::new(
            "NG720",
            "Reserve fewer CPU cores than detected; NoteGen needs at least one"
        )
    );
    let available = hw
        .logical_cores
        .saturating_sub(reserve.cpu_cores)
        .clamp(1, 512);
    let (threads, fraction, fps, idle) = match profile {
        PowerProfile::Quiet => ((available / 2).max(1), 0.70, 30, 5),
        PowerProfile::Battery => (available.min(2), 0.60, 20, 2),
        PowerProfile::Balanced => (available.saturating_sub(1).max(1), 0.85, 60, 15),
        PowerProfile::Throughput => (available, 0.95, 60, 15),
    };
    let before = ResourceValues::capture(s);
    let after = ResourceValues {
        cpu_threads: threads,
        ram_fraction: fraction,
        vram_fraction: fraction,
        reserve_ram_mib: reserve.ram_mib.unwrap_or(s.reserve_ram_mib),
        reserve_vram_mib: reserve.vram_mib.unwrap_or(s.reserve_vram_mib),
        fps,
        idle_fps: idle,
    };
    let mut proposed = s.clone();
    after.write(&mut proposed);
    let plan = planning::build(hw, &proposed, &[])?;
    Ok(ResourcePreview {
        profile,
        before,
        after,
        ram_budget_mib: plan.ram_budget_mib,
        vram_budget_mib: plan.vram_budget_mib,
        notes: vec![
            "Limits apply to NoteGen workers, model admission and interface refresh; OS power settings are unchanged.".into(),
            "Lower worker and memory budgets can increase conversion time. Actual power, temperature and quality must be measured separately.".into(),
            "Reservations are admission headroom, not hard operating-system memory quotas. Active jobs retain the plan they started with.".into(),
        ],
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueDuration {
    pub id: String,
    pub seconds: Option<f64>,
    pub measured: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct JobForecast {
    pub id: String,
    pub starts_after_seconds: Option<f64>,
    pub finishes_after_seconds: Option<f64>,
    pub measured: bool,
    pub within_budget: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct TimeBudgetPlan {
    pub budget_seconds: f64,
    pub jobs: Vec<JobForecast>,
    pub unresolved: Vec<String>,
    pub notes: Vec<String>,
}
/// Preserve queue order. An unknown duration prevents promising finish times for later jobs.
pub fn time_budget(jobs: &[QueueDuration], budget_seconds: f64) -> Result<TimeBudgetPlan> {
    anyhow::ensure!(
        budget_seconds.is_finite()
            && (0. ..=31_536_000.).contains(&budget_seconds)
            && jobs.len() <= 100_000,
        Fault::new(
            "NG720",
            "Time budget must be finite and between zero and one year; queue limit is 100000"
        )
    );
    let mut seen = BTreeSet::new();
    let mut elapsed = Some(0.);
    let mut forecasts = Vec::with_capacity(jobs.len());
    let mut unresolved = vec![];
    for job in jobs {
        anyhow::ensure!(
            !job.id.trim().is_empty()
                && job.id.len() <= 500
                && seen.insert(&job.id)
                && job
                    .seconds
                    .is_none_or(|v| v.is_finite() && (0. ..=31_536_000.).contains(&v)),
            Fault::new(
                "NG720",
                "Queue durations need unique nonempty IDs and finite nonnegative durations up to one year"
            )
        );
        let start = elapsed;
        elapsed = start.zip(job.seconds).map(|(a, b)| a + b);
        if job.seconds.is_none() || !job.measured {
            unresolved.push(job.id.clone());
        }
        forecasts.push(JobForecast {
            id: job.id.clone(),
            starts_after_seconds: start,
            finishes_after_seconds: elapsed,
            measured: job.measured && job.seconds.is_some(),
            within_budget: elapsed.is_some_and(|v| v <= budget_seconds),
        });
    }
    Ok(TimeBudgetPlan {
        budget_seconds,
        jobs: forecasts,
        unresolved,
        notes: vec!["Forecast keeps queue order and assumes the supplied durations and current settings remain representative. Unmeasured estimates are listed as unresolved; this is not a deadline guarantee.".into()],
    })
}
