//! Recompute opaque transaction previews before applying an explicitly selected token.
use clap::Subcommand;
use notegen::{
    migrations, project_layout, project_reuse,
    storage::{Control, Project, read_json},
    types::*,
};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum Action {
    /// Supported, backed-up project/settings migrations and conflict-safe recovery.
    Migrations {
        #[command(subcommand)]
        command: Migration,
    },
    /// Reversible page order, insertion, removal, splitting and merging.
    Layout {
        #[command(subcommand)]
        command: Layout,
    },
    /// Explicit compatible duplicate reuse, private revision copies and comparison.
    Reuse {
        #[command(subcommand)]
        command: Reuse,
    },
}
#[derive(Subcommand)]
pub enum Migration {
    PreviewProject {
        project: PathBuf,
    },
    ApplyProject {
        project: PathBuf,
        #[arg(long)]
        confirmation_token: String,
    },
    PreviewSettings {
        settings: PathBuf,
    },
    ApplySettings {
        settings: PathBuf,
        #[arg(long)]
        confirmation_token: String,
    },
    PreviewRollback {
        receipt: PathBuf,
    },
    Rollback {
        receipt: PathBuf,
        #[arg(long)]
        confirmation_token: String,
    },
    RecoverProject {
        project: PathBuf,
    },
    RecoverSettings {
        settings: PathBuf,
    },
}
#[derive(Subcommand)]
pub enum Layout {
    Inspect {
        project: PathBuf,
    },
    /// Operation JSON is bounded; the preview contains the exact confirmation token.
    Preview {
        project: PathBuf,
        operation: PathBuf,
    },
    Apply {
        project: PathBuf,
        operation: PathBuf,
        #[arg(long)]
        confirmation_token: String,
    },
    Undo {
        project: PathBuf,
        #[arg(long)]
        expected_revision: u64,
    },
    Redo {
        project: PathBuf,
        #[arg(long)]
        expected_revision: u64,
    },
}
#[derive(Subcommand)]
pub enum Reuse {
    Preview {
        source: PathBuf,
        #[arg(long, required = true)]
        project: Vec<PathBuf>,
    },
    Choose {
        source: PathBuf,
        #[arg(long, required = true)]
        project: Vec<PathBuf>,
        #[arg(long)]
        index: usize,
        #[arg(long, value_parser = ["reuse", "separate-revision"])]
        choice: String,
        #[arg(long)]
        target: Option<PathBuf>,
        #[arg(long)]
        confirmation_token: String,
    },
    Compare {
        first: PathBuf,
        second: PathBuf,
    },
}
pub fn run(action: Action, s: &Settings, c: &Control) -> Result<()> {
    match action {
        Action::Migrations { command } => migration(command, c),
        Action::Layout { command } => layout(command, c),
        Action::Reuse { command } => reuse(command, s, c),
    }
}
fn migration(action: Migration, c: &Control) -> Result<()> {
    match action {
        Migration::PreviewProject { project } => super::print(migrations::preview_project_sources(
            &Project::read(project)?,
            c,
        )?),
        Migration::ApplyProject {
            project,
            confirmation_token,
        } => {
            let plan = migrations::preview_project_sources(&Project::read(project)?, c)?;
            super::print(migrations::apply(&plan, &confirmation_token, c)?)
        }
        Migration::PreviewSettings { settings } => {
            super::print(migrations::preview_settings(&settings, c)?)
        }
        Migration::ApplySettings {
            settings,
            confirmation_token,
        } => {
            let plan = migrations::preview_settings(&settings, c)?;
            super::print(migrations::apply(&plan, &confirmation_token, c)?)
        }
        Migration::PreviewRollback { receipt } => super::print(migrations::rollback_preview(
            &migrations::read_receipt(&receipt, c)?,
            c,
        )?),
        Migration::Rollback {
            receipt,
            confirmation_token,
        } => {
            let plan = migrations::rollback_preview(&migrations::read_receipt(&receipt, c)?, c)?;
            super::print(migrations::apply(&plan, &confirmation_token, c)?)
        }
        Migration::RecoverProject { project } => {
            migrations::recover(&project, c)?;
            super::print(serde_json::json!({"recovered": project}))
        }
        Migration::RecoverSettings { settings } => {
            migrations::recover_settings(&settings, c)?;
            super::print(serde_json::json!({"recovered": settings}))
        }
    }
}
fn layout(action: Layout, c: &Control) -> Result<()> {
    match action {
        Layout::Inspect { project } => {
            super::print(project_layout::catalog(&Project::read(project)?)?)
        }
        Layout::Preview { project, operation } => {
            let operation = read_json(&operation, 1_000_000)?;
            super::print(project_layout::preview(
                &Project::read(project)?,
                operation,
                c,
            )?)
        }
        Layout::Apply {
            project,
            operation,
            confirmation_token,
        } => {
            let project = Project::read(project)?;
            let operation = read_json(&operation, 1_000_000)?;
            let preview = project_layout::preview(&project, operation, c)?;
            project_result(project_layout::apply(
                &project,
                &preview,
                &confirmation_token,
                c,
            )?)
        }
        Layout::Undo {
            project,
            expected_revision,
        } => project_result(project_layout::undo(
            &Project::read(project)?,
            expected_revision,
            c,
        )?),
        Layout::Redo {
            project,
            expected_revision,
        } => project_result(project_layout::redo(
            &Project::read(project)?,
            expected_revision,
            c,
        )?),
    }
}
fn reuse(action: Reuse, s: &Settings, c: &Control) -> Result<()> {
    match action {
        Reuse::Preview { source, project } => {
            super::print(project_reuse::preview(&source, s, &project, c)?)
        }
        Reuse::Choose {
            source,
            project,
            index,
            choice,
            target,
            confirmation_token,
        } => {
            anyhow::ensure!(
                choice == "separate-revision" || target.is_none(),
                Fault::new(
                    "NG102",
                    "A target folder is only used for a separate revision"
                )
            );
            let preview = project_reuse::preview(&source, s, &project, c)?;
            let choice = if choice == "reuse" {
                project_reuse::Choice::Reuse
            } else {
                project_reuse::Choice::SeparateRevision
            };
            project_result(project_reuse::choose(
                &preview,
                &confirmation_token,
                index,
                choice,
                target.as_deref(),
                c,
            )?)
        }
        Reuse::Compare { first, second } => super::print(project_reuse::compare(
            &Project::read(first)?,
            &Project::read(second)?,
            c,
        )?),
    }
}
fn project_result(project: Project) -> Result<()> {
    super::print(serde_json::json!({"project": project.root, "pages": project.manifest.pages}))
}
