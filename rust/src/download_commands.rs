//! CLI access to the same persisted transfer policy used by the desktop app.
use clap::Subcommand;
use notegen::{
    downloads,
    storage::{Control, read_json},
    types::*,
};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum Action {
    Status,
    /// Print the current policy, or replace it with a bounded JSON policy file.
    Policy {
        input: Option<PathBuf>,
    },
    Enqueue {
        request: PathBuf,
    },
    Run,
    Pause {
        id: String,
    },
    Resume {
        id: String,
    },
    Cancel {
        id: String,
    },
    /// Remove a finished job record. Downloaded and partial files are retained.
    Forget {
        id: String,
    },
    Override {
        id: String,
        request: PathBuf,
    },
}

pub fn run(action: Action, c: &Control) -> Result<()> {
    let manager = c.downloads.as_ref().ok_or_else(|| {
        Fault::new(
            "NG650",
            "Download controls require an available download store",
        )
    })?;
    match action {
        Action::Status => super::print(
            serde_json::json!({"policy":manager.policy()?, "jobs":manager.list()?, "limits":downloads::LIMITS}),
        ),
        Action::Policy { input } => {
            if let Some(input) = input {
                manager.set_policy(read_json(&input, 100_000)?)?;
            }
            super::print(manager.policy()?)
        }
        Action::Enqueue { request } => {
            super::print(manager.enqueue(read_json(&request, 100_000)?)?)
        }
        Action::Run => super::print(manager.run_pending(c, &Default::default())?),
        Action::Pause { id } => manager.pause(&id),
        Action::Resume { id } => manager.resume(&id),
        Action::Cancel { id } => manager.cancel(&id),
        Action::Forget { id } => manager.forget(&id),
        Action::Override { id, request } => {
            manager.set_override(&id, read_json(&request, 100_000)?)
        }
    }
}
