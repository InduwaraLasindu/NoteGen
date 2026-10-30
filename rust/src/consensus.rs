use crate::types::*;
use std::collections::{HashMap, HashSet};
use unicode_normalization::UnicodeNormalization;

pub fn normalize(s: &str) -> String {
    s.nfkc()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn similarity(a: &str, b: &str) -> f64 {
    strsim::normalized_levenshtein(&normalize(a), &normalize(b))
}
pub fn select(candidates: &[Candidate], minimum: usize, threshold: f64) -> (PageContent, f64) {
    if candidates.is_empty() {
        return (
            PageContent {
                warnings: vec!["No recognizer returned usable content.".into()],
                ..Default::default()
            },
            0.,
        );
    }
    // Each independent family gets one vote. Repeated crops/seeds do not become extra voters.
    let mut best: HashMap<&str, &Candidate> = HashMap::new();
    for c in candidates {
        if best
            .get(c.family.as_str())
            .is_none_or(|old| c.reliability > old.reliability)
        {
            best.insert(&c.family, c);
        }
    }
    let mut voters = best.values().copied().collect::<Vec<_>>();
    voters.sort_by(|a, b| a.model_id.cmp(&b.model_id));
    let mut selected = 0;
    let mut best_score = -1.;
    for (i, c) in voters.iter().enumerate() {
        let score = voters
            .iter()
            .map(|other| {
                similarity(&c.content.plain(), &other.content.plain())
                    * other.reliability.clamp(0.1, 1.)
            })
            .sum::<f64>()
            / voters
                .iter()
                .map(|v| v.reliability.clamp(0.1, 1.))
                .sum::<f64>();
        if score > best_score
            || (score == best_score && c.reliability > voters[selected].reliability)
        {
            selected = i;
            best_score = score;
        }
    }
    let chosen = voters[selected];
    let mut result = chosen.content.clone();
    let agreement = if voters.len() < minimum {
        0.
    } else {
        voters
            .iter()
            .filter(|v| v.model_id != chosen.model_id)
            .map(|v| similarity(&chosen.content.plain(), &v.content.plain()))
            .sum::<f64>()
            / (voters.len() - 1).max(1) as f64
    };
    if voters.len() < minimum {
        result.warnings.push(format!(
            "Only {} independent recognizer families; {} required for consensus.",
            voters.len(),
            minimum
        ));
    }
    if agreement < threshold {
        result.warnings.push("Independent readers disagree. Selected an existing reading; alternatives are preserved for review.".into());
        for b in &mut result.blocks {
            b.uncertain = true;
        }
    }
    let numbers = |text: &str| -> HashSet<String> {
        text.split_whitespace()
            .filter(|t| t.chars().any(|c| c.is_ascii_digit()))
            .map(|t| {
                t.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != ',')
                    .to_owned()
            })
            .collect()
    };
    let expected = numbers(&chosen.content.plain());
    if voters
        .iter()
        .any(|v| numbers(&v.content.plain()) != expected)
    {
        result.warnings.push("Numeric disagreement: check every conflicting amount, date, unit or equation against the scan.".into());
    }
    for b in &mut result.blocks {
        if b.plain().to_lowercase().contains("[illegible]") {
            b.uncertain = true;
        }
    }
    (result, agreement)
}
