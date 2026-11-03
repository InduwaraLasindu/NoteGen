use clap::Subcommand;
use notegen::{
    geometry, geometry_repair, language_routing,
    storage::{Control, Project, read_json},
    types::*,
};
use std::path::PathBuf;
#[derive(Subcommand)]
pub enum Action {
    Inspect {
        project: PathBuf,
        page: usize,
    },
    DetectScript {
        project: PathBuf,
        page: usize,
    },
    SuggestLanguages {
        project: PathBuf,
        page: usize,
        #[arg(long, default_value_t = 8)]
        regions: usize,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Store an explicit source-region language selection; JSON contains region/languages/layout/target.
    SelectLanguages {
        project: PathBuf,
        page: usize,
        request: PathBuf,
        #[arg(long)]
        replace: bool,
    },
    PreviewLanguages {
        project: PathBuf,
        page: usize,
        #[arg(short, long)]
        output: PathBuf,
    },
    ApplyLanguages {
        project: PathBuf,
        page: usize,
        preview: PathBuf,
        #[arg(long)]
        confirmation_token: String,
    },
    RepairLines {
        project: PathBuf,
        page: usize,
        block: usize,
        quads: PathBuf,
    },
    PreviewGrid {
        project: PathBuf,
        page: usize,
        block: usize,
        repair: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    ApplyGrid {
        project: PathBuf,
        page: usize,
        preview: PathBuf,
        #[arg(long)]
        confirmation_token: String,
    },
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    region: BoxRect,
    languages: String,
    layout: language_routing::RegionLayout,
    target: Option<usize>,
}
pub fn run(action: Action, s: &Settings, c: &Control) -> Result<()> {
    match action {
        Action::Inspect { project, page } => {
            let project = Project::read(project)?;
            let page = notegen::redactions::analysis_page(&project, &project.load(page)?, c)?;
            super::print(
                serde_json::json!({"page":page.page,"script_detection":page.script_detection,"region_languages":page.region_languages,"blocks":page.content.blocks.iter().enumerate().map(|(index,block)|serde_json::json!({"index":index,"geometry":block.geometry})).collect::<Vec<_>>()}),
            )
        }
        Action::DetectScript { project, page } => {
            let project = Project::read(project)?;
            let page = project.load(page)?;
            let (source, image) = notegen::redactions::recognition_view(&project, &page, c)?;
            super::print(
                serde_json::json!({"source":source,"observation":notegen::scan::detect_script(&image,s,c)?}),
            )
        }
        Action::SuggestLanguages {
            project,
            page,
            regions,
            output,
        } => {
            let project = Project::read(project)?;
            let page = project.load(page)?;
            let report = language_routing::suggest_regions(&project, &page, s, regions, c)?;
            super::protected_json(&output, &report)?;
            super::print(report)
        }
        Action::SelectLanguages {
            project,
            page,
            request,
            replace,
        } => {
            let request: Selection = read_json(&request, 100_000)?;
            let project = Project::read(project)?;
            let page = project.load(page)?;
            let selection = language_routing::select_region(
                &project,
                &page,
                request.region,
                &request.languages,
                request.layout,
                request.target,
                c,
            )?;
            let mut selections = if replace {
                vec![]
            } else {
                page.region_languages.clone()
            };
            selections.push(selection);
            super::print(language_routing::commit_selections(
                &project, &page, selections, c,
            )?)
        }
        Action::PreviewLanguages {
            project,
            page,
            output,
        } => {
            let project = Project::read(project)?;
            let page = project.load(page)?;
            let preview =
                language_routing::reread_preview(&project, &page, &page.region_languages, s, c)?;
            super::protected_json(&output, &preview)?;
            super::print(preview)
        }
        Action::ApplyLanguages {
            project,
            page,
            preview,
            confirmation_token,
        } => {
            let project = Project::read(project)?;
            let page = project.load(page)?;
            super::print(language_routing::apply_preview(
                &project,
                &page,
                &read_json(&preview, 32_000_000)?,
                &confirmation_token,
                c,
            )?)
        }
        Action::RepairLines {
            project,
            page,
            block,
            quads,
        } => {
            let project = Project::read(project)?;
            let page = project.load(page)?;
            let quads: Vec<geometry::Quad> = read_json(&quads, 1_000_000)?;
            super::print(geometry_repair::repair_lines(
                &project, &page, block, &quads, c,
            )?)
        }
        Action::PreviewGrid {
            project,
            page,
            block,
            repair,
            output,
        } => {
            let project = Project::read(project)?;
            let page = project.load(page)?;
            let preview = geometry_repair::preview_grid(
                &project,
                &page,
                block,
                &read_json(&repair, 1_000_000)?,
                c,
            )?;
            super::protected_json(&output, &preview)?;
            super::print(preview)
        }
        Action::ApplyGrid {
            project,
            page,
            preview,
            confirmation_token,
        } => {
            let project = Project::read(project)?;
            let page = project.load(page)?;
            super::print(geometry_repair::apply_grid(
                &project,
                &page,
                &read_json(&preview, 32_000_000)?,
                &confirmation_token,
                c,
            )?)
        }
    }
}
