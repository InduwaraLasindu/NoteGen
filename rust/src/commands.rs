//! Explicit local tools. JSON previews can be inspected before a separate apply command.
use clap::Subcommand;
use notegen::{
    evaluation, exports, inputs, preflight, profile_tools, projects, review, storage::*, types::*,
};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum Action {
    RegionRead {
        project: PathBuf,
        request: PathBuf,
    },
    ReconsiderPages {
        project: PathBuf,
        pages: String,
    },
    BundlePreview {
        #[arg(long, value_delimiter = ',')]
        models: Vec<String>,
        #[arg(long)]
        runtime: Option<String>,
    },
    ExportBundle {
        #[arg(long, value_delimiter = ',')]
        models: Vec<String>,
        #[arg(long)]
        runtime: Option<String>,
        #[arg(short, long)]
        output: PathBuf,
    },
    InspectBundle {
        input: PathBuf,
    },
    ImportBundle {
        input: PathBuf,
        destination: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    InstallPreflight {
        #[arg(long, value_delimiter = ',')]
        models: Vec<String>,
        #[arg(long)]
        runtime: Option<String>,
    },
    CapabilityCards,
    ResourcePreview {
        profile: String,
        #[arg(long, default_value_t = 1)]
        reserve_cores: usize,
        #[arg(long)]
        reserve_ram_mib: Option<u64>,
        #[arg(long)]
        reserve_vram_mib: Option<u64>,
        #[arg(short, long)]
        output: PathBuf,
    },
    ApplyResourcePreview {
        preview: PathBuf,
        #[arg(long)]
        undo: bool,
        #[arg(short, long)]
        output: PathBuf,
    },
    TimeBudget {
        jobs: PathBuf,
        seconds: f64,
    },
    PreviewWord {
        project: PathBuf,
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    ImportWord {
        project: PathBuf,
        input: PathBuf,
        preview: PathBuf,
        selections: PathBuf,
        #[arg(long, default_value = "Confirmed Word corrections")]
        label: String,
    },
    WordStyles {
        project: PathBuf,
        #[arg(long)]
        apply: Option<PathBuf>,
    },
    CleanupScan {
        input: PathBuf,
        #[arg(long)]
        options: Option<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
    },
    PerspectiveScan {
        input: PathBuf,
        corners: PathBuf,
        width: u32,
        height: u32,
        #[arg(short, long)]
        output: PathBuf,
    },
    SplitScan {
        input: PathBuf,
        #[arg(long, default_value_t = 0.5)]
        gutter_left: f32,
        #[arg(long, default_value_t = 0.5)]
        gutter_right: f32,
        #[arg(short, long)]
        output: PathBuf,
    },
    RedactScan {
        input: PathBuf,
        regions: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
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
        Action::RegionRead { project, request } => print(notegen::pipeline::reread_region(
            &Project::read(project)?,
            &read_json(&request, 100_000)?,
            s,
            c,
            &Activity::default(),
        )?),
        Action::ReconsiderPages { project, pages } => {
            let p = Project::read(project)?;
            let maximum = p.manifest.pages.iter().copied().max().unwrap_or(0);
            let selected = pages_range(&pages, maximum)?;
            anyhow::ensure!(
                selected.iter().all(|n| p.manifest.pages.contains(n)),
                Fault::new("NG100", "Selected page is not in this project")
            );
            print(notegen::pipeline::reconsider(
                &p,
                &selected,
                s,
                c,
                &Activity::default(),
            )?)
        }
        Action::BundlePreview { models, runtime } => print(notegen::bundles::preview(
            s,
            &models,
            runtime.as_deref(),
            c,
        )?),
        Action::ExportBundle {
            models,
            runtime,
            output,
        } => notegen::bundles::export(s, &models, runtime.as_deref(), &output, c),
        Action::InspectBundle { input } => print(notegen::bundles::inspect(&input, c)?),
        Action::ImportBundle {
            input,
            destination,
            output,
        } => {
            anyhow::ensure!(
                !output.exists(),
                Fault::new("NG401", "Settings output exists")
            );
            let next = notegen::bundles::import(s, &input, &destination, c)?;
            protected_json(&output, &next)
        }
        Action::InstallPreflight { models, runtime } => print(notegen::bundles::preflight(
            s,
            &models,
            runtime.as_deref(),
            c,
        )?),
        Action::CapabilityCards => print(notegen::bundles::capability_cards()?),
        Action::ResourcePreview {
            profile,
            reserve_cores,
            reserve_ram_mib,
            reserve_vram_mib,
            output,
        } => {
            let profile = serde_json::from_value(serde_json::Value::String(profile))?;
            let hw = notegen::hardware::detect_for(s, c);
            let preview = notegen::resources::preview(
                s,
                &hw,
                profile,
                &notegen::resources::Reservations {
                    cpu_cores: reserve_cores,
                    ram_mib: reserve_ram_mib,
                    vram_mib: reserve_vram_mib,
                },
            )?;
            protected_json(&output, &preview)?;
            print(preview)
        }
        Action::ApplyResourcePreview {
            preview,
            undo,
            output,
        } => {
            let p: notegen::resources::ResourcePreview = read_json(&preview, 1_000_000)?;
            let mut next = s.clone();
            if undo {
                p.undo(&mut next)?;
            } else {
                p.apply(&mut next)?;
            }
            protected_json(&output, &next)
        }
        Action::TimeBudget { jobs, seconds } => print(notegen::resources::time_budget(
            &read_json::<Vec<notegen::resources::QueueDuration>>(&jobs, 1_000_000)?,
            seconds,
        )?),
        Action::PreviewWord {
            project,
            input,
            output,
        } => {
            let preview = notegen::roundtrip::preview(&Project::read(project)?, &input, c)?;
            protected_json(&output, &preview)?;
            print(preview)
        }
        Action::ImportWord {
            project,
            input,
            preview,
            selections,
            label,
        } => print(notegen::roundtrip::apply(
            &Project::read(project)?,
            &input,
            &read_json(&preview, 32_000_000)?,
            &read_json::<Vec<String>>(&selections, 1_000_000)?,
            &label,
            c,
        )?),
        Action::WordStyles { project, apply } => {
            let p = Project::read(project)?;
            if let Some(path) = apply {
                notegen::word::save_style_profile(&p, &read_json(&path, 1_000_000)?)?;
            }
            print(notegen::word::style_profile(&p)?)
        }
        Action::CleanupScan {
            input,
            options,
            output,
        } => {
            let options = if let Some(path) = options {
                read_json(&path, 100_000)?
            } else {
                notegen::scan_tools::CleanupOptions::default()
            };
            let im = notegen::writers::open_image(&input, 40_000_000)?;
            let comparison = notegen::scan_tools::compare(&im, &options, c)?;
            notegen::scan_tools::export_comparison(&comparison, &output, c)
        }
        Action::PerspectiveScan {
            input,
            corners,
            width,
            height,
            output,
        } => {
            let im = notegen::writers::open_image(&input, 40_000_000)?;
            let result = notegen::scan_tools::perspective(
                &im,
                read_json(&corners, 100_000)?,
                width,
                height,
                c,
            )?;
            notegen::scan_tools::write_image(&result, &output, c)
        }
        Action::SplitScan {
            input,
            gutter_left,
            gutter_right,
            output,
        } => {
            let im = notegen::writers::open_image(&input, 40_000_000)?;
            let split = notegen::scan_tools::split_double(&im, gutter_left, gutter_right, c)?;
            notegen::scan_tools::export_split(&split, &output, c)
        }
        Action::RedactScan {
            input,
            regions,
            output,
        } => {
            let im = notegen::writers::open_image(&input, 40_000_000)?;
            notegen::scan_tools::export_redacted(
                &im,
                &read_json::<Vec<BoxRect>>(&regions, 100_000)?,
                &output,
                c,
            )
            .map(|_| ())
        }
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
            protected_json(&output, &preview)?;
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
            protected_json(&output, &projects::load_template(&input)?.settings)
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
