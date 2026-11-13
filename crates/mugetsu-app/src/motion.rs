//! Animations for states that can last (uncommitted changes, remote updates,
//! a blocked stack): a few cycles, then still. A `.repeat()` animation redraws
//! its whole view every frame for as long as it is shown, which kept a CPU
//! core busy while the app sat idle on a dirty working tree.

use std::time::Duration;

use gpui_kit::Animation;

/// Cycles played before the element settles.
pub const CYCLES: f32 = 3.;

/// `cycle` played [`CYCLES`] times through `easing` (one cycle over 0..1), ending on `easing(1)`.
pub fn settling(cycle: Duration, easing: impl Fn(f32) -> f32 + 'static) -> Animation {
    Animation::new(cycle.mul_f32(CYCLES)).with_easing(move |t| {
        if t >= 1. {
            easing(1.)
        } else {
            easing((t * CYCLES).fract())
        }
    })
}

/// Opacity breathing between `min` and `max` a few times, settling on `max`.
pub fn settling_pulse(cycle: Duration, min: f32, max: f32) -> Animation {
    let breathe = gpui_kit::pulsating_between(min, max);
    // Shifted by a quarter cycle so it starts and ends on its peak.
    settling(cycle, move |t| breathe((t + 0.25).fract()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn settling_pulse_starts_and_ends_at_full() {
        let breathe = gpui_kit::pulsating_between(0.35, 1.);
        let at = |t: f32| breathe((t + 0.25).fract());
        assert!((at(0.) - 1.).abs() < 1e-3);
        assert!(at(0.5) < 0.4);
    }
}
