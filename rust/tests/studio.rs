use image::{DynamicImage, GrayImage, Luma};
use notegen::{
    fontpack::{self, FontOptions},
    storage::Control,
    types::*,
    writers,
};
use std::io::Read;
fn ink() -> DynamicImage {
    let mut im = GrayImage::from_pixel(100, 120, Luma([255]));
    for y in 20..100 {
        for x in 20..65 {
            if x < 30 || y < 32 || (y > 50 && y < 62) {
                im[(x, y)] = Luma([0]);
            }
        }
    }
    DynamicImage::ImageLuma8(im)
}
#[test]
fn confirmed_writer_memory_is_persistent_bounded_and_removable() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let p = writers::create(root, "Ada").unwrap();
    assert!(writers::read(root, "../Ada").is_err());
    assert!(writers::reference(root, &p.id).unwrap().is_none());
    let before = writers::digest(root, Some(&p.id)).unwrap();
    let p =
        writers::add_example(root, &p.id, &ink(), BoxRect::full(), "r", true, "test page").unwrap();
    assert_eq!(p.samples.len(), 1);
    assert_eq!(writers::coverage(&p), "r");
    let twice =
        writers::add_example(root, &p.id, &ink(), BoxRect::full(), "r", true, "duplicate").unwrap();
    assert_eq!(twice.samples.len(), 1);
    assert_eq!(twice.revision, p.revision);
    assert_ne!(before, writers::digest(root, Some(&p.id)).unwrap());
    let sheet = writers::reference(root, &p.id).unwrap().unwrap();
    assert!(sheet.description.contains("column 1 = \"r\""));
    assert_eq!(image::load_from_memory(&sheet.png).unwrap().width(), 640);
    assert!(writers::add_example(root, &p.id, &ink(), BoxRect::full(), "word", true, "").is_err());
    assert!(
        writers::add_example(
            root,
            &p.id,
            &DynamicImage::new_rgb8(100, 100),
            BoxRect::full(),
            "x",
            true,
            ""
        )
        .is_err()
    );
    let p = writers::delete_example(root, &p.id, &p.samples[0].id).unwrap();
    assert!(p.samples.is_empty());
    writers::remove(root, &p.id).unwrap();
    assert!(writers::list(root).unwrap().is_empty());
}
#[test]
fn actual_truetype_font_pack_has_only_confirmed_glyphs_and_valid_outlines() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let p = writers::create(root, "Font test").unwrap();
    writers::add_example(root, &p.id, &ink(), BoxRect::full(), "r", true, "").unwrap();
    writers::add_example(root, &p.id, &ink(), BoxRect::full(), "term", false, "").unwrap();
    let output = root.join("font.zip");
    assert_eq!(
        fontpack::export(
            root,
            &p.id,
            &output,
            &FontOptions::default(),
            &Control::default()
        )
        .unwrap(),
        1
    );
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
    let mut bytes = vec![];
    zip.by_name("Handwriting.ttf")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    let face = ttf_parser::Face::parse(&bytes, 0).unwrap();
    assert_eq!(face.units_per_em(), 1000);
    assert_eq!(face.number_of_glyphs(), 3);
    let r = face.glyph_index('r').unwrap();
    assert!(face.glyph_bounding_box(r).is_some());
    assert!(face.glyph_index('a').is_none());
    assert!(face.glyph_index(' ').is_some());
    assert!(face.glyph_hor_advance(r).unwrap() > 100);
    let sum = bytes.chunks(4).fold(0u32, |s, c| {
        let mut a = [0; 4];
        a[..c.len()].copy_from_slice(c);
        s.wrapping_add(u32::from_be_bytes(a))
    });
    assert_eq!(sum, 0xb1b0afba);
    assert!(zip.by_name("svg/U0072.svg").is_ok());
    assert!(zip.by_name("coverage.png").is_ok());
    assert!(
        fontpack::export(
            root,
            &p.id,
            &output,
            &FontOptions::default(),
            &Control::default()
        )
        .is_err()
    );
    if let Ok(dir) = std::env::var("NOTEGEN_FONT_EVIDENCE") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(std::path::Path::new(&dir).join("NoteGen-test.ttf"), bytes).unwrap();
    }
}
#[test]
fn empty_and_cancelled_font_exports_are_not_published() {
    let temp = tempfile::tempdir().unwrap();
    let p = writers::create(temp.path(), "Empty").unwrap();
    let output = temp.path().join("empty.zip");
    assert!(
        fontpack::export(
            temp.path(),
            &p.id,
            &output,
            &FontOptions::default(),
            &Control::default()
        )
        .is_err()
    );
    assert!(!output.exists());
    writers::add_example(temp.path(), &p.id, &ink(), BoxRect::full(), "R", true, "").unwrap();
    let c = Control::default();
    c.cancelled
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(fontpack::export(temp.path(), &p.id, &output, &FontOptions::default(), &c).is_err());
    assert!(!output.exists());
}

