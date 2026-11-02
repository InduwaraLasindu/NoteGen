use clap::Subcommand;
use notegen::{
    redactions, review,
    storage::{Control, Project, read_json},
    types::*,
};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Subcommand)]
pub enum Action {
    Covers {
        project: PathBuf,
        page: usize,
    },
    Preview {
        project: PathBuf,
        page: usize,
        proposals: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    Confirm {
        project: PathBuf,
        page: usize,
        preview: PathBuf,
        selections: PathBuf,
        #[arg(long)]
        confirmation_token: String,
    },
    /// Remove explicit mask IDs. Withheld text stays withheld until re-read or undo.
    Remove {
        project: PathBuf,
        page: usize,
        #[arg(required = true)]
        ids: Vec<String>,
    },
    SharePreview {
        project: PathBuf,
        #[arg(long)]
        pages: Option<String>,
        #[arg(short, long)]
        output: PathBuf,
    },
    Share {
        project: PathBuf,
        preview: PathBuf,
        #[arg(long)]
        confirmation_token: String,
        #[arg(short, long)]
        output: PathBuf,
    },
}

pub fn run(action: Action, c: &Control) -> Result<()> {
    match action {
        Action::Covers { project, page } => {
            let project = Project::read(project)?;
            let page = project.load(page)?;
            let (_, image, _) = redactions::effective_page(&project, &page, c)?;
            super::print(redactions::suggest_opaque_covers(&image, c)?)
        }
        Action::Preview {
            project,
            page,
            proposals,
            output,
        } => {
            let project = Project::read(project)?;
            let page = project.load(page)?;
            let preview = redactions::preview_confirmation(
                &project,
                &page,
                &read_json::<Vec<redactions::MaskSpec>>(&proposals, 100_000)?,
                c,
            )?;
            super::protected_json(&output, &preview)?;
            super::print(preview)
        }
        Action::Confirm {
            project,
            page,
            preview,
            selections,
            confirmation_token,
        } => {
            let project = Project::read(project)?;
            let before = project.load(page)?;
            let mut after = before.clone();
            after.confirmed_redactions = redactions::confirm(
                &project,
                &before,
                &read_json(&preview, 100_000)?,
                &confirmation_token,
                &read_json::<Vec<usize>>(&selections, 100_000)?,
                c,
            )?;
            after = redactions::effective_page(&project, &after, c)?.0;
            after.edited = true;
            after.reviewed = false;
            super::print(review::commit(
                &project,
                &before,
                &after,
                "Confirm source redactions",
            )?)
        }
        Action::Remove { project, page, ids } => {
            let project = Project::read(project)?;
            let before = project.load(page)?;
            let selected = ids.iter().collect::<BTreeSet<_>>();
            anyhow::ensure!(
                !selected.is_empty()
                    && selected.len() == ids.len()
                    && selected
                        .iter()
                        .all(|id| before.confirmed_redactions.iter().any(|m| &m.id == *id)),
                Fault::new("NG500", "Choose distinct existing mask IDs")
            );
            let mut after = before.clone();
            after
                .confirmed_redactions
                .retain(|m| !selected.contains(&m.id));
            after.edited = true;
            after.reviewed = false;
            super::print(review::commit(
                &project,
                &before,
                &after,
                "Remove confirmed source masks; text remains withheld",
            )?)
        }
        Action::SharePreview {
            project,
            pages,
            output,
        } => {
            let project = Project::read(project)?;
            let selected = if let Some(pages) = pages {
                let selected = pages_range(
                    &pages,
                    project.manifest.pages.iter().copied().max().unwrap_or(0),
                )?;
                anyhow::ensure!(
                    selected.iter().all(|n| project.manifest.pages.contains(n)),
                    Fault::new("NG500", "Choose prepared project pages")
                );
                selected
            } else {
                project.manifest.pages.clone()
            };
            let preview = redactions::preview_share(&project, &selected, c)?;
            super::protected_json(&output, &preview)?;
            super::print(preview)
        }
        Action::Share {
            project,
            preview,
            confirmation_token,
            output,
        } => super::print(redactions::export_share(
            &Project::read(project)?,
            &read_json(&preview, 1_000_000)?,
            &confirmation_token,
            &output,
            c,
        )?),
    }
}
