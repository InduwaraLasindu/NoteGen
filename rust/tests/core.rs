use notegen::{consensus::*, hardware::*, planning, planning::*, types::*};

#[test]
fn scan_enhancement_preserves_flat_and_nearly_white_pages_without_overflow() {
    for value in [0, 1, 245, 250, 255] {
        let image = image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(
            32,
            32,
            image::Luma([value]),
        ));
        let enhanced = notegen::scan::enhance(&image);
        assert!(enhanced.pixels().all(|pixel| pixel[0] == value));
    }
    let mut image = image::GrayImage::from_pixel(32, 32, image::Luma([250]));
    for y in 0..16 {
        for x in 0..32 {
            image.put_pixel(x, y, image::Luma([255]));
        }
    }
    let enhanced = notegen::scan::enhance(&image::DynamicImage::ImageLuma8(image.clone()));
    assert_eq!(enhanced, image);
}
fn hw(vram: u64) -> Hardware {
    Hardware {
        cpu: "Test CPU".into(),
        logical_cores: 24,
        physical_cores: 16,
        ram_mib: 65536,
        free_ram_mib: 55000,
        gpus: if vram == 0 {
            vec![]
        } else {
            vec![Gpu {
                index: 1,
                name: "RTX".into(),
                uuid: "uuid".into(),
                total_mib: vram,
                free_mib: vram - 256,
                backend: "cuda".into(),
            }]
        },
        simd: vec![],
    }
}
#[test]
fn hardware_tiers_choose_larger_vision() {
    let mut previous = 0;
    for vram in [4096, 8192, 16384, 32768] {
        let p = build(&hw(vram), &Settings::default(), &catalog()).unwrap();
        let m = p.models.iter().find(|m| m.role == "vision").unwrap();
        assert!(m.resident_mib <= p.vram_budget_mib);
        assert!(m.rank >= previous);
        previous = m.rank;
        assert!(p.vram_budget_mib < vram);
        assert_eq!(p.threads, 23);
    }
    assert_eq!(previous, 99);
}
#[test]
fn maximum_uses_independent_families_sequentially() {
    let s = Settings {
        preset: Preset::Maximum,
        ..Default::default()
    };
    for vram in [4096, 16384, 32768] {
        let p = build(&hw(vram), &s, &catalog()).unwrap();
        assert_eq!(p.models.len(), 3);
        let families = p
            .models
            .iter()
            .map(|m| m.family.clone())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(families.len(), 3);
    }
}
#[test]
fn memory_pressure_and_cpu_fallback() {
    let p = build(&hw(0), &Settings::default(), &catalog()).unwrap();
    assert_eq!(p.backend, Backend::Cpu);
    assert_eq!(p.gpu_layers, 0);
    let mut h = hw(4096);
    h.free_ram_mib = 500;
    assert!(build(&h, &Settings::default(), &catalog()).is_err());
    let s = Settings {
        require_acceleration: true,
        ..Default::default()
    };
    assert!(build(&hw(0), &s, &catalog()).is_err());
}
#[test]
fn busy_gpu_falls_back_to_fitting_models() {
    let mut h = hw(32768);
    h.gpus[0].free_mib = 3200;
    let p = build(&h, &Settings::default(), &catalog()).unwrap();
    assert!(p.models.iter().all(|m| m.resident_mib <= p.vram_budget_mib));
}
#[test]
fn overrides_are_respected_and_validated() {
    let s = Settings {
        dpi: 600,
        context: 32768,
        cpu_threads: 4,
        image_tokens: 2048,
        ..Default::default()
    };
    let p = build(&hw(16384), &s, &catalog()).unwrap();
    assert_eq!(
        (p.dpi, p.context, p.threads, p.image_tokens),
        (600, 32768, 4, 2048)
    );
    for fraction in [f32::NAN, -1., 1.1] {
        assert!(
            Settings {
                vram_fraction: fraction,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}
#[test]
fn handwriting_cannot_silently_use_the_printed_only_preset() {
    for preset in [
        Preset::Fast,
        Preset::Balanced,
        Preset::Accuracy,
        Preset::Maximum,
    ] {
        let s = Settings {
            preset,
            handwriting: true,
            ..Default::default()
        };
        if preset == Preset::Fast {
            let e = s.validate().unwrap_err();
            assert!(e.to_string().contains("NG102"));
            assert!(e.to_string().contains("Handwriting"));
        } else {
            s.validate().unwrap();
        }
    }
}
#[test]
fn malicious_endpoints_blocked() {
    for u in [
        "https://example.com:8",
        "http://localhost:8",
        "http://0.0.0.0:8",
        "http://127.0.0.1:8/path",
        "http://u:p@127.0.0.1:8",
        "http://127.0.0.1:8?x",
    ] {
        assert!(local_endpoint(u).is_err(), "{u}");
    }
    assert!(local_endpoint("http://[::1]:8080").is_ok());
}
#[test]
fn page_ranges_and_boxes() {
    assert_eq!(pages_range("3,1-2,2", 4).unwrap(), vec![1, 2, 3]);
    for s in ["0", "2-1", "9", "1,,2"] {
        assert!(pages_range(s, 4).is_err());
    }
    assert!(
        !BoxRect {
            left: 0.5,
            top: 0.,
            right: 0.4,
            bottom: 1.
        }
        .valid()
    );
    let b = BoxRect {
        left: 0.50001,
        top: 0.50001,
        right: 0.50002,
        bottom: 0.50002,
    };
    let (a, b, c, d) = b.pixels(10, 10);
    assert!(a < c && b < d);
}
#[test]
fn table_merges_checked() {
    let mut b = Block {
        kind: "table".into(),
        rows: vec![vec!["a".into(), "b".into()], vec!["c".into(), "d".into()]],
        ..Default::default()
    };
    b.merges = vec![
        Merge {
            row: 0,
            col: 0,
            rowspan: 2,
            colspan: 1,
        },
        Merge {
            row: 0,
            col: 0,
            rowspan: 1,
            colspan: 2,
        },
    ];
    assert!(b.validate().is_err());
    b.merges.pop();
    assert!(b.validate().is_ok());
}
fn candidate(model: &str, family: &str, text: &str) -> Candidate {
    Candidate {
        region: None,
        model_id: model.into(),
        family: family.into(),
        view: "full".into(),
        content: PageContent {
            blocks: vec![Block::text(text)],
            ..Default::default()
        },
        reliability: 0.8,
    }
}

#[test]
fn checkpoint_validation_bounds_all_reader_evidence_before_review() {
    let mut page = Page {
        page: 1,
        image: "page-00001.png".into(),
        candidates: vec![candidate("a", "one", "visible")],
        ..Default::default()
    };
    page.validate().unwrap();
    page.candidates[0].region = Some(BoxRect {
        left: 0.8,
        top: 0.,
        right: 0.2,
        bottom: 1.,
    });
    assert!(page.validate().is_err());
    page.candidates[0].region = None;
    page.candidates[0].reliability = f64::INFINITY;
    assert!(page.validate().is_err());
    page.candidates[0].reliability = 0.8;
    page.candidates[0].content.blocks[0].kind = "unknown".into();
    assert!(page.validate().is_err());
    page.candidates[0].content.blocks[0].kind = "text".into();
    page.candidates = vec![page.candidates[0].clone(); 1025];
    assert!(page.validate().is_err());
    page.candidates.clear();
    page.seconds = -1.;
    assert!(page.validate().is_err());
    page.seconds = 0.;
    page.validate().unwrap();
}
#[test]
fn consensus_selects_real_reading() {
    let cs = vec![
        candidate("a", "one", "Revenue 1250"),
        candidate("b", "two", "Revenue 1250"),
        candidate("c", "three", "Revenue 1200"),
    ];
    let (p, agreement) = select(&cs, 2, 0.92);
    assert_eq!(p.plain(), "Revenue 1250");
    assert!(agreement < 1.);
    assert!(p.warnings.iter().any(|w| w.contains("Numeric")));
}
#[test]
fn repeated_model_is_not_independent_consensus() {
    let cs = vec![
        candidate("a", "one", "Hello"),
        candidate("a-crop", "one", "Hello"),
    ];
    let (p, a) = select(&cs, 2, 0.92);
    assert_eq!(a, 0.);
    assert!(!p.warnings.is_empty());
}
#[test]
fn disagreement_and_illegibility_force_review() {
    let cs = vec![
        candidate("a", "one", "[illegible]"),
        candidate("b", "two", "different"),
    ];
    let (p, _) = select(&cs, 2, 0.92);
    assert!(p.blocks[0].uncertain);
    assert!(
        Page {
            content: p,
            ..Default::default()
        }
        .needs_review(0.9)
    );
}
#[test]
fn nvidia_parser_handles_multiple_adapters() {
    let g = parse_nvidia("0, GT 710, GPU-a, 2048, 1000\n1, RTX 5070 Ti, GPU-b, 16376, 15000\n");
    assert_eq!(g.len(), 2);
    assert_eq!(g[1].index, 1);
    assert_eq!(g[1].free_mib, 15000);
}
#[test]
fn server_is_offline_cuda_and_not_shell_code() {
    let s = Settings::default();
    let p = build(&hw(16384), &s, &catalog()).unwrap();
    let mut m = p.models[0].clone();
    m.weights = "C:/a b/weights.gguf".into();
    let a = server_args(&m, &p, &s, 18745);
    assert!(a.contains(&"--offline".to_string()));
    assert!(a.contains(&"C:/a b/weights.gguf".to_string()));
    assert!(a.contains(&"--flash-attn".to_string()));
    assert!(a.windows(2).any(|a| a == ["--device", "CUDA1"]));
}

#[test]
fn manual_model_selection_still_respects_memory() {
    let h = hw(4096);
    let s = Settings {
        automatic_models: false,
        model_ids: vec!["qwen38-27b".into()],
        ..Default::default()
    };
    assert!(
        planning::build(&h, &s, &planning::catalog())
            .unwrap()
            .models
            .is_empty()
    );
    let s = Settings {
        automatic_models: false,
        model_ids: vec!["glm-ocr".into()],
        ..Default::default()
    };
    let p = planning::build(&h, &s, &planning::catalog()).unwrap();
    assert_eq!(p.models.len(), 1);
    assert_eq!(p.models[0].id, "glm-ocr");
}
#[test]
fn forced_cpu_never_allocates_gpu_projector() {
    let h = hw(16384);
    let s = Settings {
        backend: Backend::Cpu,
        ..Default::default()
    };
    let p = planning::build(&h, &s, &planning::catalog()).unwrap();
    assert_eq!(p.gpu_index, None);
    assert_eq!(p.gpu_layers, 0);
    let args = planning::server_args(&p.models[0], &p, &s, 18745);
    assert!(!args.contains(&"--device".into()));
    assert!(args.contains(&"--no-mmproj-offload".into()));
}
