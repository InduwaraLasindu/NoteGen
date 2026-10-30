use image::{DynamicImage, Rgb, RgbImage};
use notegen::{consensus, parsing, planning, runtime, scan, storage::*, types::*, word};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

fn candidate(id: &str, family: &str, text: &str) -> Candidate {
    Candidate {
        region: None,
        model_id: id.into(),
        family: family.into(),
        view: "full".into(),
        content: PageContent {
            blocks: vec![Block::text(text)],
            ..Default::default()
        },
        reliability: 0.9,
    }
}
fn project(root: &Path, pages: Vec<usize>) -> Project {
    Project::open(
        root.into(),
        Manifest {
            schema: 2,
            input_sha256: "a".into(),
            fingerprint: "b".into(),
            pages,
            models: vec![],
            settings: Settings::default(),
        },
    )
    .unwrap()
}
fn pdf(path: &Path, im: &RgbImage) {
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    let w = im.width();
    let h = im.height();
    let contents = format!(
        "q {} 0 0 {} 0 0 cm /Im0 Do Q",
        w as f32 * 72. / 150.,
        h as f32 * 72. / 150.
    );
    let objects=vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>",w as f32*72./150.,h as f32*72./150.).into_bytes(),[format!("<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",im.as_raw().len()).into_bytes(),im.as_raw().clone(),b"\nendstream".to_vec()].concat(),format!("<< /Length {} >>\nstream\n{}\nendstream",contents.len(),contents).into_bytes()];
    for (i, obj) in objects.into_iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(obj);
        out.extend(b"\nendobj\n");
    }
    let start = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for o in offsets.iter().skip(1) {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF",
            offsets.len()
        )
        .as_bytes(),
    );
    fs::write(path, out).unwrap();
}
fn fixture() -> RgbImage {
    let mut im = RgbImage::from_pixel(900, 1200, Rgb([255, 255, 255]));
    let font = [
        "C:/Windows/Fonts/arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]
    .iter()
    .find_map(|p| fs::read(p).ok())
    .map(|b| ab_glyph::FontArc::try_from_vec(b).unwrap())
    .expect("A test font must be installed");
    for (x, y, t) in [
        (50, 60, "NoteGen scanned page"),
        (50, 110, "Revenue is 1250 in October."),
        (80, 330, "Item"),
        (480, 330, "Amount"),
        (80, 430, "Sales"),
        (480, 430, "1250"),
    ] {
        imageproc::drawing::draw_text_mut(&mut im, Rgb([0, 0, 0]), x, y, 28., &font, t);
    }
    for y in [300, 400, 500] {
        for dy in 0..3 {
            for x in 50..850 {
                im.put_pixel(x, y + dy, Rgb([0, 0, 0]));
            }
        }
    }
    for x in [50, 450, 850] {
        for dx in 0..3 {
            for y in 300..500 {
                im.put_pixel(x + dx, y, Rgb([0, 0, 0]));
            }
        }
    }
    im
}
fn pdfium() -> PathBuf {
    std::env::var_os("NOTEGEN_PDFIUM")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/notegen-native/lib/libpdfium.so"))
}
fn serve(body: String, status: &str, delay: u64) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let status = status.to_string();
    let t = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut read = vec![0; 65536];
        let _ = stream.read(&mut read);
        thread::sleep(Duration::from_millis(delay));
        let header = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(body.as_bytes());
    });
    (endpoint, t)
}
fn sse(text: &str, finish: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        serde_json::json!({"choices":[{"delta":{"content":text}}]}),
        serde_json::json!({"choices":[{"delta":{},"finish_reason":finish}]})
    )
}
#[test]
fn lighton_grounding_protocol() {
    let p = parsing::lighton(
        "![title](10,20,900,80) ## Biology\n![text](10,100,900,200) Evaporation follows heating.\n![image](10,300,900,600) Arrow from water to vapour.",
    );
    assert_eq!(p.blocks.len(), 3);
    assert_eq!(p.blocks[0].kind, "heading");
    assert_eq!(p.blocks[2].kind, "figure");
    assert!(p.blocks[2].bbox.as_ref().unwrap().valid());
    assert!(p.validate().is_ok());
}
#[test]
fn specialist_html_table_merges() {
    let b=parsing::html_table("<table><tr><th colspan=2>Year</th></tr><tr><td rowspan=2>A</td><td>1250</td></tr><tr><td>1500</td></tr></table>").unwrap();
    assert_eq!(b.rows.len(), 3);
    assert_eq!(b.rows[2][1], "1500");
    assert_eq!(b.merges.len(), 2);
    assert_eq!(b.header_rows, 1);
}
#[test]
fn markdown_and_json_not_interchangeable() {
    let p = parsing::plain_blocks("# Heading\n\n| Item | Amount |\n|---|---|\n|Sales|1250|");
    assert!(p.iter().any(|b| b.kind == "table"));
    assert!(parsing::structured("not JSON").is_err());
    assert!(parsing::structured(r#"{"blocks":[{"kind":"figure"}]}"#).is_err());
}
#[test]
fn no_transcript_blending_and_crop_does_not_replace_page() {
    let mut cs = vec![
        candidate("a", "a", "Revenue 1250. Complete page."),
        candidate("b", "b", "Revenue 1250. Complete page."),
    ];
    let mut crop = candidate("c", "c", "1250");
    crop.view = "crop-1".into();
    cs.push(crop);
    let (p, a) = consensus::select(&cs, 2, 0.92);
    assert_eq!(p.plain(), "Revenue 1250. Complete page.");
    assert_eq!(a, 1.);
}

#[test]
fn omitted_tables_and_figures_are_not_discarded() {
    let mut a = candidate("a", "one", "Heading text");
    a.content.blocks.push(Block {
        kind: "table".into(),
        rows: vec![vec!["Amount".into()], vec!["1250".into()]],
        ..Default::default()
    });
    for (left, right) in [(0., 0.4), (0.6, 1.)] {
        a.content.blocks.push(Block {
            kind: "figure".into(),
            bbox: Some(BoxRect {
                left,
                top: 0.4,
                right,
                bottom: 0.8,
            }),
            description: "Source diagram".into(),
            ..Default::default()
        });
    }
    let b = candidate("b", "two", "Heading text");
    let (p, _) = consensus::select(&[a, b], 2, 0.92);
    assert_eq!(p.blocks.iter().filter(|b| b.kind == "figure").count(), 2);
    assert!(p.blocks.iter().any(|b| b.kind == "table"));
}
#[test]
fn runtime_device_names_and_free_memory() {
    let g = notegen::hardware::parse_devices(
        "Available devices:\nCUDA0: NVIDIA GeForce RTX 5070 Ti (16376 MiB, 15000 MiB free)\nVulkan1: AMD Radeon (32768 MiB, 30000 MiB free)",
    );
    assert_eq!(g.len(), 2);
    assert_eq!(g[1].backend, "vulkan");
    assert_eq!(g[0].index, 0);
}
#[test]
fn independent_crops_can_correct_a_region() {
    let bbox = BoxRect {
        left: 0.,
        top: 0.,
        right: 0.5,
        bottom: 0.2,
    };
    let mut b = Block::text("Revenue 1200");
    b.bbox = Some(bbox.clone());
    b.uncertain = true;
    let mut page = Page {
        content: PageContent {
            blocks: vec![b],
            ..Default::default()
        },
        ..Default::default()
    };
    for (id, family) in [("a", "one"), ("b", "two")] {
        let mut c = candidate(id, family, "Revenue 1250");
        c.view = "crop".into();
        c.region = Some(bbox.clone());
        page.candidates.push(c);
    }
    consensus::refine_regions(&mut page, &Settings::default());
    assert_eq!(page.content.plain(), "Revenue 1250");
    assert_eq!(page.content.blocks[0].alternatives.len(), 2);
}
#[test]
fn checkpoints_match_and_paths_are_local() {
    let t = tempfile::tempdir().unwrap();
    let p = project(t.path(), vec![1]);
    let page = Page {
        page: 1,
        image: "page-00001.png".into(),
        content: PageContent {
            blocks: vec![Block::text("Hello")],
            ..Default::default()
        },
        ..Default::default()
    };
    p.save(&page).unwrap();
    assert_eq!(p.load(1).unwrap().content.plain(), "Hello");
    let mut m = p.manifest.clone();
    m.fingerprint = "changed".into();
    assert!(Project::open(t.path().into(), m).is_err());
    assert!(word::safe_image(t.path(), "../x.png").is_err());
}
#[test]
fn native_word_has_real_merges_and_embedded_figure() {
    let t = tempfile::tempdir().unwrap();
    let p = project(t.path(), vec![1]);
    fixture().save(p.image(1)).unwrap();
    p.save(&Page {
        page: 1,
        image: "page-00001.png".into(),
        content: PageContent {
            blocks: vec![
                Block::text("A < B & Ω\0"),
                Block {
                    kind: "table".into(),
                    rows: vec![
                        vec!["Year".into(), "".into()],
                        vec!["A".into(), "1250".into()],
                        vec!["".into(), "1500".into()],
                    ],
                    merges: vec![
                        Merge {
                            row: 0,
                            col: 0,
                            rowspan: 1,
                            colspan: 2,
                        },
                        Merge {
                            row: 1,
                            col: 0,
                            rowspan: 2,
                            colspan: 1,
                        },
                    ],
                    header_rows: 1,
                    ..Default::default()
                },
                Block {
                    kind: "figure".into(),
                    bbox: Some(BoxRect::full()),
                    description: "Visible arrow A → B".into(),
                    interpretation: "A likely precedes B".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    let target = t.path().join("notes.docx");
    word::export(&p, &target, &Control::default()).unwrap();
    let mut z = zip::ZipArchive::new(fs::File::open(&target).unwrap()).unwrap();
    let mut text = String::new();
    z.by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let d = roxmltree::Document::parse(&text).unwrap();
    assert!(d.descendants().any(|n| n.tag_name().name() == "gridSpan"));
    assert!(d.descendants().any(|n| n.tag_name().name() == "vMerge"));
    assert!(text.contains("1250") && text.contains("&amp;"));
    assert!(z.by_name("word/media/image1.jpg").is_ok());
    assert!(word::export(&p, &target, &Control::default()).is_err());
}
#[test]
fn local_sse_and_no_cross_request_state() {
    let s = Settings::default();
    for text in ["Hello Ω", "Second reading"] {
        let (url, t) = serve(sse(text, "stop"), "200 OK", 0);
        let value = runtime::chat(
            &url,
            serde_json::json!({}),
            &s,
            &Control::default(),
            &Activity::default(),
            1,
        )
        .unwrap();
        assert_eq!(value, text);
        t.join().unwrap();
    }
}
#[test]
fn truncation_redirect_and_oom_are_errors() {
    for (body, status, code) in [
        (sse("partial", "length"), "200 OK", "NG201"),
        ("".into(), "307 Temporary Redirect", "NG203"),
        ("CUDA error: out of memory".into(), "500 Error", "NG202"),
    ] {
        let (url, t) = serve(body, status, 0);
        let e = runtime::chat(
            &url,
            serde_json::json!({}),
            &Settings::default(),
            &Control::default(),
            &Activity::default(),
            1,
        )
        .unwrap_err();
        assert!(e.to_string().contains(code), "{e}");
        t.join().unwrap();
    }
}
#[test]
fn cancellation_during_model_wait_is_prompt() {
    let (url, t) = serve(sse("slow", "stop"), "200 OK", 600);
    let c = Control::default();
    let flag = c.cancelled.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(80));
        flag.store(true, Ordering::Relaxed);
    });
    let now = Instant::now();
    let e = runtime::chat(
        &url,
        serde_json::json!({}),
        &Settings::default(),
        &c,
        &Activity::default(),
        1,
    )
    .unwrap_err();
    assert!(e.to_string().contains("NG900"));
    assert!(now.elapsed() < Duration::from_secs(2));
    t.join().unwrap();
}
#[test]
fn pause_cancel_preserves_work() {
    let c = Control::default();
    c.paused.store(true, Ordering::Relaxed);
    let cancel = c.cancelled.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(80));
        cancel.store(true, Ordering::Relaxed);
    });
    assert!(c.check().is_err());
}
#[test]
fn registry_checks_gguf_and_local_manifest() {
    let t = tempfile::tempdir().unwrap();
    let a = t.path().join("weights.gguf");
    let b = t.path().join("projector.gguf");
    fs::write(&a, b"GGUFtest").unwrap();
    fs::write(&b, b"GGUFtest").unwrap();
    runtime::register(&t.path().join("models"), "glm-ocr", &a, &b).unwrap();
    assert_eq!(
        runtime::inventory(&t.path().join("models")).unwrap().len(),
        1
    );
    fs::write(&a, b"html").unwrap();
    assert!(runtime::inventory(&t.path().join("models")).is_err());
}
#[test]
fn pdf_bounds_and_real_tesseract_pipeline() {
    let library = pdfium();
    assert!(
        library.exists(),
        "Install PDFium and set NOTEGEN_PDFIUM for the native gate"
    );
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("scan.pdf");
    pdf(&source, &fixture());
    if let Some(out)=std::env::var_os("NOTEGEN_FIXTURE_DIR") {
        let folder=PathBuf::from(out);fs::create_dir_all(&folder).unwrap();
        fs::copy(&source,folder.join("scan.pdf")).unwrap();fixture().save(folder.join("scan.png")).unwrap();
    }
    let mut im = None;
    scan::render_pdf(
        &source,
        &library,
        "",
        150,
        1_200_000,
        &Control::default(),
        |_, p| {
            assert!(p.width() * p.height() <= 1_200_000);
            im = Some(p);
            Ok(())
        },
    )
    .unwrap();
    let s = Settings {
        preset: Preset::Fast,
        pdfium: library,
        auto_rotate: false,
        dpi: 150,
        ..Default::default()
    };
    let content = scan::printed(im.as_ref().unwrap(), &s, &Control::default()).unwrap();
    assert!(content.plain().contains("1250"), "{}", content.plain());
    assert!(content.blocks.iter().any(|b| b.kind == "table"));
    let first = notegen::pipeline::convert(
        &source,
        &t.path().join("notes.docx"),
        &s,
        &Control::default(),
        &Activity::default(),
    )
    .unwrap();
    assert_eq!(first.pages, 1);
    let second = notegen::pipeline::convert(
        &source,
        &t.path().join("notes2.docx"),
        &s,
        &Control::default(),
        &Activity::default(),
    )
    .unwrap();
    assert_eq!(first.project.root, second.project.root);
    let mut edited = second.project.load(1).unwrap();
    edited.edited = true;
    edited.reviewed = true;
    edited.content = PageContent {
        blocks: vec![Block::text("Human correction 1500")],
        ..Default::default()
    };
    second.project.save(&edited).unwrap();
    let mut themed = s.clone();
    themed.ui.dark = false;
    let third = notegen::pipeline::convert(
        &source,
        &t.path().join("notes3.docx"),
        &themed,
        &Control::default(),
        &Activity::default(),
    )
    .unwrap();
    assert_eq!(third.project.root, second.project.root);
    assert_eq!(
        third.project.load(1).unwrap().content.plain(),
        "Human correction 1500"
    );
    fs::write(third.project.page_path(1), b"broken checkpoint").unwrap();
    let repaired = notegen::pipeline::convert(
        &source,
        &t.path().join("notes4.docx"),
        &s,
        &Control::default(),
        &Activity::default(),
    )
    .unwrap();
    assert!(
        repaired
            .project
            .load(1)
            .unwrap()
            .content
            .plain()
            .contains("1250")
    );
}
#[test]
#[ignore = "requires downloaded local model and native runtime; invoke explicitly"]
fn live_model() {
    let root = PathBuf::from(std::env::var("NOTEGEN_LIVE_ROOT").expect("Set NOTEGEN_LIVE_ROOT"));
    fs::create_dir_all(&root).unwrap();
    let weights = PathBuf::from(std::env::var("NOTEGEN_LIVE_WEIGHTS").unwrap());
    let projector = PathBuf::from(std::env::var("NOTEGEN_LIVE_PROJECTOR").unwrap());
    let mut model = planning::catalog()
        .into_iter()
        .find(|m| m.id == std::env::var("NOTEGEN_LIVE_ID").unwrap_or_else(|_|"glm-ocr".into()))
        .unwrap();
    model.weights = weights;
    model.projector = projector;
    let mut s = Settings {
        backend: Backend::Cpu,
        server: std::env::var("NOTEGEN_LIVE_SERVER").unwrap().into(),
        request_timeout_secs: 600,
        max_tokens: 2048,
        context: 8192,
        auto_rotate: false,
        ..Default::default()
    };
    s.cpu_threads = 8;
    let p = planning::build(
        &notegen::hardware::detect(),
        &s,
        std::slice::from_ref(&model),
    )
    .unwrap();
    let server = runtime::Server::start(
        &model,
        &p,
        &s,
        &Control::default(),
        &Activity::default(),
        &root,
    )
    .unwrap();
    let result = server
        .read(
            &DynamicImage::ImageRgb8(fixture()),
            "",
            "Accounting worksheet",
            &s,
            &Control::default(),
            &Activity::default(),
            1,
        )
        .unwrap();
    fs::write(
        root.join("recognized.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    assert!(result.plain().contains("1250"), "{}", result.plain());
    let table_image = DynamicImage::ImageRgb8(fixture()).crop_imm(45, 295, 815, 215);
    let table = server
        .read_table(
            &table_image,
            "",
            "Accounting table",
            &s,
            &Control::default(),
            &Activity::default(),
            1,
        )
        .unwrap();
    fs::write(
        root.join("table.json"),
        serde_json::to_vec_pretty(&table).unwrap(),
    )
    .unwrap();
    assert!(
        table
            .blocks
            .iter()
            .any(|b| b.kind == "table" && b.rows.iter().flatten().any(|c| c.contains("1250"))),
        "{}",
        table.plain()
    );
}

#[test]
fn gpu_offload_requires_real_allocated_layers() {
    assert_eq!(runtime::offloaded_layers("CUDA0 device ready"), 0);
    assert_eq!(
        runtime::offloaded_layers("load_tensors: offloaded 0/33 layers to GPU"),
        0
    );
    assert_eq!(
        runtime::offloaded_layers("load_tensors: offloaded 33/33 layers to GPU"),
        33
    );
}
#[test]
fn appearance_changes_do_not_invalidate_recognition() {
    let a = Settings::default();
    let mut b = a.clone();
    b.ui.dark = false;
    b.ui.accent = [1, 2, 3];
    b.font = "Arial".into();
    assert_eq!(
        fingerprint(&notegen::pipeline::recognition_settings(&a)).unwrap(),
        fingerprint(&notegen::pipeline::recognition_settings(&b)).unwrap()
    );
    b.handwriting = true;
    assert_ne!(
        fingerprint(&notegen::pipeline::recognition_settings(&a)).unwrap(),
        fingerprint(&notegen::pipeline::recognition_settings(&b)).unwrap()
    );
}
#[test]
fn unrecognized_graphics_are_preserved() {
    let mut g = image::GrayImage::from_pixel(600, 800, image::Luma([255]));
    for x in 80..480 {
        for y in 400..405 {
            g.put_pixel(x, y, image::Luma([0]));
        }
    }
    for y in 200..405 {
        for x in 80..85 {
            g.put_pixel(x, y, image::Luma([0]));
        }
    }
    for y in 250..400 {
        for x in 200..260 {
            g.put_pixel(x, y, image::Luma([20]));
        }
    }
    let blocks = scan::graphics(&g, &[], &[]);
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].bbox.as_ref().unwrap().valid());
    assert!(blocks[0].uncertain);
}

#[test]
fn live_lighton_mixed_html_markdown_table_is_editable() {
    let text="<table>\n### Table - Sales Summary\n*Type: data*\n\n| Item | Amount |\n|------|-------|\n| Sales | 1250 |\n\n</table>";
    let p=parsing::lighton(text);let t=p.blocks.iter().find(|b|b.kind=="table").unwrap();
    assert_eq!(t.rows[1][1],"1250");assert!(p.validate().is_ok());
}
#[test]
fn balanced_skips_only_agreed_plain_pages_and_preserves_human_work() {
    let mut p=Page{agreement:0.99,content:PageContent{blocks:vec![Block::text("Clean print")],..Default::default()},..Default::default()};
    let mut s=Settings{preset:Preset::Balanced,..Default::default()};
    assert!(!notegen::pipeline::should_read("vision","vision",&p,&s));
    assert!(notegen::pipeline::should_read("ocr","ocr",&p,&s));
    s.handwriting=true;assert!(notegen::pipeline::should_read("vision","vision",&p,&s));
    s.handwriting=false;p.content.blocks.push(Block{kind:"figure".into(),bbox:Some(BoxRect::full()),..Default::default()});
    assert!(notegen::pipeline::should_read("vision","vision",&p,&s));
    p.edited=true;assert!(!notegen::pipeline::should_read("ocr","ocr",&p,&s));
}
