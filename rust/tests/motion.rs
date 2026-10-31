use notegen::{
    motion,
    types::{Appearance, Settings},
};
use std::time::Duration;
#[test]
fn reduced_motion_and_zero_speed_freeze_every_decorative_property() {
    for reduced in [true, false] {
        let style = Appearance {
            reduced_motion: reduced,
            motion: if reduced { 2. } else { 0. },
            ..Default::default()
        };
        assert_eq!(
            motion::scene(0., &style, false, false, None),
            motion::scene(1234., &style, true, false, None)
        );
        assert_eq!(motion::transition(&style), 0.);
        assert_eq!(motion::repaint_delay(&style, true, false, false), None);
    }
}
#[test]
fn motion_changes_with_time_and_completes_without_looping_checkmarks() {
    let style = Appearance::default();
    assert_ne!(
        motion::scene(1., &style, false, false, None),
        motion::scene(2., &style, false, false, None)
    );
    assert_eq!(motion::scene(2., &style, false, false, Some(1.)).check, 1.);
    assert_eq!(
        motion::scene(12., &style, false, false, Some(10.)).check,
        1.
    );
    assert_eq!(motion::scene(0., &style, false, false, None).reveal, 0.);
    assert_eq!(motion::scene(2., &style, false, false, None).reveal, 1.);
}
#[test]
fn animation_math_stays_finite_after_clock_extremes() {
    for time in [f64::NAN, f64::INFINITY, -10., 0., 1., 1e20, f64::MAX] {
        let scene = motion::scene(time, &Appearance::default(), true, false, Some(time));
        for value in [scene.scan, scene.pulse, scene.reveal, scene.check] {
            assert!(value.is_finite() && (0.0..=1.0).contains(&value));
        }
        assert!(scene.orbit.is_finite());
    }
}
#[test]
fn background_and_paused_windows_do_not_run_at_animation_frame_rate() {
    let style = Appearance {
        fps: 120,
        idle_fps: 24,
        ..Default::default()
    };
    assert_eq!(
        motion::repaint_delay(&style, true, false, false),
        Some(Duration::from_secs_f64(1. / 24.))
    );
    assert_eq!(
        motion::repaint_delay(&style, true, true, false),
        Some(Duration::from_secs_f64(1. / 120.))
    );
    assert_eq!(motion::repaint_delay(&style, false, false, false), None);
    assert_eq!(
        motion::repaint_delay(&style, false, true, false),
        Some(Duration::from_secs(1))
    );
    assert_eq!(
        motion::repaint_delay(&style, true, true, true),
        Some(Duration::from_secs(1))
    );
    assert_eq!(
        motion::scene(10., &style, true, true, None),
        motion::scene(20., &style, true, true, None)
    );
}
#[test]
fn motion_settings_roundtrip_and_invalid_budgets_are_rejected() {
    let mut s = Settings::default();
    s.ui.transition_ms = 350;
    s.ui.ambient_density = 5;
    s.ui.scan_animation = false;
    let raw = serde_json::to_string(&s).unwrap();
    let loaded: Settings = serde_json::from_str(&raw).unwrap();
    assert_eq!(loaded.ui.transition_ms, 350);
    assert!(!loaded.ui.scan_animation);
    loaded.validate().unwrap();
    s.ui.idle_fps = 0;
    assert!(s.validate().is_err());
    s.ui.idle_fps = 30;
    s.ui.ambient_density = 9;
    assert!(s.validate().is_err());
}
