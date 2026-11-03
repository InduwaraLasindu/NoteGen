//! Bounded release regressions for concrete accessibility gaps found in the
//! existing panels. Native screen-reader speech remains a separate acceptance check.
use super::*;
use egui::accesskit::{
    Action, ActionData, ActionRequest, Live, Node, NodeId, Role, TreeId, TreeUpdate,
};

fn action(target: NodeId, action: Action, data: Option<ActionData>) -> egui::Event {
    egui::Event::AccessKitActionRequest(ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: target,
        data,
    })
}
fn draw(
    app: &mut Desktop,
    ctx: &egui::Context,
    size: Vec2,
    events: Vec<egui::Event>,
    mut render: impl FnMut(&mut Desktop, &mut egui::Ui),
) -> TreeUpdate {
    app.theme(ctx);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| render(app, ui),
    );
    output.textures_delta.clear();
    output
        .platform_output
        .accesskit_update
        .take()
        .expect("AccessKit tree enabled")
}
fn named<'a>(tree: &'a TreeUpdate, label: &str, role: Role) -> (NodeId, &'a Node) {
    tree.nodes
        .iter()
        .find(|(_, node)| node.role() == role && node.label() == Some(label))
        .map(|(id, node)| (*id, node))
        .unwrap_or_else(|| panic!("Missing accessible node {label:?} ({role:?})"))
}
fn app(root: &Path) -> Desktop {
    let mut state = Workspace::for_data_dir(root);
    state.settings.ui.reduced_motion = true;
    state.settings.experience.check_updates = false;
    state.settings.assistance.ask_questions = false;
    Desktop::new(state)
}

#[test]
fn studio_inputs_keep_distinct_names_after_accessible_focus_and_text_entry() {
    for (tab, name, role) in [
        (
            0,
            "Document subject and recognition context",
            Role::MultilineTextInput,
        ),
        (
            0,
            "Names, terminology and glossary",
            Role::MultilineTextInput,
        ),
        (1, "New writer profile name", Role::TextInput),
        (3, "Public topic search query", Role::TextInput),
    ] {
        let root = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = app(root.path());
        app.studio_open(tab);
        let size = egui::vec2(1200., 1050.);
        for _ in 0..3 {
            draw(&mut app, &ctx, size, vec![], |app, ui| {
                app.studio_ui(ui.ctx())
            });
        }
        let before = draw(&mut app, &ctx, size, vec![], |app, ui| {
            app.studio_ui(ui.ctx())
        });
        let (id, node) = named(&before, name, role);
        assert!(!node.is_disabled());
        assert!(node.supports_action(Action::Focus));
        assert!(node.supports_action(Action::SetValue));
        draw(
            &mut app,
            &ctx,
            size,
            vec![action(id, Action::Focus, None)],
            |app, ui| app.studio_ui(ui.ctx()),
        );
        let focused = draw(&mut app, &ctx, size, vec![], |app, ui| {
            app.studio_ui(ui.ctx())
        });
        assert_eq!(
            focused.focus, id,
            "{name} could not receive accessible focus"
        );
        draw(
            &mut app,
            &ctx,
            size,
            vec![action(
                id,
                Action::SetValue,
                Some(ActionData::Value(
                    "accessibility audit entered value".into(),
                )),
            )],
            |app, ui| app.studio_ui(ui.ctx()),
        );
        let after = draw(&mut app, &ctx, size, vec![], |app, ui| {
            app.studio_ui(ui.ctx())
        });
        assert_eq!(
            named(&after, name, role).0,
            id,
            "input identity/name changed with its value"
        );
        assert!(
            after
                .nodes
                .iter()
                .any(|(_, node)| node.role() == Role::TextRun
                    && node
                        .value()
                        .is_some_and(|text| text.contains("accessibility audit entered value"))),
            "accessible text entry was not applied to {name}"
        );
    }
}

