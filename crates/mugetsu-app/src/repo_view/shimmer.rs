//! Light sweep across a history row: new commits after a reload, or the tip
//! of a branch being shown.

use std::time::Duration;

use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, IntoElement, ParentElement as _, Styled as _, div,
    linear_color_stop, linear_gradient, relative,
};

use crate::theme;

/// An overlay for a `relative()` + `overflow_hidden()` parent; `seq` restarts it.
pub fn sweep(seq: usize) -> AnyElement {
    let glow = theme::alpha(gpui_kit::white(), 0.13);
    let clear = theme::alpha(gpui_kit::white(), 0.);
    let band = |from, to| {
        div().h_full().w(relative(0.5)).bg(linear_gradient(
            90.,
            linear_color_stop(from, 0.),
            linear_color_stop(to, 1.),
        ))
    };
    div()
        .absolute()
        .top_0()
        .h_full()
        .w(relative(0.35))
        .flex()
        .child(band(clear, glow))
        .child(band(glow, clear))
        .with_animation(
            ("shimmer", seq),
            Animation::new(Duration::from_millis(1100)).with_easing(gpui_kit::ease_in_out),
            |el, t| el.left(relative(-0.35 + 1.35 * t)),
        )
        .into_any_element()
}

/// Pushed branch chip: a turning gradient edge (lime → accent) with a light
/// sweep, wrapped around `chip`.
pub fn pushed(chip: AnyElement, seq: usize) -> AnyElement {
    div()
        .relative()
        .flex_none()
        .rounded_md()
        .p(gpui_kit::px(1.5))
        .overflow_hidden()
        .with_animation(
            ("pushed-edge", seq),
            Animation::new(Duration::from_millis(1400)).repeat(),
            |el, t| {
                el.bg(linear_gradient(
                    360. * t,
                    linear_color_stop(theme::lime(), 0.),
                    linear_color_stop(theme::accent(), 1.),
                ))
            },
        )
        .child(chip)
        .child(sweep(seq))
        .into_any_element()
}

/// Checkout: a bright sweep crossing the whole row, carrying particles that
/// fly off to the right and slow down (inertia).
pub fn checkout(seq: usize) -> AnyElement {
    const COUNT: usize = 22;
    let particles = (0..COUNT).map(move |i| {
        // Deterministic spread: start along the sweep, various speeds and heights.
        let delay = (i % 7) as f32 * 0.06;
        let from = 0.05 + (i % 5) as f32 * 0.08;
        let reach = 0.35 + (i % 4) as f32 * 0.16;
        let top = 3. + ((i * 7) % 18) as f32;
        let size = 2. + (i % 3) as f32;
        let color = theme::lane_color(i as u16);
        div()
            .absolute()
            .size(gpui_kit::px(size))
            .rounded_full()
            .bg(color)
            .top(gpui_kit::px(top))
            .with_animation(
                ("checkout-particle", seq * COUNT + i),
                Animation::new(Duration::from_millis(1500)),
                move |el, t| {
                    let local = ((t - delay) / (1. - delay)).clamp(0., 1.);
                    // Ease-out quint: fast start, long glide.
                    let eased = 1. - (1. - local).powi(5);
                    el.left(relative(from + reach * eased))
                        .opacity(if local == 0. {
                            0.
                        } else {
                            (1. - local).powf(1.5)
                        })
                },
            )
    });
    let glow = theme::alpha(theme::accent(), 0.28);
    let clear = theme::alpha(theme::accent(), 0.);
    div()
        .absolute()
        .inset_0()
        .child(
            div()
                .absolute()
                .top_0()
                .h_full()
                .w(relative(0.3))
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(clear, 0.),
                    linear_color_stop(glow, 1.),
                ))
                .with_animation(
                    ("checkout-sweep", seq),
                    Animation::new(Duration::from_millis(900))
                        .with_easing(gpui_kit::ease_out_quint()),
                    |el, t| el.left(relative(-0.3 + 1.3 * t)).opacity(1. - t * t),
                ),
        )
        .children(particles)
        .into_any_element()
}
