use notegen::{inputs, storage::Control};
use std::{fs, sync::atomic::Ordering};

#[test]
fn real_native_ocr_image_folder_conversion_and_resume() {
    use ab_glyph::FontArc;
    use notegen::{pipeline, storage::Activity, types::*};
    let font_path = [
        "C:/Windows/Fonts/arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]
    .iter()
    .find(|p| std::path::Path::new(p).is_file())
    .expect("native tests require a system font");
    let font = FontArc::try_from_vec(fs::read(font_path).unwrap()).unwrap();
    let d = tempfile::tempdir().unwrap();
    let folder = d.path().join("scans");
    fs::create_dir(&folder).unwrap();
    let mut im = image::RgbImage::from_pixel(1000, 650, image::Rgb([255, 255, 255]));
    imageproc::drawing::draw_text_mut(
        &mut im,
        image::Rgb([0, 0, 0]),
        40,
        60,
        52.,
        &font,
        "Invoice Total 1250",
    );
    im.save(folder.join("001.png")).unwrap();
    im.save(folder.join("002.jpg")).unwrap();
    let mut s = Settings {
        preset: Preset::Fast,
        auto_rotate: false,
        deskew: false,
        ..Default::default()
    };
    s.assistance.ask_questions = false;
    for name in ["first.docx", "resumed.docx"] {
        let done = pipeline::convert(
            &folder,
            &d.path().join(name),
            &s,
            &Control::default(),
            &Activity::default(),
        )
        .unwrap();
        assert_eq!(done.pages, 2);
        assert!(done.output.is_file());
        for n in [1, 2] {
            assert!(
                done.project
                    .load(n)
                    .unwrap()
                    .content
                    .plain()
                    .contains("1250")
            );
        }
        assert_eq!(
            notegen::projects::metadata(&done.project).unwrap().source,
            Some(folder.canonicalize().unwrap())
        );
    }
}

#[test]
fn png_and_jpeg_share_page_range_and_pixel_limits() {
    let d = tempfile::tempdir().unwrap();
    for ext in ["png", "jpg"] {
        let p = d.path().join(format!("scan.{ext}"));
        image::RgbImage::from_pixel(30, 40, image::Rgb([255, 255, 255]))
            .save(&p)
            .unwrap();
        assert_eq!(inputs::selected_page_count(&p, d.path(), "").unwrap(), 1);
        let mut n = 0;
        inputs::render(
            &p,
            d.path(),
            "1",
            150,
            2000,
            &Control::default(),
            |page, im| {
                assert_eq!(page, 1);
                assert_eq!(im.width(), 30);
                n += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(n, 1);
        assert!(
            inputs::render(
                &p,
                d.path(),
                "2",
                150,
                2000,
                &Control::default(),
                |_, _| Ok(())
            )
            .is_err()
        );
        assert!(
            inputs::render(&p, d.path(), "", 150, 100, &Control::default(), |_, _| Ok(
                ()
            ))
            .is_err()
        );
    }
}
#[test]
fn multipage_tiff_honors_range_order_and_does_not_decode_unselected_pages() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("scan.tiff");
    {
        let mut enc = tiff::encoder::TiffEncoder::new(fs::File::create(&p).unwrap()).unwrap();
        enc.write_image::<tiff::encoder::colortype::RGB8>(2, 2, &[10u8; 12])
            .unwrap();
        enc.write_image::<tiff::encoder::colortype::RGB8>(2, 2, &[20u8; 12])
            .unwrap();
    }
    assert_eq!(inputs::selected_page_count(&p, d.path(), "").unwrap(), 2);
    let mut pages = vec![];
    inputs::render(&p, d.path(), "2", 150, 100, &Control::default(), |n, im| {
        pages.push(n);
        assert_eq!(im.to_rgb8().get_pixel(0, 0)[0], 20);
        Ok(())
    })
    .unwrap();
    assert_eq!(pages, vec![2]);
}
#[test]
fn folder_import_is_sorted_content_bound_and_ignores_unrelated_files() {
    let d = tempfile::tempdir().unwrap();
    for name in ["02.png", "01.png"] {
        image::GrayImage::from_pixel(5, 5, image::Luma([90]))
            .save(d.path().join(name))
            .unwrap();
    }
    fs::write(d.path().join("ignore.txt"), "hello").unwrap();
    let files = inputs::files(d.path()).unwrap();
    assert_eq!(files[0].file_name().unwrap(), "01.png");
    let old = inputs::hash(d.path(), &Control::default()).unwrap();
    assert_eq!(
        inputs::selected_page_count(d.path(), d.path(), "").unwrap(),
        2
    );
    image::GrayImage::from_pixel(5, 5, image::Luma([40]))
        .save(d.path().join("02.png"))
        .unwrap();
    assert_ne!(old, inputs::hash(d.path(), &Control::default()).unwrap());
}
#[test]
fn cancelled_and_invalid_inputs_stop_without_callback() {
    let d = tempfile::tempdir().unwrap();
    assert!(inputs::files(d.path()).is_err());
    let p = d.path().join("bad.tiff");
    fs::write(&p, b"broken").unwrap();
    assert!(inputs::selected_page_count(&p, d.path(), "").is_err());
    let p = d.path().join("scan.png");
    image::GrayImage::new(5, 5).save(&p).unwrap();
    let c = Control::default();
    c.cancelled.store(true, Ordering::Relaxed);
    assert!(
        inputs::render(&p, d.path(), "", 150, 100, &c, |_, _| panic!(
            "cancel must prevent decoding"
        ))
        .is_err()
    );
}
