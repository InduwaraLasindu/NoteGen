//! Vault secrets arrive through bounded piped stdin, never command-line arguments.
use clap::Subcommand;
use notegen::{
    storage::{self, Control, Project, read_json},
    types::*,
    vault,
};
use std::{
    io::{IsTerminal, Read},
    path::PathBuf,
    time::Duration,
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Subcommand)]
pub enum Action {
    RecoveryKey {
        output: PathBuf,
        #[arg(long)]
        offline_export: bool,
    },
    /// Read one passphrase line from piped stdin. Original project files remain outside the vault.
    Create {
        project: PathBuf,
        recovery_key: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    Verify {
        input: PathBuf,
        #[arg(long)]
        recovery_key: Option<PathBuf>,
    },
    Backup {
        input: PathBuf,
        #[arg(long)]
        recovery_key: Option<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
    },
    Restore {
        input: PathBuf,
        #[arg(long)]
        recovery_key: Option<PathBuf>,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Read old and new passphrase lines, or just the new line when using a recovery key.
    Rotate {
        input: PathBuf,
        #[arg(long)]
        recovery_key: Option<PathBuf>,
        #[arg(long)]
        new_recovery_key: Option<PathBuf>,
    },
    /// Execute a bounded JSON operation list in a private work copy, then commit and lock.
    Work {
        input: PathBuf,
        work_parent: PathBuf,
        operations: PathBuf,
        #[arg(long)]
        recovery_key: Option<PathBuf>,
    },
    InspectRetained {
        work_root: PathBuf,
    },
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Inspect,
    SavePage {
        page: usize,
        expected_fingerprint: String,
        replacement: Box<Page>,
    },
    Convert {
        input: PathBuf,
    },
    Export {
        format: String,
        output: PathBuf,
    },
}

fn secrets(count: usize) -> Result<Vec<vault::SecretString>> {
    anyhow::ensure!(
        !std::io::stdin().is_terminal(),
        Fault::new(
            "NG682",
            "Use the desktop password field or pipe the required passphrase line(s) through stdin; passwords are never accepted as arguments"
        )
    );
    let mut bytes = Zeroizing::new(Vec::new());
    std::io::stdin().lock().take(8193).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 8192,
        Fault::new("NG682", "Passphrase input exceeds its byte limit")
    );
    let mut text = Zeroizing::new(
        std::str::from_utf8(&bytes)
            .map_err(|_| Fault::new("NG682", "Passphrase input must be UTF-8"))?
            .to_owned(),
    );
    let lines = text
        .split_terminator('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect::<Vec<_>>();
    anyhow::ensure!(
        lines.len() == count
            && lines
                .iter()
                .all(|line| !line.is_empty() && line.len() <= 4096),
        Fault::new(
            "NG682",
            format!("Pipe exactly {count} nonempty passphrase line(s) through stdin")
        )
    );
    let result = lines
        .into_iter()
        .map(|line| vault::SecretString::from(line.to_owned()))
        .collect();
    text.zeroize();
    Ok(result)
}

fn with_credentials<T>(
    key: Option<&PathBuf>,
    f: impl FnOnce(vault::Credentials<'_>) -> Result<T>,
) -> Result<T> {
    if let Some(key) = key {
        f(vault::Credentials::RecoveryKey(key))
    } else {
        let pass = secrets(1)?.remove(0);
        f(vault::Credentials::Passphrase(&pass))
    }
}

pub fn run(action: Action, settings: &Settings, c: &Control) -> Result<()> {
    match action {
        Action::RecoveryKey {
            output,
            offline_export,
        } => super::print(if offline_export {
            vault::generate_recovery_key_offline_export(&output, c)?
        } else {
            vault::generate_recovery_key(&output, c)?
        }),
        Action::Create {
            project,
            recovery_key,
            output,
        } => {
            let pass = secrets(1)?.remove(0);
            super::print(vault::create(
                &project,
                &output,
                &pass,
                &recovery_key,
                &Default::default(),
                c,
            )?)
        }
        Action::Verify {
            input,
            recovery_key,
        } => with_credentials(recovery_key.as_ref(), |key| {
            super::print(vault::verify(&input, &key, c)?)
        }),
        Action::Backup {
            input,
            recovery_key,
            output,
        } => with_credentials(recovery_key.as_ref(), |key| {
            super::print(vault::backup(&input, &output, &key, c)?)
        }),
        Action::Restore {
            input,
            recovery_key,
            output,
        } => with_credentials(recovery_key.as_ref(), |key| {
            super::print(vault::restore(&input, &output, &key, c)?)
        }),
        Action::Rotate {
            input,
            recovery_key,
            new_recovery_key,
        } => {
            let pass = secrets(if recovery_key.is_some() { 1 } else { 2 })?;
            let (key, new) = if let Some(key) = &recovery_key {
                (vault::Credentials::RecoveryKey(key), &pass[0])
            } else {
                (vault::Credentials::Passphrase(&pass[0]), &pass[1])
            };
            super::print(vault::rotate(
                &input,
                &key,
                new,
                new_recovery_key.as_deref(),
                &Default::default(),
                c,
            )?)
        }
        Action::Work {
            input,
            work_parent,
            operations,
            recovery_key,
        } => {
            let operations: Vec<Operation> = read_json(&operations, 2_000_000)?;
            anyhow::ensure!(
                !operations.is_empty() && operations.len() <= 100,
                Fault::new("NG682", "Choose one to 100 explicit work operations")
            );
            let pass = secrets(1)?.remove(0);
            let key = if let Some(key) = &recovery_key {
                vault::Credentials::RecoveryKey(key)
            } else {
                vault::Credentials::Passphrase(&pass)
            };
            let mut work = vault::unlock(
                &input,
                &key,
                &work_parent,
                recovery_key.as_ref().map(|_| &pass),
                &Default::default(),
                c,
            )?;
            eprintln!(
                "Managed plaintext work copy: {}. {}",
                work.work_root().display(),
                work.note()
            );
            let result = (|| -> Result<()> {
                for operation in operations {
                    c.check()?;
                    if let Operation::Convert { input } = operation {
                        let report = work.run_session(&input, settings, c, &Default::default())?;
                        let success = report.success;
                        super::print(report)?;
                        anyhow::ensure!(
                            success,
                            Fault::new("NG682", "Vault session conversion did not complete")
                        );
                        continue;
                    }
                    let _guard = work.begin_worker()?;
                    let project = Project::read(work.project_path())?;
                    match operation {
                        Operation::Inspect => super::print(&project.manifest)?,
                        Operation::SavePage {
                            page,
                            expected_fingerprint,
                            replacement,
                        } => {
                            let before = project.load(page)?;
                            anyhow::ensure!(
                                storage::fingerprint(&before)? == expected_fingerprint
                                    && replacement.page == page
                                    && replacement.confirmed_redactions
                                        == before.confirmed_redactions,
                                Fault::new(
                                    "NG682",
                                    "Vault page changed; inspect a fresh page and preserve confirmed source masks"
                                )
                            );
                            super::print(notegen::review::commit(
                                &project,
                                &before,
                                &replacement,
                                "Vault review edit",
                            )?)?;
                        }
                        Operation::Export { format, output } => match format.as_str() {
                            "word" | "docx" => {
                                notegen::word::export(&project, &output, c)?;
                            }
                            "pdf" => {
                                super::print(notegen::searchable_pdf::export(
                                    &project,
                                    &output,
                                    &Default::default(),
                                    c,
                                )?)?;
                            }
                            _ => {
                                super::print(notegen::exports::export(
                                    &project,
                                    &output,
                                    super::format(&format)?,
                                    c,
                                )?)?;
                            }
                        },
                        Operation::Convert { .. } => unreachable!(),
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                return Err(Fault::new("NG682", format!("Vault work stopped: {error}. The previous encrypted revision is unchanged. Plaintext is retained for explicit recovery at {}", work.work_root().display())).into());
            }
            let report = work.commit_and_lock(Duration::from_secs(30), c)?;
            let fully_locked = report.fully_locked;
            super::print(report)?;
            anyhow::ensure!(
                fully_locked,
                Fault::new(
                    "NG682",
                    "Encrypted commit succeeded but some plaintext paths were retained; inspect the lock report"
                )
            );
            Ok(())
        }
        Action::InspectRetained { work_root } => {
            super::print(vault::inspect_retained(&work_root, c)?)
        }
    }
}
