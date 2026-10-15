//! Little particle burst played around a branch label after a checkout.

use std::f32::consts::TAU;
use std::time::Duration;

use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, IntoElement, ParentElement as _, Styled as _, div,
    px, relative,
};

use crate::theme;

const COUNT: usize = 18;

/// An overlay centred on its (relative) parent; `seq` restarts the animation.
pub fn burst(seq: usize) -> AnyElement {
    let particles = (0..COUNT).map(move |i| {
        // Deterministic spread: alternate distances and sizes, palette colours.
        let angle = TAU * i as f32 / COUNT as f32 + (i % 3) as f32 * 0.13;
        let distance = 26. + (i % 4) as f32 * 9.;
        let size = 3. + (i % 3) as f32;
        let color = theme::lane_color(i as u16);
        let (dx, dy) = (angle.cos() * distance * 1.6, angle.sin() * distance);
        div()
            .absolute()
            .size(px(size))
            .rounded_full()
            .bg(color)
            .with_animation(
                ("burst", seq * COUNT + i),
                Animation::new(Duration::from_millis(900 + (i % 5) as u64 * 60))
                    .with_easing(gpui_kit::ease_out_quint()),
                move |el, t| {
                    el.left(px(dx * t - size / 2.))
                        .top(px(dy * t - size / 2.))
                        .opacity(1. - t * t)
                },
            )
    });
    div()
        .absolute()
        .top(relative(0.5))
        .left(relative(0.5))
        .size_0()
        .children(particles)
        .into_any_element()
}
