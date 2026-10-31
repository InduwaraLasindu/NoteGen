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

pub fn overlap(a: &BoxRect, b: &BoxRect) -> f32 {
    let intersection = (a.right.min(b.right) - a.left.max(b.left)).max(0.)
        * (a.bottom.min(b.bottom) - a.top.max(b.top)).max(0.);
    let area = (a.right - a.left) * (a.bottom - a.top) + (b.right - b.left) * (b.bottom - b.top)
        - intersection;
    if area > 0. { intersection / area } else { 0. }
}
pub fn refine_regions(page: &mut Page, s: &Settings) {
    for block in &mut page.content.blocks {
        if block.kind == "figure" {
            continue;
        }
        if block.bbox.is_none() {
            let mut found = None;
            let mut score = 0.5;
            for c in page.candidates.iter().filter(|c| c.view == "full") {
                for other in &c.content.blocks {
                    if other.bbox.is_some() && other.kind == block.kind {
                        let value = similarity(&block.plain(), &other.plain());
                        if value > score {
                            found = other.bbox.clone();
                            score = value;
                        }
                    }
                }
            }
            block.bbox = found;
        }
        let Some(bbox) = &block.bbox else {
            continue;
        };
        let mut readings = vec![];
        for c in page
            .candidates
            .iter()
            .filter(|c| c.region.as_ref().is_some_and(|r| overlap(r, bbox) > 0.65))
        {
            let relevant = c
                .content
                .blocks
                .iter()
                .filter(|b| {
                    b.kind != "figure" && b.bbox.as_ref().is_none_or(|r| overlap(r, bbox) > 0.35)
                })
                .cloned()
                .collect::<Vec<_>>();
            if !relevant.is_empty() {
                readings.push(Candidate {
                    region: None,
                    model_id: c.model_id.clone(),
                    family: c.family.clone(),
                    view: "full".into(),
                    content: PageContent {
                        blocks: relevant,
                        ..Default::default()
                    },
                    reliability: c.reliability,
                });
            }
        }
        let families = readings.iter().map(|c| &c.family).collect::<HashSet<_>>();
        if families.len() < s.consensus_min {
            continue;
        }
        let (result, agreement) = select(&readings, s.consensus_min, s.agreement_threshold);
        let text = result.plain();
        if text.len() < block.plain().len() / 3 || text.len() > block.plain().len().max(20) * 4 {
            continue;
        }
        block.alternatives = readings
            .iter()
            .map(|r| format!("{}: {}", r.model_id, r.content.plain()))
            .collect();
        if agreement >= s.agreement_threshold {
            if block.kind == "table" {
                if let Some(table) = result.blocks.iter().find(|b| b.kind == "table") {
                    let bbox = block.bbox.clone();
                    let alternatives = block.alternatives.clone();
                    *block = table.clone();
                    block.bbox = bbox;
                    block.alternatives = alternatives;
                }
            } else {
                block.text = text;
            }
        }
        block.uncertain = agreement < s.agreement_threshold
            || result.warnings.iter().any(|w| w.contains("Numeric"));
        if !result.warnings.is_empty() {
            page.content.warnings.extend(result.warnings);
        }
    }
}
pub fn select(candidates: &[Candidate], minimum: usize, threshold: f64) -> (PageContent, f64) {
    if !candidates.iter().any(|c| c.view == "full") {
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
    for c in candidates.iter().filter(|c| c.view == "full") {
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
    // Align structures one-to-one per reader. Similar tables at different locations
    // must not collapse, and a grounded reader can supply a missing source box.
    for candidate in &voters {
        let mut assigned = HashSet::new();
        for block in candidate
            .content
            .blocks
            .iter()
            .filter(|b| b.kind == "table" || b.kind == "figure")
        {
            let matched = result.blocks.iter().enumerate().position(|(index, b)| {
                !assigned.contains(&index)
                    && b.kind == block.kind
                    && match b.bbox.as_ref().zip(block.bbox.as_ref()) {
                        Some((a, b)) => overlap(a, b) > 0.35,
                        None => b.kind == "table" && similarity(&b.plain(), &block.plain()) > 0.85,
                    }
            });
            if let Some(index) = matched {
                assigned.insert(index);
                if result.blocks[index].bbox.is_none() {
                    result.blocks[index].bbox = block.bbox.clone();
                }
            } else {
                assigned.insert(result.blocks.len());
                result.blocks.push(block.clone());
                result.warnings.push("A reader omitted a structured region; another reader's table/figure was retained for verification.".into());
            }
        }
    }
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
                    .trim_matches(['.', ','])
                    .to_owned()
            })
            .filter(|t| !t.is_empty())
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
