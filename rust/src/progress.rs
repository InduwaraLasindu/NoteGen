//! Work-based estimates. Logs, token counters and download bytes never masquerade as pages.
use serde::{Deserialize, Serialize};
use std::{cell::Cell, collections::BTreeMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Render,
    Load,
    Read,
    Crop,
    Finalize,
    Export,
    Runtime,
    Download,
    Activate,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Work {
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
    pub unit_millis: u64,
    pub active: bool,
}
impl Work {
    pub fn new(phase: Phase, done: usize, total: usize, unit_millis: u64, active: bool) -> Self {
        Self {
            phase,
            done: done as u64,
            total: total as u64,
            unit_millis,
            active,
        }
    }
}
#[derive(Clone, Debug)]
struct Step {
    done: u64,
    total: u64,
    seconds: f64,
    updated: f64,
    sampled: bool,
    samples: u64,
    fastest: f64,
    slowest: f64,
}
/// A planning range from observed stage rates, not a calibrated confidence interval.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FinishWindow {
    pub earliest_seconds: f64,
    pub latest_seconds: f64,
    pub reasons: Vec<String>,
}
impl FinishWindow {
    pub fn scaled(&self, factor: f64) -> Option<Self> {
        if !factor.is_finite() || factor < 0. {
            return None;
        }
        let earliest_seconds = self.earliest_seconds * factor;
        let latest_seconds = self.latest_seconds * factor;
        (earliest_seconds.is_finite() && latest_seconds.is_finite()).then(|| Self {
            earliest_seconds,
            latest_seconds,
            reasons: self.reasons.clone(),
        })
    }
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub fraction: f32,
    pub remaining: Option<f64>,
    pub elapsed: f64,
    pub measured: bool,
}
#[derive(Clone, Debug)]
pub struct Estimate {
    steps: BTreeMap<Phase, Step>,
    active: Option<Phase>,
    start: f64,
    held: f64,
    suspended: Option<f64>,
    terminal: bool,
    succeeded: bool,
    floor: Cell<f32>,
}
impl Estimate {
    pub fn new(now: f64) -> Self {
        Self {
            steps: BTreeMap::new(),
            active: None,
            start: now,
            held: 0.,
            suspended: None,
            terminal: false,
            succeeded: false,
            floor: Cell::new(0.),
        }
    }
    fn time(&self, now: f64) -> f64 {
        (self.suspended.unwrap_or(now) - self.start - self.held).max(0.)
    }
    pub fn suspend(&mut self, now: f64, suspended: bool) {
        if suspended {
            self.suspended.get_or_insert(now);
        } else if let Some(start) = self.suspended.take() {
            self.held += (now - start).max(0.);
        }
    }
    pub fn update(&mut self, w: &Work, now: f64) {
        if self.terminal {
            return;
        }
        let time = self.time(now);
        let step = self.steps.entry(w.phase).or_insert(Step {
            done: 0,
            total: w.total,
            seconds: (w.unit_millis as f64 / 1000.).max(0.001),
            updated: time,
            sampled: false,
            samples: 0,
            fastest: 0.,
            slowest: 0.,
        });
        // Byte counters are intentionally not timed as single-unit operations.
        let next = w.done.min(w.total);
        if next > step.done && self.active == Some(w.phase) && time > step.updated {
            let rate = (time - step.updated) / (next - step.done) as f64;
            step.seconds = if step.sampled {
                0.65 * step.seconds + 0.35 * rate
            } else {
                rate
            };
            step.seconds = step.seconds.max(0.000_001);
            if !step.sampled {
                step.fastest = rate;
                step.slowest = rate;
            } else {
                step.fastest = step.fastest.min(rate);
                step.slowest = step.slowest.max(rate);
            }
            step.samples = step.samples.saturating_add(1);
            step.sampled = true;
        }
        step.done = next;
        step.total = w.total;
        step.updated = time;
        if w.active {
            self.active = Some(w.phase);
        } else if self.active == Some(w.phase) && next == w.total {
            self.active = None;
        }
    }
    pub fn finish(&mut self, now: f64, success: bool) {
        self.suspend(now, true);
        self.terminal = true;
        self.succeeded = success;
        if success {
            self.floor.set(1.);
        }
    }
    pub fn snapshot(&self, now: f64) -> Snapshot {
        let time = self.time(now);
        let mut complete = 0.;
        let mut remaining = 0.;
        for (phase, s) in &self.steps {
            complete += s.done as f64 * s.seconds;
            let left = s.total.saturating_sub(s.done) as f64;
            let working = if self.active == Some(*phase) && left > 0. && !self.terminal {
                (time - s.updated).max(0.).min(s.seconds * 0.85)
            } else {
                0.
            };
            complete += working;
            // A slower-than-predicted request increases ETA instead of counting down to zero.
            let overrun = if self.active == Some(*phase) && left > 0. {
                ((time - s.updated) - s.seconds).max(0.) * 0.5
            } else {
                0.
            };
            remaining += (left * s.seconds - working + overrun).max(0.);
        }
        let total = complete + remaining;
        let raw = if self.succeeded {
            1.
        } else if total > 0. {
            (complete / total).min(0.99) as f32
        } else {
            0.
        };
        let fraction = raw.max(self.floor.get());
        self.floor.set(fraction);
        Snapshot {
            fraction,
            remaining: if self.succeeded {
                Some(0.)
            } else if self.terminal || total == 0. || self.suspended.is_some() {
                None
            } else {
                Some(remaining.max(1.))
            },
            elapsed: time,
            measured: self.steps.values().any(|s| s.sampled),
        }
    }
    /// Account for per-stage variability and widen stages with fewer than three observations.
    /// New work, different pages or model loads can still exceed this planning range.
    pub fn window(&self, now: f64) -> Option<FinishWindow> {
        let remaining = self.snapshot(now).remaining?;
        if self.succeeded {
            return Some(FinishWindow { earliest_seconds: 0., latest_seconds: 0., reasons: vec![] });
        }
        let mut expected = 0.;
        let mut lower = 0.;
        let mut upper = 0.;
        let mut contributors = vec![];
        for (phase, s) in &self.steps {
            let left = s.total.saturating_sub(s.done) as f64;
            if left == 0. { continue; }
            let estimate = left * s.seconds;
            expected += estimate;
            let (lo, hi) = if s.samples >= 3 {
                (s.fastest.max(0.000_001) * 0.8, s.slowest.max(s.seconds) * 1.25)
            } else {
                (s.seconds * 0.5, s.slowest.max(s.seconds) * 2.5)
            };
            lower += left * lo;
            upper += left * hi;
            contributors.push((estimate, if s.samples == 0 {
                format!("{phase:?}: initial estimate; no completed timing samples")
            } else {
                format!("{phase:?}: {} timing samples, observed {:.2}–{:.2}s per unit{}",s.samples,s.fastest,s.slowest,
                    if s.samples < 3 { "; sparse observations widen range" } else { "" })
            }));
        }
        if expected <= 0. || !expected.is_finite() || !remaining.is_finite() {
            return None;
        }
        contributors.sort_by(|a,b| b.0.total_cmp(&a.0));
        Some(FinishWindow {
            earliest_seconds: remaining * (lower / expected).clamp(0.,1.),
            latest_seconds: remaining * (upper / expected).max(1.),
            reasons: contributors.into_iter().map(|(_,reason)|reason).collect(),
        })
    }
}

