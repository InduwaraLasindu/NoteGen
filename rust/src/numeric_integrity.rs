//! Bounded numeric review observations. No value is rewritten or inferred.
use crate::{
    document_tools::SourceRef,
    storage::{Control, Project, fingerprint},
    types::*,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

const LIMIT: usize = 1000;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NumericIssue {
    pub kind: String,
    pub token: String,
    pub source: SourceRef,
    pub message: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NumericReport {
    pub issues: Vec<NumericIssue>,
    pub inspected_blocks: usize,
    pub omitted_issues: usize,
    pub note: String,
}
fn grouped(value: &str, separator: char) -> bool {
    let mut chunks = value.split(separator);
    let Some(first) = chunks.next() else {
        return false;
    };
    (1..=3).contains(&first.len())
        && first.bytes().all(|b| b.is_ascii_digit())
        && chunks.all(|s| s.len() == 3 && s.bytes().all(|b| b.is_ascii_digit()))
}
fn ordinary_grouping(value: &str) -> bool {
    match (value.contains(','), value.contains('.')) {
        (true, false) => grouped(value, ','),
        (false, true) => grouped(value, '.'),
        (true, true) => {
            let decimal = if value.rfind('.') > value.rfind(',') {
                '.'
            } else {
                ','
            };
            let thousands = if decimal == '.' { ',' } else { '.' };
            let (whole, fraction) = value.rsplit_once(decimal).unwrap();
            !whole.contains(decimal)
                && !fraction.is_empty()
                && fraction.bytes().all(|b| b.is_ascii_digit())
                && grouped(whole, thousands)
        }
        _ => true,
    }
}
pub fn check(project: &Project, c: &Control) -> Result<NumericReport> {
    static DATES: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b([0-9]{4})-([0-9]{2})-([0-9]{2})\b|\b([0-9]{1,2})/([0-9]{1,2})/([0-9]{4})\b")
            .expect("fixed date pattern")
    });
    static SIGNS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?:^|[^0-9A-Za-z])([-+−]{2,}[0-9]+(?:[.,][0-9]+)?)")
            .expect("fixed sign pattern")
    });
    static PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b[0-9]+(?:[.,]{1,3}[0-9]+){2,}\b|\b[0-9]+[.,]{2,}[0-9]+\b")
            .expect("fixed punctuation pattern")
    });
    let mut report = NumericReport { note: "Review observations for ASCII numeric notation, ISO dates and slash-separated dates. Locale, identifiers, version numbers and code can explain these patterns. Nothing is corrected automatically. Unit and repeated-identifier variants are available in the consistency check.".into(), ..Default::default() };
    for n in &project.manifest.pages {
        c.check()?;
        let page = crate::redactions::analysis_page(project, &project.load(*n)?, c)?;
        for (index, block) in page.content.blocks.iter().enumerate() {
            c.check()?;
            report.inspected_blocks += 1;
            let text = block.plain();
            // Construct the source record only for retained observations.
            let mut push = |kind: &str, token: &str, message: &str| -> Result<()> {
                if report.issues.len() == LIMIT {
                    report.omitted_issues = report.omitted_issues.saturating_add(1);
                    return Ok(());
                }
                report.issues.push(NumericIssue {
                    kind: kind.into(),
                    token: token.chars().take(96).collect(),
                    source: SourceRef {
                        page: page.page,
                        block_index: index,
                        source_image: page.image.clone(),
                        region: block.bbox.clone(),
                        excerpt: text.chars().take(240).collect(),
                        block_fingerprint: fingerprint(block)?,
                    },
                    message: message.into(),
                });
                Ok(())
            };
            for date in DATES.captures_iter(&text) {
                let value = |i| {
                    date.get(i)
                        .and_then(|s| s.as_str().parse::<u32>().ok())
                        .unwrap_or(0)
                };
                let (year, month, day) = if date.get(1).is_some() {
                    (value(1), value(2), value(3))
                } else {
                    (value(6), value(5), value(4))
                };
                let valid =
                    |m, d| year > 0 && chrono::NaiveDate::from_ymd_opt(year as i32, m, d).is_some();
                if date.get(1).is_some() {
                    if !valid(month, day) {
                        push(
                            "invalid-calendar-date",
                            &date[0],
                            "The recognized ISO date is not a valid Gregorian calendar date. Compare each digit with the source.",
                        )?;
                    }
                } else if !valid(month, day) && !valid(day, month) {
                    push(
                        "invalid-calendar-date",
                        &date[0],
                        "Neither day/month/year nor month/day/year is a valid calendar date. Confirm the source and intended format.",
                    )?;
                } else if month != day && valid(month, day) && valid(day, month) {
                    push(
                        "ambiguous-date-order",
                        &date[0],
                        "Both day/month and month/day are possible. Confirm the intended date order; no locale is assumed.",
                    )?;
                }
            }
            for signs in SIGNS.captures_iter(&text) {
                push(
                    "repeated-sign",
                    &signs[1],
                    "Adjacent numeric signs may be transcription errors or intentional mathematical/code notation. Confirm them against the source.",
                )?;
            }
            for number in PUNCTUATION.find_iter(&text) {
                if !ordinary_grouping(number.as_str()) {
                    push(
                        "numeric-separators",
                        number.as_str(),
                        "Multiple separators do not match common three-digit grouping. Confirm the decimal/grouping marks, or whether this is an identifier or version.",
                    )?;
                }
            }
        }
    }
    Ok(report)
}
