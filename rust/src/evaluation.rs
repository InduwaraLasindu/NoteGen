//! Reproducible recognition scorecards. Corpus ownership and held-out splits are explicit.
use crate::{storage::read_json, types::*};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Corpus {
    pub schema: u32,
    pub version: String,
    pub permission: String,
    pub samples: Vec<Sample>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub id: String,
    pub writer: String,
    /// training, calibration, or test. Writers must not cross splits.
    pub split: String,
    pub reference: PageContent,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prediction {
    pub id: String,
    pub content: PageContent,
    /// Reader uncertainty, not assumed to be calibrated.
    pub uncertainty: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metric {
    pub errors: usize,
    pub reference_units: usize,
    pub predicted_units: usize,
    /// None means the reference contained no units of this type.
    pub error_rate: Option<f64>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Scorecard {
    pub corpus_version: String,
    pub split: String,
    pub samples: usize,
    pub characters: Metric,
    pub words: Metric,
    pub exact_numbers: Metric,
    pub table_cells: Metric,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CalibrationBin {
    pub lower: f64,
    pub upper: f64,
    pub samples: usize,
    pub erroneous_samples: usize,
    pub observed_error_rate: Option<f64>,
    pub wilson_lower: Option<f64>,
    pub wilson_upper: Option<f64>,
}

impl Corpus {
    pub fn read(path: &Path) -> Result<Self> {
        let corpus: Self = read_json(path, 32_000_000)?;
        corpus.validate()?;
        Ok(corpus)
    }
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.schema == 1 && !self.version.trim().is_empty() && self.version.len() <= 120,
            Fault::new("NG710", "Corpus needs schema 1 and a version")
        );
        anyhow::ensure!(
            !self.permission.trim().is_empty() && self.permission.len() <= 8000,
            Fault::new(
                "NG710",
                "Record permission or license before using a corpus"
            )
        );
        anyhow::ensure!(
            !self.samples.is_empty() && self.samples.len() <= 10000,
            Fault::new("NG710", "Corpus must contain 1–10000 samples")
        );
        let mut ids = BTreeSet::new();
        let mut writers = std::collections::BTreeMap::new();
        for s in &self.samples {
            anyhow::ensure!(
                !s.id.is_empty()
                    && s.id.len() <= 200
                    && ids.insert(&s.id)
                    && !s.writer.trim().is_empty()
                    && s.writer.len() <= 200
                    && ["training", "calibration", "test"].contains(&s.split.as_str()),
                Fault::new("NG710", "Invalid sample, duplicate ID or unknown split")
            );
            if let Some(split) = writers.insert(&s.writer, &s.split) {
                anyhow::ensure!(
                    split == &s.split,
                    Fault::new("NG710", "Writer leakage across corpus splits")
                );
            }
            s.reference.validate()?;
        }
        Ok(())
    }
}

/// Bounded Levenshtein distance over Unicode scalar values or tokens.
fn distance<T: PartialEq>(a: &[T], b: &[T]) -> Result<usize> {
    anyhow::ensure!(
        a.len().saturating_mul(b.len()) <= 25_000_000,
        Fault::new(
            "NG710",
            "Comparison exceeds the 25 million operation limit; split the sample"
        )
    );
    let (a, b) = if b.len() <= a.len() { (a, b) } else { (b, a) };
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (old + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(x != y));
            diagonal = old;
        }
    }
    Ok(row[b.len()])
}
fn numbers(s: &str) -> Vec<String> {
    // Preserve signs, decimal punctuation, dates and units. Never normalize numeric identity.
    s.split_whitespace()
        .filter(|t| t.chars().any(|c| c.is_ascii_digit()))
        .map(str::to_string)
        .collect()
}
fn cells(content: &PageContent) -> std::collections::BTreeMap<(usize, usize, usize), String> {
    let mut result = std::collections::BTreeMap::new();
    for (table, b) in content
        .blocks
        .iter()
        .filter(|b| b.kind == "table")
        .enumerate()
    {
        for (row, values) in b.rows.iter().enumerate() {
            for (column, value) in values.iter().enumerate() {
                result.insert((table, row, column), value.clone());
            }
        }
    }
    result
}
fn add<T: PartialEq>(m: &mut Metric, a: &[T], b: &[T]) -> Result<()> {
    m.errors += distance(a, b)?;
    m.reference_units += a.len();
    m.predicted_units += b.len();
    m.error_rate = (m.reference_units > 0).then(|| m.errors as f64 / m.reference_units as f64);
    Ok(())
}
fn matched<'a>(
    corpus: &'a Corpus,
    predictions: &'a [Prediction],
    split: &str,
) -> Result<Vec<(&'a Sample, &'a Prediction)>> {
    corpus.validate()?;
    anyhow::ensure!(
        ["training", "calibration", "test"].contains(&split),
        Fault::new("NG710", "Unknown split")
    );
    anyhow::ensure!(
        predictions.len() <= 10000,
        Fault::new("NG710", "Too many predictions")
    );
    let mut ids = BTreeSet::new();
    for p in predictions {
        anyhow::ensure!(
            ids.insert(&p.id)
                && p.uncertainty.is_finite()
                && (0.0..=1.0).contains(&p.uncertainty)
                && corpus.samples.iter().any(|s| s.id == p.id),
            Fault::new(
                "NG710",
                "Duplicate/unknown prediction or invalid uncertainty"
            )
        );
        p.content.validate()?;
    }
    let mut result = vec![];
    for s in corpus.samples.iter().filter(|s| s.split == split) {
        let p = predictions
            .iter()
            .find(|p| p.id == s.id)
            .ok_or_else(|| Fault::new("NG710", format!("Missing prediction for {}", s.id)))?;
        result.push((s, p));
    }
    anyhow::ensure!(
        !result.is_empty(),
        Fault::new("NG710", "Selected split has no samples")
    );
    Ok(result)
}
pub fn score(corpus: &Corpus, predictions: &[Prediction], split: &str) -> Result<Scorecard> {
    let pairs = matched(corpus, predictions, split)?;
    let mut score = Scorecard {
        corpus_version: corpus.version.clone(),
        split: split.into(),
        samples: pairs.len(),
        ..Default::default()
    };
    for (s, p) in pairs {
        let a = s.reference.plain();
        let b = p.content.plain();
        add(
            &mut score.characters,
            &a.chars().collect::<Vec<_>>(),
            &b.chars().collect::<Vec<_>>(),
        )?;
        add(
            &mut score.words,
            &a.split_whitespace().collect::<Vec<_>>(),
            &b.split_whitespace().collect::<Vec<_>>(),
        )?;
        add(&mut score.exact_numbers, &numbers(&a), &numbers(&b))?;
        let a = cells(&s.reference);
        let b = cells(&p.content);
        let keys: BTreeSet<_> = a.keys().chain(b.keys()).collect();
        score.table_cells.errors += keys
            .iter()
            .filter(|key| a.get(**key) != b.get(**key))
            .count();
        score.table_cells.reference_units += a.len();
        score.table_cells.predicted_units += b.len();
        score.table_cells.error_rate = (score.table_cells.reference_units > 0)
            .then(|| score.table_cells.errors as f64 / score.table_cells.reference_units as f64);
    }
    Ok(score)
}
/// Empirical bins from a calibration split only. The 95% interval communicates sparse evidence.
pub fn calibrate(corpus: &Corpus, predictions: &[Prediction]) -> Result<Vec<CalibrationBin>> {
    let pairs = matched(corpus, predictions, "calibration")?;
    let mut bins: Vec<_> = (0..10)
        .map(|i| CalibrationBin {
            lower: i as f64 / 10.,
            upper: (i + 1) as f64 / 10.,
            samples: 0,
            erroneous_samples: 0,
            observed_error_rate: None,
            wilson_lower: None,
            wilson_upper: None,
        })
        .collect();
    for (s, p) in pairs {
        let bin = &mut bins[((p.uncertainty * 10.).floor() as usize).min(9)];
        bin.samples += 1;
        bin.erroneous_samples += usize::from(
            s.reference.plain() != p.content.plain() || cells(&s.reference) != cells(&p.content),
        );
    }
    for b in &mut bins {
        if b.samples == 0 {
            continue;
        }
        let n = b.samples as f64;
        let p = b.erroneous_samples as f64 / n;
        let z = 1.96_f64;
        let denominator = 1. + z * z / n;
        let center = (p + z * z / (2. * n)) / denominator;
        let half = z * ((p * (1. - p) / n + z * z / (4. * n * n)).sqrt()) / denominator;
        b.observed_error_rate = Some(p);
        b.wilson_lower = Some((center - half).max(0.));
        b.wilson_upper = Some((center + half).min(1.));
    }
    Ok(bins)
}
