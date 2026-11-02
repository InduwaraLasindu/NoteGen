use notegen::{
    assistance as a, pipeline, planning, review,
    storage::{Activity, Control, Manifest, Project, fingerprint},
    types::*,
};
use std::fs;
struct Fixture {
    _temp: tempfile::TempDir,
    project: Project,
}
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let project = Project::open(
        temp.path().join("project"),
        Manifest {
            schema: 2,
            input_sha256: "input".into(),
            fingerprint: "settings".into(),
            pages: vec![1, 2],
            models: vec![],
            settings: Settings::default(),
        },
    )
    .unwrap();
    for n in [1, 2] {
        project
            .save(&Page {
                page: n,
                image: format!("page-{n:05}.png"),
                content: PageContent {
                    blocks: vec![Block {
                        text: format!("Unclear {n}05"),
                        kind: "text".into(),
                        bbox: Some(BoxRect::full()),
                        uncertain: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
    }
    Fixture {
        _temp: temp,
        project,
    }
}
fn answers(qs: &[a::Question]) -> Vec<a::Answer> {
    qs.iter()
        .map(|q| a::Answer {
            id: q.id.clone(),
            replacement: format!("Confirmed {}05", q.page),
            context: format!("Page {} is physics", q.page),
            ..Default::default()
        })
        .collect()
}
fn model() -> planning::Model {
    planning::catalog()
        .into_iter()
        .find(|m| m.role == "vision")
        .unwrap()
}
#[test]
fn clarification_pages_and_guidance_commit_and_undo_as_one_revision() {
    let f = fixture();
    let p = &f.project;
    let original = fs::read(p.page_path(1)).unwrap();
    let qs = a::questions(p, 32, 0.9).unwrap();
    assert_eq!(qs.len(), 2);
    assert_eq!(a::apply(p, &qs, &answers(&qs)).unwrap(), 2);
    assert_eq!(p.load(1).unwrap().content.plain(), "Confirmed 105");
    assert_eq!(a::load_guidance(p).unwrap().notes.len(), 2);
    assert_eq!(a::load_guidance(p).unwrap().answers.len(), 2);
    assert_eq!(review::timeline(p).unwrap().revisions.len(), 1);
    assert_eq!(fs::read(p.page_path(1)).unwrap(), original);
    review::undo(p).unwrap();
    assert_eq!(p.load(1).unwrap().content.plain(), "Unclear 105");
    assert_eq!(p.load(2).unwrap().content.plain(), "Unclear 205");
    assert!(a::load_guidance(p).unwrap().answers.is_empty());
    assert!(!p.root.join("guidance.json").exists());
    review::redo(p).unwrap();
    assert_eq!(p.load(2).unwrap().content.plain(), "Confirmed 205");
    assert_eq!(a::load_guidance(p).unwrap().notes.len(), 2);
}
#[test]
fn stale_answer_on_a_later_page_prevents_every_correction() {
    let f = fixture();
    let p = &f.project;
    let qs = a::questions(p, 32, 0.9).unwrap();
    let mut page = p.load(2).unwrap();
    page.content.blocks[0].text = "Changed outside the inbox".into();
    p.save(&page).unwrap();
    assert!(a::apply(p, &qs, &answers(&qs)).is_err());
    assert_eq!(p.load(1).unwrap().content.plain(), "Unclear 105");
    assert!(a::load_guidance(p).unwrap().answers.is_empty());
    assert!(review::timeline(p).unwrap().revisions.is_empty());
}
#[test]
fn invalid_table_answer_and_oversized_guidance_do_not_partially_apply() {
    let f = fixture();
    let p = &f.project;
    let mut qs = a::questions(p, 32, 0.9).unwrap();
    qs[1].field = "cell:99:99".into();
    assert!(a::apply(p, &qs, &answers(&qs)).is_err());
    assert_eq!(p.load(1).unwrap().content.plain(), "Unclear 105");
    assert!(review::timeline(p).unwrap().revisions.is_empty());
    let guidance = a::Guidance {
        notes: vec!["x".repeat(4000); 1000],
        ..Default::default()
    };
    notegen::storage::atomic_json(&p.root.join("guidance.json"), &guidance).unwrap();
    assert!(a::apply(p, &qs, &answers(&qs)).is_err());
    assert_eq!(p.load(1).unwrap().content.plain(), "Unclear 105");
}
#[test]
fn notes_only_context_is_named_reversible_and_does_not_edit_pages() {
    let f = fixture();
    let p = &f.project;
    let before = fingerprint(&p.load(1).unwrap()).unwrap();
    a::add_context(p, "These are kinematics notes.").unwrap();
    let history = review::timeline(p).unwrap();
    assert_eq!(history.revisions.len(), 1);
    assert!(history.revisions[0].pages.is_empty());
    assert_eq!(fingerprint(&p.load(1).unwrap()).unwrap(), before);
    assert_eq!(a::load_guidance(p).unwrap().notes.len(), 1);
    review::undo(p).unwrap();
    assert!(a::load_guidance(p).unwrap().notes.is_empty());
    review::redo(p).unwrap();
    assert_eq!(
        a::load_guidance(p).unwrap().notes[0],
        "These are kinematics notes."
    );
}
#[test]
fn concurrent_context_writes_are_rejected_by_expected_snapshot() {
    let f = fixture();
    let p = &f.project;
    let stale = a::load_guidance(p).unwrap();
    a::add_context(p, "Newest context").unwrap();
    let mut proposed = stale.clone();
    proposed.notes.push("Stale context".into());
    assert!(review::commit_guidance(p, &[], &stale, &proposed, "Stale notes").is_err());
    assert_eq!(a::load_guidance(p).unwrap().notes, vec!["Newest context"]);
}
#[test]
fn skipped_answers_and_empty_batches_do_not_create_revisions() {
    let f = fixture();
    let p = &f.project;
    let qs = a::questions(p, 32, 0.9).unwrap();
    let skipped = qs
        .iter()
        .map(|q| a::Answer {
            id: q.id.clone(),
            skip: true,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    assert_eq!(a::apply(p, &qs, &skipped).unwrap(), 0);
    assert_eq!(a::apply(p, &[], &[]).unwrap(), 0);
    assert!(review::timeline(p).unwrap().revisions.is_empty());
}
#[test]
fn contextual_reader_evidence_and_human_text_have_reversible_comparison() {
    let f = fixture();
    let p = &f.project;
    let before = p.load(1).unwrap();
    let mut confirmed = before.clone();
    confirmed.content.blocks[0].text = "Confirmed 105".into();
    confirmed.reviewed = true;
    review::commit(p, &before, &confirmed, "Confirmed source").unwrap();
    let before = p.load(1).unwrap();
    let content = PageContent {
        blocks: vec![Block::text("AI guessed 155")],
        ..Default::default()
    };
    let id = pipeline::save_context_reading(p, &before, &content, &model())
        .unwrap()
        .unwrap();
    let after = p.load(1).unwrap();
    assert_eq!(after.content.blocks[0].text, "Confirmed 105");
    assert_eq!(after.candidates.len(), 1);
    assert!(
        after.content.blocks[0]
            .alternatives
            .contains(&"AI guessed 155".into())
    );
    let differences = review::compare(&before, &after).unwrap();
    assert!(differences.iter().any(|d| d.field == "candidates"));
    assert!(differences.iter().any(|d| d.field == "alternatives"));
    assert_eq!(review::snapshot(p, id, 1).unwrap().candidates.len(), 1);
    review::undo(p).unwrap();
    assert!(p.load(1).unwrap().candidates.is_empty());
    assert_eq!(p.load(1).unwrap().content.blocks[0].text, "Confirmed 105");
}
#[test]
fn stale_contextual_results_do_not_overwrite_newer_corrections() {
    let f = fixture();
    let p = &f.project;
    let stale = p.load(1).unwrap();
    let mut current = stale.clone();
    current.content.blocks[0].text = "Newest human correction".into();
    review::commit(p, &stale, &current, "Human correction").unwrap();
    let content = PageContent {
        blocks: vec![Block::text("Late AI answer")],
        ..Default::default()
    };
    assert!(pipeline::save_context_reading(p, &stale, &content, &model()).is_err());
    assert_eq!(
        p.load(1).unwrap().content.blocks[0].text,
        "Newest human correction"
    );
    assert!(p.load(1).unwrap().candidates.is_empty());
}
#[test]
fn crop_readings_map_to_page_coordinates_without_overwriting_confirmed_values() {
    let mut page = Page {
        page: 1,
        edited: true,
        reviewed: true,
        content: PageContent {
            blocks: vec![Block {
                text: "Confirmed 105".into(),
                kind: "text".into(),
                bbox: Some(BoxRect::full()),
                ..Default::default()
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    let region = BoxRect {
        left: 0.2,
        top: 0.3,
        right: 0.8,
        bottom: 0.9,
    };
    let content = PageContent {
        blocks: vec![Block {
            text: "Alternative 155".into(),
            kind: "text".into(),
            bbox: Some(BoxRect {
                left: 0.1,
                top: 0.2,
                right: 0.9,
                bottom: 0.8,
            }),
            ..Default::default()
        }],
        ..Default::default()
    };
    pipeline::attach_region_reading(&mut page, &region, &content, &model()).unwrap();
    assert_eq!(page.content.blocks[0].text, "Confirmed 105");
    assert!(!page.reviewed);
    assert!(page.content.blocks[0].uncertain);
    let bbox = page.candidates[0].content.blocks[0].bbox.as_ref().unwrap();
    assert!((bbox.left - 0.26).abs() < 0.00001);
    assert!((bbox.top - 0.42).abs() < 0.00001);
    assert_eq!(page.candidates[0].region, Some(region));
    assert!(page.candidates[0].view.starts_with("manual-region-"));
}
#[test]
fn invalid_region_requests_and_empty_reader_output_are_rejected() {
    let f = fixture();
    let p = &f.project;
    let mut request = pipeline::RegionRequest {
        page: 1,
        region: BoxRect::full(),
        reader: "reader".into(),
        image_edge: 512,
        context: "Context".into(),
        task: "table".into(),
    };
    assert!(request.validate(p).is_ok());
    request.page = 99;
    assert!(request.validate(p).is_err());
    request.page = 1;
    request.image_edge = 8193;
    assert!(request.validate(p).is_err());
    request.image_edge = 512;
    request.task = "unsupported".into();
    assert!(request.validate(p).is_err());
    let mut page = p.load(1).unwrap();
    assert!(
        pipeline::attach_region_reading(
            &mut page,
            &BoxRect::full(),
            &PageContent::default(),
            &model()
        )
        .is_err()
    );
}
#[test]
fn exact_region_request_reaches_only_selected_loopback_reader_and_is_undoable() {
    use base64::Engine;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::{Duration, Instant},
    };
    let f = fixture();
    let p = &f.project;
    let mut source = image::RgbImage::from_pixel(200, 200, image::Rgb([0, 0, 255]));
    for y in 50..100 {
        for x in 50..150 {
            source.put_pixel(
                x,
                y,
                image::Rgb(if x < 100 { [255, 0, 0] } else { [0, 255, 0] }),
            );
        }
    }
    source.save(p.image(1)).unwrap();
    let original_checkpoint = fs::read(p.page_path(1)).unwrap();
    let unused_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    unused_listener.set_nonblocking(true).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let endpoint = format!("http://{address}");
    type Captured = (usize, Vec<serde_json::Value>);
    struct RegionMock {
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        worker: Option<std::thread::JoinHandle<Captured>>,
    }
    impl RegionMock {
        fn finish(mut self) -> Captured {
            self.stop.store(true, std::sync::atomic::Ordering::Release);
            self.worker
                .take()
                .unwrap()
                .join()
                .expect("Region HTTP mock failed")
        }
    }
    impl Drop for RegionMock {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Release);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopping = stop.clone();
    let server = RegionMock {
        stop,
        worker: Some(std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut identities = 0;
            let mut completions = vec![];
            let mut connections = 0;
            while !stopping.load(std::sync::atomic::Ordering::Acquire) {
                assert!(
                    Instant::now() < deadline,
                    "Region protocol request deadline exceeded"
                );
                let mut socket = match listener.accept() {
                    Ok((socket, _)) => socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("Region protocol accept failed: {e}"),
                };
                connections += 1;
                assert!(
                    connections <= 16,
                    "Too many region protocol transport connections"
                );
                // Winsock accepted sockets inherit listener properties. Reads
                // below are blocking with explicit deadlines, unlike accept().
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut headers = vec![];
                let mut one = [0];
                loop {
                    assert!(
                        Instant::now() < deadline,
                        "Region protocol header deadline exceeded"
                    );
                    match socket.read_exact(&mut one) {
                        Ok(()) => headers.push(one[0]),
                        Err(e)
                            if headers.is_empty()
                                && matches!(
                                    e.kind(),
                                    std::io::ErrorKind::UnexpectedEof
                                        | std::io::ErrorKind::ConnectionReset
                                        | std::io::ErrorKind::ConnectionAborted
                                ) =>
                        {
                            break;
                        }
                        Err(e) => panic!("Incomplete region HTTP headers: {e}"),
                    }
                    assert!(headers.len() < 16000);
                    if headers.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                // TCP connections are not HTTP requests: a transport may close a
                // speculative connection before writing any headers, notably on Windows.
                if headers.is_empty() {
                    continue;
                }
                let headers = String::from_utf8(headers).unwrap();
                let request_line = headers.lines().next().unwrap();
                let size = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|n| n.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                assert!(size < 1_000_000);
                let response = match request_line {
                    "GET /v1/models HTTP/1.1" => {
                        assert_eq!(size, 0);
                        identities += 1;
                        assert!(
                            identities <= 4,
                            "Unexpected repeated model identity requests"
                        );
                        serde_json::json!({"data":[{"id":"region-reader"}]}).to_string()
                    }
                    "POST /v1/chat/completions HTTP/1.1" => {
                        assert!(
                            identities > 0,
                            "Completion preceded registered-reader identity verification"
                        );
                        assert!(size > 0, "Completion request has no JSON body");
                        let mut body = vec![0; size];
                        socket.read_exact(&mut body).unwrap();
                        completions.push(serde_json::from_slice(&body).unwrap());
                        assert_eq!(
                            completions.len(),
                            1,
                            "More than one region completion was requested"
                        );
                        let content=serde_json::json!({"blocks":[{"kind":"text","text":"Alternative 155","box":{"left":0,"top":0,"right":1,"bottom":1}}],"warnings":[],"summary":"Source crop"}).to_string();
                        serde_json::json!({"choices":[{"message":{"content":content},"finish_reason":"stop"}]}).to_string()
                    }
                    other => panic!("Unexpected region HTTP request: {other}"),
                };
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
            }
            (identities, completions)
        })),
    };
    // Exercise an empty transport explicitly on every OS, rather than relying
    // on the client's platform-specific connection behavior to reproduce it.
    drop(std::net::TcpStream::connect(address).unwrap());
    let mut settings = Settings {
        models_dir: p.root.join("models"),
        backend: Backend::Cpu,
        ..Default::default()
    };
    settings.experience.live_preview = false;
    let mut reader = model();
    reader.id = "region-reader".into();
    reader.family = "region-test".into();
    reader.endpoint = Some(endpoint);
    reader.alias = "region-reader".into();
    reader.prompt = "structured".into();
    let mut unused_reader = reader.clone();
    unused_reader.id = "unused-reader".into();
    unused_reader.alias = "unused-reader".into();
    unused_reader.endpoint = Some(format!("http://{}", unused_listener.local_addr().unwrap()));
    notegen::models::save_custom(&settings.models_dir, unused_reader, &Control::default()).unwrap();
    notegen::models::save_custom(&settings.models_dir, reader, &Control::default()).unwrap();
    let request = pipeline::RegionRequest {
        page: 1,
        region: BoxRect {
            left: 0.25,
            top: 0.25,
            right: 0.75,
            bottom: 0.5,
        },
        reader: "region-reader".into(),
        image_edge: 512,
        context: "User crop topic".into(),
        task: "handwriting".into(),
    };
    let result = pipeline::reread_region(
        p,
        &request,
        &settings,
        &Control::default(),
        &Activity::default(),
    );
    let (identities, mut completions) = server.finish();
    result.unwrap();
    assert!(identities >= 1);
    assert_eq!(completions.len(), 1);
    let raw = completions.pop().unwrap();
    assert_eq!(
        unused_listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(raw["model"], "region-reader");
    let url = raw["messages"][0]["content"][0]["image_url"]["url"]
        .as_str()
        .unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(url.strip_prefix("data:image/png;base64,").unwrap())
        .unwrap();
    let image = image::load_from_memory(&bytes).unwrap().to_rgb8();
    // image_edge is a target longest edge: runtime::read resizes the 100×50 crop
    // to 512×256, preserving its aspect ratio and only the selected source pixels.
    assert_eq!((image.width(), image.height()), (512, 256));
    assert_eq!(image.get_pixel(64, 64).0, [255, 0, 0]);
    assert_eq!(image.get_pixel(448, 192).0, [0, 255, 0]);
    assert!(
        image.pixels().all(|pixel| pixel[2] == 0),
        "Source outside the requested region reached the reader"
    );
    let content = raw["messages"][0]["content"].as_array().unwrap();
    assert_eq!(
        content
            .iter()
            .filter(|item| item["type"] == "image_url")
            .count(),
        1
    );
    let prompt = content
        .iter()
        .filter_map(|item| item["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(prompt.contains("User crop topic"));
    assert!(prompt.contains("Region task: handwriting"));
    let page = p.load(1).unwrap();
    assert_eq!(page.content.blocks[0].text, "Unclear 105");
    assert_eq!(page.candidates.len(), 1);
    assert_eq!(page.candidates[0].model_id, "region-reader");
    assert_eq!(page.candidates[0].region, Some(request.region.clone()));
    assert_eq!(
        page.candidates[0].content.blocks[0].bbox,
        Some(request.region)
    );
    assert_eq!(fs::read(p.page_path(1)).unwrap(), original_checkpoint);
    review::undo(p).unwrap();
    assert!(p.load(1).unwrap().candidates.is_empty());
}
