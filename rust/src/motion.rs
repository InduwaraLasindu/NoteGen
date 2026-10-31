//! Small, deterministic animation math shared by the native UI and its tests.
use crate::types::Appearance;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scene {
    pub scan: f32,
    pub orbit: f32,
    pub pulse: f32,
    pub reveal: f32,
    pub check: f32,
}

pub fn enabled(style: &Appearance) -> bool {
    !style.reduced_motion && style.motion.is_finite() && style.motion > 0.
}

pub fn transition(style: &Appearance) -> f32 {
    if enabled(style) {
        style.transition_ms.min(1000) as f32 / 1000. / style.motion.clamp(0.1, 3.)
    } else {
        0.
    }
}

fn ease_out(value: f64) -> f32 {
    (1. - (1. - value.clamp(0., 1.)).powi(3)) as f32
}

pub fn scene(
    elapsed: f64,
    style: &Appearance,
    busy: bool,
    paused: bool,
    completed: Option<f64>,
) -> Scene {
    if !enabled(style) || paused {
        return Scene {
            scan: 0.5,
            orbit: 0.,
            pulse: 0.5,
            reveal: 1.,
            check: if completed.is_some() { 1. } else { 0. },
        };
    }
    let seconds = if elapsed.is_finite() {
        elapsed.max(0.)
    } else {
        0.
    };
    let speed = style.motion.clamp(0.1, 3.) as f64;
    let time = seconds.rem_euclid(86400.) * speed;
    let period = if busy { 2.4 } else { 4.8 };
    let angle = time.rem_euclid(period) / period * std::f64::consts::TAU;
    Scene {
        scan: ((1. - angle.cos()) * 0.5) as f32,
        orbit: time.rem_euclid(1000.) as f32 * 0.12,
        pulse: ((1. + (time * 2.2).sin()) * 0.5) as f32,
        reveal: if style.intro_animation {
            ease_out(seconds / 0.55 * speed)
        } else {
            1.
        },
        check: completed.map_or(0., |age| {
            ease_out(if age.is_finite() {
                age.max(0.) * style.motion as f64 / 0.45
            } else {
                1.
            })
        }),
    }
}

/// Ambient work sleeps when hidden/unfocused. A running job still refreshes time once a second.
pub fn repaint_delay(
    style: &Appearance,
    focused: bool,
    busy: bool,
    paused: bool,
) -> Option<Duration> {
    let decorative = style.particles || style.scan_animation;
    if enabled(style) && focused && decorative && !paused {
        let fps = if busy {
            style.fps
        } else {
            style.idle_fps.min(style.fps)
        }
        .clamp(1, 144);
        Some(Duration::from_secs_f64(1. / fps as f64))
    } else if busy {
        Some(Duration::from_secs(1))
    } else {
        None
    }
}
