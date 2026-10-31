use clap::{Parser, Subcommand};
use notegen::{hardware, pipeline, planning, runtime, storage::*, types::*, word};
use std::{fs, path::PathBuf, sync::Arc};
#[derive(Parser)]
#[command(
    name = "notegen-cli",
    version,
    about = "Offline adaptive PDF to Word — Rust"
)]
struct Args {
    #[arg(long, global = true)]
    settings: Option<PathBuf>,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    Doctor,
    RegisterReader {
        spec: PathBuf,
    },
    CheckUpdate {
        id: String,
    },
    UpdateModel {
        id: String,
    },
    Writers,
    AddWriter {
        name: String,
    },
    Teach {
        writer: String,
        image: PathBuf,
        label: String,
        #[arg(long)]
        glyph: bool,
    },
    FontPack {
        writer: String,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        family: Option<String>,
    },
    ExportWriter {
        writer: String,
        #[arg(short, long)]
        output: PathBuf,
    },
    ImportWriter {
        input: PathBuf,
    },
    Questions {
        project: PathBuf,
    },
    Clarify {
        project: PathBuf,
        answers: PathBuf,
    },
    Reconsider {
        project: PathBuf,
    },
    Plan {
        #[arg(long)]
        catalog: bool,
    },
    Catalog,
    Register {
        #[arg(long)]
        id: String,
        #[arg(long)]
        weights: PathBuf,
        #[arg(long)]
        projector: PathBuf,
    },
    Convert {
        pdf: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        preset: Option<String>,
        #[arg(long)]
        handwriting: bool,
        #[arg(long)]
        pages: Option<String>,
    },
    Export {
        project: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    Inspect {
        project: PathBuf,
    },
    Defaults {
        #[arg(short, long)]
        output: PathBuf,
    },
    SelfTest {
        #[arg(short, long)]
        output: PathBuf,
    },
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e:#}");
        if let Some(f) = e.downcast_ref::<Fault>() {
            eprintln!("Fix: {}", f.remedy());
        }
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args = Args::parse();
    let mut s = if let Some(p) = args.settings {
        serde_json::from_reader(fs::File::open(p)?)?
    } else {
        Settings::default()
    };
    s.validate()?;
    let c = Control::default();
    let log = Activity {
        path: None,
        callback: Some(Arc::new(|e| {
            println!(
                "[{}] {}/{} p{} {} {}",
                e.stage, e.done, e.total, e.page, e.code, e.message
            )
        })),
    };
    match args.command {
        Action::RegisterReader { spec } => {
            let model = read_json(&spec, 100_000)?;
            let m = notegen::models::save_custom(&s.models_dir, model, &c)?;
            println!("Registered {}", m.id);
        }
        Action::CheckUpdate { id } => {
            let model = runtime::inventory(&s.models_dir)?
                .into_iter()
                .find(|m| m.id == id)
                .ok_or_else(|| Fault::new("NG650", "Reader is not registered"))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&notegen::models::check(&model, &c)?)?
            );
        }
        Action::UpdateModel { id } => {
            let model = runtime::inventory(&s.models_dir)?
                .into_iter()
                .find(|m| m.id == id)
                .ok_or_else(|| Fault::new("NG650", "Reader is not registered"))?;
            let update = notegen::models::check(&model, &c)?;
            let installed = notegen::models::install(&s.models_dir, &model, &update, &c, &log)?;
            println!("Activated {} at {}", installed.id, installed.revision);
        }
        Action::Writers => println!(
            "{}",
            serde_json::to_string_pretty(&notegen::writers::list(&s.writer.root)?)?
        ),
        Action::AddWriter { name } => println!(
            "{}",
            serde_json::to_string_pretty(&notegen::writers::create(&s.writer.root, &name)?)?
        ),
        Action::Teach {
            writer,
            image,
            label,
            glyph,
        } => {
            let image = notegen::writers::open_image(&image, 40_000_000)?;
            let p = notegen::writers::add_example(
                &s.writer.root,
                &writer,
                &image,
                BoxRect::full(),
                &label,
                glyph,
                "CLI confirmed example",
            )?;
            println!("Saved example for {} (revision {})", p.name, p.revision);
        }
        Action::FontPack {
            writer,
            output,
            family,
        } => {
            let mut options = notegen::fontpack::FontOptions::default();
            if let Some(family) = family {
                options.family = family;
            }
            let n = notegen::fontpack::export(&s.writer.root, &writer, &output, &options, &c)?;
            println!("Exported {n} glyphs: {}", output.display());
        }
        Action::ExportWriter { writer, output } => {
            notegen::writers::export_profile(&s.writer.root, &writer, &output, &c)?
        }
        Action::ImportWriter { input } => println!(
            "{}",
            serde_json::to_string_pretty(&notegen::writers::import_profile(
                &s.writer.root,
                &input,
                &c
            )?)?
        ),
        Action::Questions { project } => {
            let project = Project::read(project)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&notegen::assistance::questions(
                    &project,
                    32,
                    s.review_threshold
                )?)?
            );
        }
        Action::Clarify { project, answers } => {
            let project = Project::read(project)?;
            let questions = notegen::assistance::questions(&project, 32, s.review_threshold)?;
            let answers: Vec<notegen::assistance::Answer> = read_json(&answers, 4_000_000)?;
            println!(
                "Applied {} corrections",
                notegen::assistance::apply(&project, &questions, &answers)?
            );
        }
        Action::Reconsider { project } => {
            let project = Project::read(project)?;
            pipeline::reconsider(&project, &project.manifest.pages, &s, &c, &log)?;
        }
        Action::Doctor => {
            let h = hardware::detect_for(&s, &c);
            println!("{}", serde_json::to_string_pretty(&h)?);
            println!(
                "Installed readers: {}",
                runtime::inventory(&s.models_dir)?.len()
            );
            println!("Runtime: {}", s.server.display());
        }
        Action::Plan { catalog } => {
            let models = if catalog {
                planning::catalog()
            } else {
                runtime::inventory(&s.models_dir)?
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&planning::build(
                    &hardware::detect_for(&s, &c),
                    &s,
                    &models
                )?)?
            );
        }
        Action::Catalog => println!("{}", serde_json::to_string_pretty(&planning::catalog())?),
        Action::Register {
            id,
            weights,
            projector,
        } => {
            let m = runtime::register(&s.models_dir, &id, &weights, &projector)?;
            println!(
                "Registered {} with verified local GGUFs and checksum {}",
                m.name, m.sha256
            );
        }
        Action::Convert {
            pdf,
            output,
            preset,
            handwriting,
            pages,
        } => {
            if let Some(p) = preset {
                s.preset = match p.as_str() {
                    "fast" => Preset::Fast,
                    "balanced" => Preset::Balanced,
                    "accuracy" => Preset::Accuracy,
                    "maximum" => Preset::Maximum,
                    _ => {
                        return Err(Fault::new(
                            "NG102",
                            "Preset must be fast, balanced, accuracy or maximum",
                        )
                        .into());
                    }
                };
            }
            s.handwriting |= handwriting;
            if let Some(p) = pages {
                s.page_range = p;
            }
            let r = pipeline::convert(&pdf, &output, &s, &c, &log)?;
            println!(
                "Saved: {}\nProject: {}\nPages: {}\nReview: {}\nSeconds: {:.2}",
                r.output.display(),
                r.project.root.display(),
                r.pages,
                r.review,
                r.seconds
            );
        }
        Action::Export { project, output } => {
            word::export(&Project::read(project)?, &output, &c)?;
            println!("Saved {}", output.display());
        }
        Action::Inspect { project } => {
            let p = Project::read(project)?;
            for n in &p.manifest.pages {
                let page = p.load(*n)?;
                println!(
                    "Page {} · {} candidates · agreement {:.1}% · review {}",
                    n,
                    page.candidates.len(),
                    page.agreement * 100.,
                    page.needs_review(p.manifest.settings.review_threshold)
                );
            }
        }
        Action::Defaults { output } => atomic_json(&output, &s)?,
        Action::SelfTest { output } => {
            let temp = tempfile::tempdir()?;
            let p = Project::open(
                temp.path().into(),
                Manifest {
                    schema: 2,
                    input_sha256: "synthetic".into(),
                    fingerprint: "synthetic".into(),
                    pages: vec![1],
                    models: vec![],
                    settings: s,
                },
            )?;
            image::RgbImage::from_pixel(300, 400, image::Rgb([255, 255, 255])).save(p.image(1))?;
            p.save(&Page {
                page: 1,
                image: "page-00001.png".into(),
                content: PageContent {
                    blocks: vec![
                        Block::text("NoteGen Rust native export"),
                        Block {
                            kind: "table".into(),
                            rows: vec![
                                vec!["Item".into(), "Amount".into()],
                                vec!["Sales".into(), "1250".into()],
                            ],
                            ..Default::default()
                        },
                        Block {
                            kind: "figure".into(),
                            bbox: Some(BoxRect::full()),
                            description: "Synthetic source figure".into(),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                ..Default::default()
            })?;
            word::export(&p, &output, &c)?;
            println!("Native Word export passed: {}", output.display());
        }
    }
    Ok(())
}
