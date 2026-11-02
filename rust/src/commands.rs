//! Explicit local tools. JSON previews can be inspected before a separate apply command.
use clap::Subcommand;
use notegen::{
    evaluation, exports, inputs, preflight, profile_tools, projects, review, storage::*, types::*,
};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum Action {
    /// Export markdown, html, latex, xlsx or traceability JSON.
    Export {
        project: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        format: String,
    },
    /// Measure CER, WER, exact-number and table-cell errors on a versioned corpus.
    Score {
        corpus: PathBuf,
        predictions: PathBuf,
        #[arg(long, default_value = "test")]
        split: String,
    },
    /// Compute empirical uncertainty bins only from the calibration split.
    Calibrate {
        corpus: PathBuf,
        predictions: PathBuf,
    },
    /// Inspect one source page without OCR or model downloads.
    Preflight {
        input: PathBuf,
        #[arg(long, default_value_t = 1)]
        page: usize,
    },
    Undo {
        project: PathBuf,
    },
    Redo {
        project: PathBuf,
    },
    Timeline {
        project: PathBuf,
    },
    Restore {
        project: PathBuf,
        revision: u64,
    },
    Issues {
        project: PathBuf,
    },
    PreviewRule {
        project: PathBuf,
        rule: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    ApplyRule {
        project: PathBuf,
        preview: PathBuf,
        #[arg(long, default_value = "Confirmed replacement rule")]
        label: String,
    },
    Catalog {
        #[arg(required = true)]
        projects: Vec<PathBuf>,
        #[arg(long, default_value = "")]
        query: String,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        status: Option<String>,
    },
    Search {
        query: String,
        #[arg(required = true)]
        projects: Vec<PathBuf>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    Duplicates {
        #[arg(required = true)]
        projects: Vec<PathBuf>,
    },
    Recovery {
        project: PathBuf,
    },
    /// Print metadata, or apply a reviewed metadata JSON file.
    Metadata {
        project: PathBuf,
        #[arg(long)]
        apply: Option<PathBuf>,
    },
    AttachReference {
        project: PathBuf,
        input: PathBuf,
        #[arg(long)]
        label: String,
        #[arg(long, default_value = "background")]
        role: String,
    },
    SaveTemplate {
        name: String,
        #[arg(short, long)]
        output: PathBuf,
    },
    LoadTemplate {
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    ProfileCoverage {
        writer: String,
    },
    ProfileExercises {
        writer: String,
    },
    GlossarySuggestions {
        writer: String,
    },
    Quarantine {
        writer: String,
        sample: String,
        #[arg(long)]
        reason: String,
    },
    RestoreExample {
        writer: String,
        sample: String,
    },
    ExampleConditions {
        writer: String,
        sample: String,
        conditions: Vec<String>,
    },
    ProfileSnapshot {
        writer: String,
        name: String,
    },
    ProfileVersions {
        writer: String,
    },
    CompareProfiles {
        writer: String,
        left: String,
        right: String,
    },
    BranchProfile {
        writer: String,
        name: String,
        #[arg(long)]
        version: Option<String>,
    },
    PreviewProfileShare {
        writer: String,
        selection: PathBuf,
    },
    ExportProfileSelection {
        writer: String,
        selection: PathBuf,
        #[arg(long)]
        preview_token: String,
        #[arg(short, long)]
        output: PathBuf,
    },
    FontProof {
        writer: String,
        #[arg(short, long)]
        output: PathBuf,
    },
}
fn print<T: Serialize>(value: T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
fn format(value: &str) -> Result<exports::Format> {
    Ok(match value {
        "markdown" | "md" => exports::Format::Markdown,
        "html" => exports::Format::Html,
        "latex" | "tex" => exports::Format::Latex,
        "xlsx" => exports::Format::Spreadsheet,
        "traceability" | "json" => exports::Format::Traceability,
        _ => {
            return Err(Fault::new(
                "NG102",
                "Format must be markdown, html, latex, xlsx or traceability",
            )
            .into());
        }
    })
}
pub fn run(action: Action, s: &Settings, c: &Control) -> Result<()> {
    match action {
        Action::Export {
            project,
            output,
            format: kind,
        } => exports::export(&Project::read(project)?, &output, format(&kind)?, c),
        Action::Score {
            corpus,
            predictions,
            split,
        } => print(evaluation::score(
            &evaluation::Corpus::read(&corpus)?,
            &read_json::<Vec<evaluation::Prediction>>(&predictions, 32_000_000)?,
            &split,
        )?),
        Action::Calibrate {
            corpus,
            predictions,
        } => print(evaluation::calibrate(
            &evaluation::Corpus::read(&corpus)?,
            &read_json::<Vec<evaluation::Prediction>>(&predictions, 32_000_000)?,
        )?),
        Action::Preflight { input, page } => {
            let mut result = None;
            inputs::render(
                &input,
                &s.pdfium,
                &page.to_string(),
                150,
                s.max_pixels,
                c,
                |_, im| {
                    result = Some(preflight::analyze(&im));
                    Ok(())
                },
            )?;
            print(result)
        }
        Action::Undo { project } => print(
            review::undo(&Project::read(project)?)?
                .iter()
                .map(|p| p.page)
                .collect::<Vec<_>>(),
        ),
        Action::Redo { project } => print(
            review::redo(&Project::read(project)?)?
                .iter()
                .map(|p| p.page)
                .collect::<Vec<_>>(),
        ),
        Action::Timeline { project } => print(review::timeline(&Project::read(project)?)?),
        Action::Restore { project, revision } => print(
            review::restore(&Project::read(project)?, revision)?
                .iter()
                .map(|p| p.page)
                .collect::<Vec<_>>(),
        ),
        Action::Issues { project } => print(review::issues(&Project::read(project)?)?),
        Action::PreviewRule {
            project,
            rule,
            output,
        } => {
            let preview =
                review::preview_rule(&Project::read(project)?, &read_json(&rule, 1_000_000)?)?;
            anyhow::ensure!(
                !output.exists(),
                Fault::new("NG401", "Preview output already exists")
            );
            atomic_json(&output, &preview)?;
            print(preview)
        }
        Action::ApplyRule {
            project,
            preview,
            label,
        } => print(review::apply_rule(
            &Project::read(project)?,
            &read_json(&preview, 32_000_000)?,
            &label,
        )?),
        Action::Catalog {
            projects: roots,
            query,
            tag,
            status,
        } => print(projects::catalog(
            &roots,
            &projects::Filter {
                query,
                tag,
                status,
                ..Default::default()
            },
        )?),
        Action::Search {
            query,
            projects: roots,
            limit,
        } => print(projects::search(&roots, &query, limit)?),
        Action::Duplicates { projects: roots } => print(projects::duplicates(&roots)?),
        Action::Recovery { project } => print(projects::recovery(&Project::read(project)?)?),
        Action::Metadata { project, apply } => {
            let p = Project::read(project)?;
            if let Some(path) = apply {
                projects::save_metadata(&p, &read_json(&path, 1_000_000)?)?;
            }
            print(projects::metadata(&p)?)
        }
        Action::AttachReference {
            project,
            input,
            label,
            role,
        } => print(projects::attach_reference(
            &Project::read(project)?,
            &input,
            &label,
            &role,
            c,
        )?),
        Action::SaveTemplate { name, output } => projects::save_template(&output, &name, s),
        Action::LoadTemplate { input, output } => {
            anyhow::ensure!(
                !output.exists(),
                Fault::new("NG401", "Settings output already exists")
            );
            atomic_json(&output, &projects::load_template(&input)?.settings)
        }
        Action::ProfileCoverage { writer } => {
            print(profile_tools::coaching(&s.writer.root, &writer)?)
        }
        Action::ProfileExercises { writer } => {
            print(profile_tools::confusion_exercises(&s.writer.root, &writer)?)
        }
        Action::GlossarySuggestions { writer } => print(profile_tools::glossary_suggestions(
            &s.writer.root,
            &writer,
        )?),
        Action::Quarantine {
            writer,
            sample,
            reason,
        } => print(profile_tools::quarantine(
            &s.writer.root,
            &writer,
            &sample,
            &reason,
        )?),
        Action::RestoreExample { writer, sample } => print(profile_tools::restore_example(
            &s.writer.root,
            &writer,
            &sample,
        )?),
        Action::ExampleConditions {
            writer,
            sample,
            conditions,
        } => print(profile_tools::set_conditions(
            &s.writer.root,
            &writer,
            &sample,
            &conditions,
        )?),
        Action::ProfileSnapshot { writer, name } => {
            print(profile_tools::snapshot(&s.writer.root, &writer, &name, c)?)
        }
        Action::ProfileVersions { writer } => {
            print(profile_tools::snapshots(&s.writer.root, &writer)?)
        }
        Action::CompareProfiles {
            writer,
            left,
            right,
        } => print(profile_tools::compare_snapshots(
            &s.writer.root,
            &writer,
            &left,
            &right,
            c,
        )?),
        Action::BranchProfile {
            writer,
            name,
            version,
        } => print(profile_tools::branch(
            &s.writer.root,
            &writer,
            version.as_deref(),
            &name,
            c,
        )?),
        Action::PreviewProfileShare { writer, selection } => print(profile_tools::share_preview(
            &s.writer.root,
            &writer,
            &read_json(&selection, 1_000_000)?,
            c,
        )?),
        Action::ExportProfileSelection {
            writer,
            selection,
            preview_token,
            output,
        } => profile_tools::export_selected(
            &s.writer.root,
            &writer,
            &read_json(&selection, 1_000_000)?,
            &preview_token,
            &output,
            c,
        ),
        Action::FontProof { writer, output } => print(profile_tools::font_proof_sheet(
            &s.writer.root,
            &writer,
            &output,
            &Default::default(),
            c,
        )?),
    }
}
