use clap::Subcommand;
use notegen::{
    project_lock,
    storage::{Control, Project, read_json},
    types::*,
};
use std::{collections::BTreeSet, path::PathBuf};
#[derive(Subcommand)]
pub enum Action {
    Inspect {
        lock: PathBuf,
    },
    Preview {
        project: PathBuf,
        #[arg(long, value_delimiter = ',')]
        models: Vec<String>,
        #[arg(short, long)]
        output: PathBuf,
    },
    Apply {
        project: PathBuf,
        preview: PathBuf,
        #[arg(long)]
        expected_current: Option<String>,
    },
    Preferences {
        project: PathBuf,
        request: PathBuf,
        #[arg(long)]
        expected_current: String,
    },
    RestorePreview {
        lock: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    Restore {
        preview: PathBuf,
        #[arg(long)]
        confirmation_token: String,
    },
    ImportMissing {
        preview: PathBuf,
        index: usize,
        source: PathBuf,
        #[arg(long)]
        confirmation_token: String,
    },
}
pub fn needs_download_control(action: &Action) -> bool {
    matches!(action, Action::Restore { .. })
}
pub fn run(action: Action, s: &Settings, c: &Control) -> Result<()> {
    match action {
        Action::Inspect { lock } => super::print(project_lock::read(&lock, c)?),
        Action::Preview {
            project,
            models,
            output,
        } => {
            let project = Project::read(project)?;
            let installed = notegen::runtime::inventory(&s.models_dir)?;
            let ids = if models.is_empty() {
                project
                    .manifest
                    .models
                    .iter()
                    .filter(|id| id.as_str() != "tesseract")
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                models
            };
            let distinct = ids.iter().collect::<BTreeSet<_>>();
            anyhow::ensure!(
                distinct.len() == ids.len(),
                Fault::new("NG670", "Choose distinct local reader IDs")
            );
            let models = ids
                .iter()
                .map(|id| {
                    installed
                        .iter()
                        .find(|model| &model.id == id)
                        .cloned()
                        .ok_or_else(|| Fault::new("NG670", "Selected reader is not installed"))
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let preview = project_lock::capture(
                &project,
                &models,
                s,
                project_lock::Origin::FutureSelection,
                c,
            )?;
            super::protected_json(&output, &preview)?;
            super::print(preview)
        }
        Action::Apply {
            project,
            preview,
            expected_current,
        } => super::print(project_lock::write(
            &Project::read(project)?,
            &read_json(&preview, 4_000_000)?,
            expected_current.as_deref(),
            c,
        )?),
        Action::Preferences {
            project,
            request,
            expected_current,
        } => super::print(project_lock::set_preferences(
            &Project::read(project)?,
            &expected_current,
            read_json(&request, 100_000)?,
            c,
        )?),
        Action::RestorePreview { lock, output } => {
            let preview = project_lock::preview_restore(&lock, s, c)?;
            super::protected_json(&output, &preview)?;
            super::print(preview)
        }
        Action::Restore {
            preview,
            confirmation_token,
        } => super::print(project_lock::restore(
            &read_json(&preview, 4_000_000)?,
            &confirmation_token,
            s,
            c,
            &Default::default(),
        )?),
        Action::ImportMissing {
            preview,
            index,
            source,
            confirmation_token,
        } => super::print(project_lock::import_missing(
            &read_json(&preview, 4_000_000)?,
            &confirmation_token,
            index,
            &source,
            s,
            c,
        )?),
    }
}
