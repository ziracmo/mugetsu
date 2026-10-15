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