fn example_project(root: &std::path::Path) -> notegen::storage::Project {
    use notegen::storage::*;
    Project::open(
        root.into(),
        Manifest {
            schema: 2,
            input_sha256: "fixture".into(),
            fingerprint: "fixture".into(),
            pages: vec![1],
            models: vec![],
            settings: Settings::default(),
        },
    )
    .unwrap()
}
#[test]
fn clarification_preserves_evidence_and_edits_exact_numbers_and_table_cells() {
    use notegen::assistance as a;
    let temp = tempfile::tempdir().unwrap();
    let project = example_project(temp.path());
    let blocks = vec![
        Block {
            uncertain: true,
            ..Block::text("Rate is 1[illegible]5")
        },
        Block {
            kind: "table".into(),
            rows: vec![vec!["[illegible]".into(), "1250".into()]],
            uncertain: true,
            ..Default::default()
        },
        Block {
            kind: "figure".into(),
            bbox: Some(BoxRect::full()),
            description: "Visible arrow".into(),
            uncertain: true,
            ..Default::default()
        },
    ];
    project
        .save(&Page {
            page: 1,
            image: "page-00001.png".into(),
            content: PageContent {
                blocks,
                ..Default::default()
            },
            candidates: vec![Candidate {
                model_id: "reader".into(),
                family: "one".into(),
                view: "full".into(),
                region: None,
                reliability: 0.7,
                content: PageContent {
                    blocks: vec![Block::text("original")],
                    ..Default::default()
                },
            }],
            ..Default::default()
        })
        .unwrap();
    let qs = a::questions(&project, 8, 0.9).unwrap();
    assert_eq!(qs.len(), 3);
    assert!(qs[0].prompt.contains("numbers"));
    assert_eq!(qs[1].field, "cell:0:0");
    let answers = vec![
        a::Answer {
            id: qs[0].id.clone(),
            replacement: "Rate is 105".into(),
            context: "Physics notes".into(),
            ..Default::default()
        },
        a::Answer {
            id: qs[1].id.clone(),
            replacement: "Revenue".into(),
            ..Default::default()
        },
        a::Answer {
            id: qs[2].id.clone(),
            replacement: "Water flows from tank A to tank B".into(),
            ..Default::default()
        },
    ];
    assert_eq!(a::apply(&project, &qs, &answers).unwrap(), 3);
    let page = project.load(1).unwrap();
    assert_eq!(page.content.blocks[0].text, "Rate is 105");
    assert_eq!(page.content.blocks[1].rows[0][1], "1250");
    assert_eq!(page.candidates[0].content.plain(), "original");
    assert!(page.edited && !page.reviewed);
    assert_eq!(a::load_guidance(&project).unwrap().notes.len(), 1);
    assert!(a::apply(&project, &qs, &answers).is_err());
    assert!(a::questions(&project, 8, 0.9).unwrap().is_empty());
}
#[test]
fn knowledge_lookup_requires_opt_in_and_parses_sourced_plain_text() {
    use notegen::assistance as a;
    assert!(
        a::search("topic", false, &Control::default())
            .unwrap_err()
            .to_string()
            .contains("NG640")
    );
    let sources=a::parse_search(br#"{"query":{"search":[{"title":"Water","pageid":42,"snippet":"The <span class=\"searchmatch\">water</span> cycle &amp; energy."}]}}"#).unwrap();
    assert_eq!(sources[0].url, "https://en.wikipedia.org/?curid=42");
    assert_eq!(sources[0].excerpt, "The water cycle & energy.");
    assert!(a::parse_search(br#"{"error":{"code":"limited"}}"#).is_err());
}
#[test]
fn custom_readers_and_updates_validate_sources_hashes_and_rollback() {
    use notegen::{models as m, planning, runtime};
    let temp = tempfile::tempdir().unwrap();
    let mut custom = planning::catalog().remove(0);
    custom.id = "my-reader".into();
    custom.name = "My vision model".into();
    custom.family = "my-family".into();
    custom.prompt = "structured".into();
    custom.role = "vision".into();
    custom.endpoint = Some("http://127.0.0.1:1234".into());
    custom.alias = "my-vision:latest".into();
    custom.revision = "local-1".into();
    let installed = m::save_custom(temp.path(), custom.clone(), &Control::default()).unwrap();
    assert!(installed.weights.as_os_str().is_empty());
    assert_eq!(runtime::inventory(temp.path()).unwrap().len(), 1);
    custom.revision = "local-2".into();
    m::save_custom(temp.path(), custom.clone(), &Control::default()).unwrap();
    assert_eq!(
        m::rollback(temp.path(), "my-reader").unwrap().revision,
        "local-1"
    );
    custom.endpoint = Some("https://example.com".into());
    assert!(m::save_custom(temp.path(), custom.clone(), &Control::default()).is_err());
    custom.endpoint = None;
    custom.hub = Some(m::Hub {
        repository: "owner/repo".into(),
        weights: "reader.gguf".into(),
        projector: "vision.gguf".into(),
    });
    let data = serde_json::json!({"sha":"a".repeat(40),"siblings":[{"rfilename":"reader.gguf","lfs":{"sha256":"b".repeat(64),"size":100}},{"rfilename":"vision.gguf","lfs":{"sha256":"c".repeat(64),"size":200}}]});
    let update = m::parse_update(&custom, &data).unwrap();
    assert_eq!(update.weights.bytes, 100);
    let mut incomplete = data.clone();
    incomplete["siblings"][1]["lfs"]["sha256"] = serde_json::json!("");
    assert!(m::parse_update(&custom, &incomplete).is_err());
    custom.hub.as_mut().unwrap().weights = "../outside.gguf".into();
    assert!(m::validate_model(&custom).is_err());
    assert!(m::content_range("bytes 5-19/20", 5, 20));
    assert!(!m::content_range("bytes 0-19/20", 5, 20));
}

#[test]
fn non_origin_crops_are_tight_and_profiles_round_trip_without_font_glyphs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let p = writers::create(root, "Words only").unwrap();
    let image = ink();
    let region = BoxRect {
        left: 0.15,
        top: 0.10,
        right: 0.75,
        bottom: 0.90,
    };
    let p = writers::add_example(root, &p.id, &image, region, "river", false, "").unwrap();
    let pack = root.join("profile.zip");
    writers::export_profile(root, &p.id, &pack, &Control::default()).unwrap();
    let imported = writers::import_profile(root, &pack, &Control::default()).unwrap();
    assert_ne!(p.id, imported.id);
    assert_eq!(imported.samples[0].label, "river");
    assert!(writers::coverage(&imported).is_empty());
    assert!(
        writers::reference(root, &imported.id)
            .unwrap()
            .unwrap()
            .description
            .contains("river")
    );
    let malformed = root.join("bad.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&malformed).unwrap());
    let mut bad = serde_json::to_value(&p).unwrap();
    bad["samples"][0]["image"] = serde_json::json!("../../outside.png");
    zip.start_file("profile.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(&mut zip, &serde_json::to_vec(&bad).unwrap()).unwrap();
    zip.finish().unwrap();
    assert!(writers::import_profile(root, &malformed, &Control::default()).is_err());
}
#[test]
fn contextual_re_readings_never_overwrite_confirmed_text_or_add_independent_votes() {
    use notegen::{pipeline, planning};
    let model = planning::catalog()
        .into_iter()
        .find(|m| m.role == "vision")
        .unwrap();
    let mut page = Page {
        edited: true,
        agreement: 0.66,
        content: PageContent {
            blocks: vec![Block::text("User confirmed 105")],
            ..Default::default()
        },
        ..Default::default()
    };
    let fresh = PageContent {
        blocks: vec![Block::text("AI guessed 155")],
        ..Default::default()
    };
    pipeline::attach_context_reading(&mut page, &fresh, &model);
    pipeline::attach_context_reading(&mut page, &fresh, &model);
    assert_eq!(page.content.blocks[0].text, "User confirmed 105");
    assert_eq!(page.content.blocks[0].alternatives, vec!["AI guessed 155"]);
    assert_eq!(page.agreement, 0.66);
    assert_eq!(page.content.warnings.len(), 1);
    assert!(page.candidates.iter().all(|c| c.family == model.family));
}
#[test]
fn clarification_validation_rejects_invalid_learning_before_any_write() {
    use notegen::assistance as a;
    let q = a::Question {
        id: "question".into(),
        page: 1,
        block: 0,
        field: "text".into(),
        prompt: "Read this".into(),
        reading: "word".into(),
        alternatives: vec![],
        region: Some(BoxRect::full()),
        image: "page.png".into(),
        block_digest: "hash".into(),
    };
    let mut answer = a::Answer {
        id: q.id.clone(),
        replacement: "two letters".into(),
        learn: true,
        glyph: true,
        ..Default::default()
    };
    assert!(a::validate_answers(std::slice::from_ref(&q), std::slice::from_ref(&answer)).is_err());
    answer.glyph = false;
    assert!(a::validate_answers(std::slice::from_ref(&q), std::slice::from_ref(&answer)).is_ok());
    assert!(a::validate_answers(&[q], &[answer.clone(), answer]).is_err());
}
#[test]
fn healthy_models_remain_available_when_an_unrelated_descriptor_is_broken() {
    use notegen::{models, planning, runtime};
    let t = tempfile::tempdir().unwrap();
    let mut m = planning::catalog().remove(3);
    m.id = "local-test".into();
    m.endpoint = Some("http://127.0.0.1:1234".into());
    m.alias = "reader".into();
    models::save_custom(t.path(), m, &Control::default()).unwrap();
    std::fs::write(t.path().join("bad.notegen.json"), b"broken").unwrap();
    let (models, errors) = runtime::inventory_available(t.path()).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(errors.len(), 1);
    assert!(runtime::inventory(t.path()).is_err());
}
#[test]
fn visual_writer_examples_reach_a_third_party_local_server_as_a_separate_image() {
    use notegen::{hardware, planning, runtime, storage::Activity};
    use std::{io::Write, net::TcpListener};
    let temp = tempfile::tempdir().unwrap();
    let p = writers::create(temp.path(), "Visual memory").unwrap();
    writers::add_example(temp.path(), &p.id, &ink(), BoxRect::full(), "r", true, "").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let t = std::thread::spawn(move || {
        let mut request = serde_json::Value::Null;
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut headers = vec![];
            let mut byte = [0];
            while socket.read_exact(&mut byte).is_ok() {
                headers.push(byte[0]);
                if headers.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let header = String::from_utf8(headers).unwrap();
            let size = header
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let mut body = vec![0; size];
            socket.read_exact(&mut body).unwrap();
            if header.starts_with("GET ") {
                let body = r#"{"data":[{"id":"vision:test"}]}"#;
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            } else {
                request = serde_json::from_slice(&body).unwrap();
                let data=serde_json::json!({"blocks":[{"kind":"text","text":"Target 1250"}],"warnings":[],"summary":"Target"}).to_string();
                let packet=serde_json::json!({"choices":[{"delta":{"content":data},"finish_reason":"stop"}]}).to_string();
                let out = format!("data: {packet}\n\n");
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",out.len()).unwrap();
            }
        }
        request
    });
    let mut model = planning::catalog().remove(3);
    model.endpoint = Some(endpoint);
    model.alias = "vision:test".into();
    let s = Settings {
        backend: Backend::Cpu,
        writer_reference: writers::reference(temp.path(), &p.id).unwrap(),
        ..Default::default()
    };
    let plan = planning::build(&hardware::detect(), &s, std::slice::from_ref(&model)).unwrap();
    let server = runtime::Server::start(
        &model,
        &plan,
        &s,
        &Control::default(),
        &Activity::default(),
        temp.path(),
    )
    .unwrap();
    assert!(!server.acceleration_verified);
    let read = server
        .read(
            &ink(),
            "",
            "Topic data",
            &s,
            &Control::default(),
            &Activity::default(),
            1,
        )
        .unwrap();
    assert!(read.plain().contains("1250"));
    let body = t.join().unwrap();
    assert_eq!(body["model"], "vision:test");
    assert!(body.get("chat_template_kwargs").is_none());
    assert_eq!(
        body["response_format"],
        serde_json::json!({"type":"json_object"})
    );
    let content = body["messages"][0]["content"].as_array().unwrap();
    assert_eq!(
        content.iter().filter(|v| v["type"] == "image_url").count(),
        2
    );
    assert!(content.iter().any(|v| v["text"].as_str().is_some_and(|s| {
        s.contains("Confirmed handwriting examples") && s.contains("column 1 = \"r\"")
    })));
}

#[test]
fn contextual_reread_previews_its_own_page_and_respects_the_disabled_setting() {
    use notegen::{models, pipeline, planning, storage::*};
    use std::{
        io::Write,
        net::TcpListener,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    for enabled in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let start = Instant::now();
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && start.elapsed() < Duration::from_secs(10) =>
                        {
                            std::thread::sleep(Duration::from_millis(5))
                        }
                        Err(e) => {
                            panic!("Local test server did not receive the expected request: {e}")
                        }
                    }
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut headers = vec![];
                let mut byte = [0];
                while socket.read_exact(&mut byte).is_ok() {
                    headers.push(byte[0]);
                    assert!(headers.len() < 16000);
                    if headers.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let headers = String::from_utf8(headers).unwrap();
                let size = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|n| n.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                assert!(size < 1_000_000);
                let mut body = vec![0; size];
                socket.read_exact(&mut body).unwrap();
                let response = if headers.starts_with("GET ") {
                    serde_json::json!({"data":[{"id":"preview-reader"}]}).to_string()
                } else {
                    let content=serde_json::json!({"blocks":[{"kind":"text","text":"Alternative 1250"}],"warnings":[],"summary":"Alternative"}).to_string();
                    serde_json::json!({"choices":[{"message":{"content":content},"finish_reason":"stop"}]}).to_string()
                };
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
            }
        });
        let mut settings = Settings {
            backend: Backend::Cpu,
            models_dir: temp.path().join("models"),
            ..Default::default()
        };
        settings.experience.live_preview = enabled;
        let mut model = planning::catalog()
            .into_iter()
            .find(|m| m.role == "vision" && m.prompt == "structured")
            .unwrap();
        model.id = "preview-reader".into();
        model.name = "Preview test reader".into();
        model.family = "preview-test".into();
        model.endpoint = Some(endpoint);
        model.alias = "preview-reader".into();
        models::save_custom(&settings.models_dir, model, &Control::default()).unwrap();
        let project = Project::open(
            temp.path().join("project"),
            Manifest {
                schema: 2,
                input_sha256: "input".into(),
                fingerprint: "fixture".into(),
                pages: vec![1, 2],
                models: vec![],
                settings: settings.clone(),
            },
        )
        .unwrap();
        ink().save(project.image(2)).unwrap();
        project
            .save(&Page {
                page: 2,
                image: "page-00002.png".into(),
                edited: true,
                content: PageContent {
                    blocks: vec![Block::text("Confirmed 1250")],
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let log = Activity {
            path: None,
            callback: Some(Arc::new(move |e| {
                if let Some(p) = e.preview {
                    captured.lock().unwrap().push(p);
                }
            })),
        };
        let result = pipeline::reconsider(&project, &[2], &settings, &Control::default(), &log);
        server.join().unwrap();
        assert_eq!(result.unwrap(), 1);
        let events = events.lock().unwrap();
        if enabled {
            assert_eq!(
                events.first().unwrap().image.as_ref(),
                Some(&project.image(2))
            );
            assert_eq!(events.first().unwrap().page, 2);
            assert_eq!(events.first().unwrap().reader, "Preview test reader");
            assert!(events.iter().skip(1).any(|p| {
                p.page == 2
                    && p.draft
                        .as_ref()
                        .is_some_and(|d| d.contains("Alternative 1250"))
            }));
        } else {
            assert!(events.is_empty());
        }
        let page = project.load(2).unwrap();
        assert_eq!(page.content.plain(), "Confirmed 1250");
        assert!(
            page.candidates
                .iter()
                .any(|c| c.content.plain().contains("Alternative 1250"))
        );
    }
}

#[test]
fn managed_custom_reader_uses_its_registered_alias_and_extreme_glyphs_stay_in_bounds() {
    use notegen::{models, planning};
    let t = tempfile::tempdir().unwrap();
    let weights = t.path().join("weights.gguf");
    let projector = t.path().join("vision.gguf");
    std::fs::write(&weights, b"GGUFweights").unwrap();
    std::fs::write(&projector, b"GGUFprojector").unwrap();
    let mut model = planning::catalog().remove(3);
    model.id = "custom-managed".into();
    model.alias = "old-local-service-alias".into();
    model.weights = weights;
    model.projector = projector;
    let registered = models::save_custom(t.path(), model, &Control::default()).unwrap();
    assert!(registered.alias.is_empty());
    let p = writers::create(t.path(), "Wide ink").unwrap();
    let mut image = GrayImage::from_pixel(300, 30, Luma([255]));
    for x in 5..295 {
        image[(x, 15)] = Luma([0]);
    }
    writers::add_example(
        t.path(),
        &p.id,
        &DynamicImage::ImageLuma8(image),
        BoxRect::full(),
        "-",
        true,
        "",
    )
    .unwrap();
    let pack = t.path().join("wide.zip");
    fontpack::export(
        t.path(),
        &p.id,
        &pack,
        &FontOptions::default(),
        &Control::default(),
    )
    .unwrap();
    let mut z = zip::ZipArchive::new(std::fs::File::open(pack).unwrap()).unwrap();
    let mut bytes = vec![];
    z.by_name("Handwriting.ttf")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    let f = ttf_parser::Face::parse(&bytes, 0).unwrap();
    let bbox = f.glyph_bounding_box(f.glyph_index('-').unwrap()).unwrap();
    assert!(bbox.x_max <= 2000 && bbox.y_max <= 800);
}