/// Weight documents by selected pages; preflight fills these counts without OCR.
#[derive(Clone, Debug, Default)]
pub struct Batch {
    pages: BTreeMap<usize, usize>,
    finished: BTreeMap<usize, bool>,
    floor: Cell<f32>,
}
impl Batch {
    pub fn new(jobs: impl IntoIterator<Item = (usize, usize)>) -> Self {
        Self {
            pages: jobs.into_iter().map(|(i, n)| (i, n.max(1))).collect(),
            ..Default::default()
        }
    }
    pub fn include(&mut self, index: usize) {
        self.pages.entry(index).or_insert(1);
    }
    pub fn set_pages(&mut self, jobs: impl IntoIterator<Item = (usize, usize)>) {
        for (i, n) in jobs {
            self.pages.insert(i, n.max(1));
        }
    }
    pub fn finish(&mut self, index: usize) {
        self.finished.insert(index, true);
    }
    pub fn snapshot(&self, index: usize, current: &Snapshot) -> Snapshot {
        let total: usize = self.pages.values().sum();
        let finished: usize = self
            .pages
            .iter()
            .filter(|(i, _)| self.finished.contains_key(i))
            .map(|(_, n)| *n)
            .sum();
        let pages = *self.pages.get(&index).unwrap_or(&1);
        let active = if self.finished.contains_key(&index) {
            0.
        } else {
            current.fraction as f64 * pages as f64
        };
        let success = !self.pages.is_empty() && self.finished.len() == self.pages.len();
        let fraction = if success {
            1.
        } else {
            ((finished as f64 + active) / total.max(1) as f64).min(0.99) as f32
        }
        .max(self.floor.get());
        self.floor.set(fraction);
        let future =
            total
                .saturating_sub(finished)
                .saturating_sub(if self.finished.contains_key(&index) {
                    0
                } else {
                    pages
                });
        let remaining = if success {
            Some(0.)
        } else {
            current.remaining.map(|r| {
                let duration = (current.elapsed + r) / pages as f64;
                r + future as f64 * duration
            })
        };
        Snapshot {
            fraction,
            remaining,
            ..current.clone()
        }
    }
}
pub fn duration(seconds: f64) -> String {
    let s = seconds.max(0.).ceil() as u64;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m", s / 3600, s / 60 % 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn estimates_learn_stage_cost_and_never_finish_early() {
        let mut e = Estimate::new(0.);
        e.update(&Work::new(Phase::Read, 0, 3, 10000, true), 0.);
        e.update(&Work::new(Phase::Export, 0, 1, 2000, false), 0.);
        e.update(&Work::new(Phase::Read, 1, 3, 10000, true), 20.);
        assert!(e.snapshot(20.).measured);
        assert!(e.snapshot(20.).remaining.unwrap() >= 42.);
        let old = e.snapshot(25.).fraction;
        e.update(&Work::new(Phase::Crop, 0, 10, 12000, true), 25.);
        assert!(e.snapshot(25.).fraction >= old);
        assert!(e.snapshot(9999.).fraction < 1.);
        e.finish(10000., true);
        assert_eq!(e.snapshot(10001.).fraction, 1.);
    }
    #[test]
    fn pauses_and_clarifications_are_excluded_and_failures_are_incomplete() {
        let mut e = Estimate::new(0.);
        e.update(&Work::new(Phase::Read, 0, 2, 10000, true), 0.);
        e.suspend(3., true);
        let held = e.snapshot(100.);
        assert_eq!(held.elapsed, 3.);
        assert!(held.remaining.is_none());
        e.suspend(103., false);
        assert_eq!(e.snapshot(105.).elapsed, 5.);
        e.finish(106., false);
        assert!(e.snapshot(900.).fraction < 1.);
        assert!(e.snapshot(900.).remaining.is_none());
    }
    #[test]
    fn queue_is_weighted_by_pages_and_requires_all_files_to_finish() {
        let mut b = Batch::new([(0, 1), (1, 99)]);
        b.finish(0);
        let s = Snapshot {
            fraction: 0.5,
            remaining: Some(50.),
            elapsed: 50.,
            measured: true,
        };
        assert!((b.snapshot(1, &s).fraction - 0.505).abs() < 0.001);
        b.finish(1);
        assert_eq!(b.snapshot(1, &s).fraction, 1.);
    }
    #[test]
    fn empty_plans_and_changed_counts_are_finite() {
        let mut e = Estimate::new(10.);
        assert!(e.snapshot(10.).remaining.is_none());
        e.update(&Work::new(Phase::Download, 50, 100, 1, true), 10.);
        let first = e.snapshot(11.).fraction;
        e.update(&Work::new(Phase::Download, 60, 1000, 1, true), 12.);
        assert!(e.snapshot(12.).fraction >= first);
        assert!(e.snapshot(12.).remaining.unwrap().is_finite());
    }
    #[test]
    fn planning_windows_show_sparse_stages_and_observed_variability() {
        let mut e=Estimate::new(0.);
        e.update(&Work::new(Phase::Read,0,10,10000,true),0.);
        e.update(&Work::new(Phase::Export,0,1,1000,false),0.);
        let initial=e.window(0.).unwrap();
        assert_eq!(initial.earliest_seconds,50.5);
        assert_eq!(initial.latest_seconds,252.5);
        assert!(initial.reasons[0].contains("initial"));
        for (done,now) in [(1,5.),(2,15.),(3,35.)] {
            e.update(&Work::new(Phase::Read,done,10,10000,true),now);
        }
        let measured=e.window(35.).unwrap();
        assert!(measured.earliest_seconds < e.snapshot(35.).remaining.unwrap());
        assert!(measured.latest_seconds > e.snapshot(35.).remaining.unwrap());
        assert!(measured.reasons[0].contains("3 timing samples"));
        assert!(measured.reasons[0].contains("5.00–20.00"));
        assert!(measured.reasons[1].contains("Export"));
    }
    #[test]
    fn planning_windows_pause_finish_and_scale_without_nonfinite_output() {
        let mut e=Estimate::new(0.);
        assert!(e.window(0.).is_none());
        e.update(&Work::new(Phase::Read,0,2,1000,true),0.);
        let w=e.window(0.).unwrap();
        assert_eq!(w.scaled(2.).unwrap().latest_seconds,w.latest_seconds*2.);
        assert!(w.scaled(f64::INFINITY).is_none());
        assert!(w.scaled(-1.).is_none());
        e.suspend(1.,true); assert!(e.window(10.).is_none());
        e.suspend(11.,false); assert!(e.window(12.).is_some());
        e.finish(13.,true); assert_eq!(e.window(13.).unwrap().latest_seconds,0.);
    }
}