#[test]
fn workspace_results_are_stable_polite_atomic_statuses_without_taking_keyboard_focus() {
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let mut app = app(root.path());
    app.notice = "Export completed in the selected output folder".into();
    let size = egui::vec2(1200., 1050.);
    for _ in 0..3 {
        draw(&mut app, &ctx, size, vec![], |app, ui| app.render(ui));
    }
    let before = draw(&mut app, &ctx, size, vec![], |app, ui| app.render(ui));
    let (status, _) = named(&before, &app.notice, Role::Status);
    let (focus, _) = named(&before, "Settings", Role::Button);
    draw(
        &mut app,
        &ctx,
        size,
        vec![action(focus, Action::Focus, None)],
        |app, ui| app.render(ui),
    );
    app.notice = "The output folder is unavailable; previous output is preserved".into();
    app.error_code = "NG400".into();
    let after = draw(&mut app, &ctx, size, vec![], |app, ui| app.render(ui));
    let (changed, node) = named(&after, &app.notice, Role::Status);
    assert_eq!(changed, status);
    assert_eq!(node.live(), Some(Live::Polite));
    assert!(node.is_live_atomic());
    assert_eq!(
        after.focus, focus,
        "result announcements stole keyboard focus"
    );
}

fn review_fixture(root: &Path) -> Project {
    let project = Project::open(
        root.join("project"),
        Manifest {
            schema: 2,
            input_sha256: "a".repeat(64),
            fingerprint: "b".repeat(64),
            pages: vec![1],
            models: vec![],
            settings: Settings::default(),
        },
    )
    .unwrap();
    image::RgbImage::from_pixel(640, 900, image::Rgb([255, 255, 255]))
        .save(project.image(1))
        .unwrap();
    project
        .save(&Page {
            page: 1,
            image: "page-00001.png".into(),
            content: PageContent {
                blocks: vec![Block::text("Original editable source")],
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
    project
}
#[test]
fn review_save_scrolls_into_minimum_viewport_and_remains_clickable_at_normal_and_large_text() {
    for scale in [1., 1.4] {
        let root = tempfile::tempdir().unwrap();
        let project = review_fixture(root.path());
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = app(root.path());
        app.state.settings.ui.scale = scale;
        assert!(app.open_review(project.root.clone(), &ctx));
        let review = app.review.as_mut().unwrap();
        review.page.content.blocks[0].text = "Saved from the minimum supported viewport".into();
        review.dirty = true;
        let size = egui::vec2(740., 650.) / scale;
        for _ in 0..3 {
            draw(&mut app, &ctx, size, vec![], |app, ui| {
                app.review_ui(ui.ctx())
            });
        }
        let before = draw(&mut app, &ctx, size, vec![], |app, ui| {
            app.review_ui(ui.ctx())
        });
        let (save, _) = before
            .nodes
            .iter()
            .find(|(_, node)| {
                node.role() == Role::Button
                    && node
                        .label()
                        .is_some_and(|label| label.starts_with("Save draft"))
            })
            .map(|(id, node)| (*id, node))
            .expect("review save action");
        draw(
            &mut app,
            &ctx,
            size,
            vec![action(save, Action::ScrollIntoView, None)],
            |app, ui| app.review_ui(ui.ctx()),
        );
        for _ in 0..3 {
            draw(&mut app, &ctx, size, vec![], |app, ui| {
                app.review_ui(ui.ctx())
            });
        }
        let after = draw(&mut app, &ctx, size, vec![], |app, ui| {
            app.review_ui(ui.ctx())
        });
        let node = &after
            .nodes
            .iter()
            .find(|(id, _)| *id == save)
            .expect("same review save action")
            .1;
        assert!(!node.is_hidden() && !node.is_disabled());
        let bounds = node.bounds().unwrap();
        let rect = egui::Rect::from_min_max(
            egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
            egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
        );
        assert!(rect.width() > 40. && rect.height() > 20. && rect.is_finite());
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(rect),
            "Save not reachable at minimum viewport, scale={scale}: {rect:?}, viewport={size:?}"
        );
        let window = ctx
            .memory(|memory| memory.area_rect(egui::Id::unique("notegen-review-window")))
            .unwrap();
        assert!(
            window.contains_rect(rect),
            "Save extends outside the review window"
        );
        for pressed in [true, false] {
            draw(
                &mut app,
                &ctx,
                size,
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                |app, ui| app.review_ui(ui.ctx()),
            );
        }
        assert!(
            !app.review.as_ref().unwrap().dirty,
            "Save not clickable: {}",
            app.review_error
        );
        assert_eq!(
            project.load(1).unwrap().content.plain(),
            "Saved from the minimum supported viewport"
        );
    }
}
