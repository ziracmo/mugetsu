//! Hover card of a PR, shown almost at once: a short shimmering skeleton,
//! then the card is revealed by a fiery liquid front, a rolling wave, sweeping
//! left to right and shedding sparks.

use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, AnyView, App, IntoElement, ParentElement as _,
    Styled as _, Window, div, linear_color_stop, linear_gradient, px, relative,
};
use mugetsu_forge::PullRequest;

use super::toolbar::pr_card;
use crate::theme;

/// Hover time before the card (in its loading state) appears.
pub const DELAY: Duration = Duration::from_millis(70);

/// Skeleton alone, then the sweep, then the last sparks fade.
const SKELETON_MS: f32 = 240.;
const SWEEP_MS: f32 = 200.;
const SPARK_MS: f32 = 170.;
/// Whole entrance.
const TOTAL_MS: f32 = SKELETON_MS + SWEEP_MS + SPARK_MS;
/// The sweep, as a fraction of the entrance.
const SWEEP: (f32, f32) = (SKELETON_MS / TOTAL_MS, (SKELETON_MS + SWEEP_MS) / TOTAL_MS);
const SPARKS: usize = 22;
/// Horizontal bands the front is cut into, each offset to draw the wave.
const BANDS: usize = 24;
/// Wave depth, as a fraction of the card's width.
const WAVE: f32 = 0.07;
/// Width of the fiery gradient trailing the front.
const FIRE_W: f32 = 56.;

/// The tooltip view for `pr`.
pub fn view(pr: &PullRequest, window: &mut Window, cx: &mut App) -> AnyView {
    let pr = pr.clone();
    Tooltip::element(move |_, cx| card(&pr, cx)).build(window, cx)
}

/// 0..1 progress of the front across the card at animation time `t`.
fn front(t: f32) -> f32 {
    let k = ((t - SWEEP.0) / (SWEEP.1 - SWEEP.0)).clamp(0., 1.);
    // Fast start, liquid settle.
    1. - (1. - k).powi(2)
}

/// Where the front sits at height `row` (0..1, top to bottom) for progress
/// `k`: a wave of about one and a half crests that rolls downward as it
/// crosses. Starts left of the card and ends right of it, so the card is
/// fully covered at 0 and fully revealed at 1.
fn wave(k: f32, row: f32) -> f32 {
    let phase = std::f32::consts::TAU * (1.5 * row - 1.2 * k);
    -WAVE + (1. + 2. * WAVE) * k + WAVE * phase.sin()
}

/// The accent pushed toward full saturation, for the fire.
fn vivid(c: gpui_kit::Hsla) -> gpui_kit::Hsla {
    gpui_kit::Hsla {
        s: (c.s * 1.5 + 0.2).min(1.),
        l: c.l.clamp(0.5, 0.62),
        ..c
    }
}

fn card(pr: &PullRequest, cx: &mut App) -> AnyElement {
    let cover_bg = cx.theme().tokens.popover;
    let content = pr_card(pr, cx);
    let bar = |w: f32, h: f32| {
        div()
            .w(relative(w))
            .h(px(h))
            .rounded_md()
            .bg(theme::alpha(gpui_kit::white(), 0.08))
    };
    let skeleton = move |i: usize| {
        v_flex()
            .py_1()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(bar(0.22, 16.))
                    .child(bar(0.1, 16.))
                    .child(div().flex_1())
                    .child(
                        div()
                            .size(px(16.))
                            .rounded_full()
                            .bg(theme::alpha(gpui_kit::white(), 0.08)),
                    ),
            )
            .child(bar(0.85, 14.))
            .child(bar(0.55, 12.))
            .child(bar(0.7, 12.))
            .with_animation(
                ("pr-tip-skeleton", i),
                Animation::new(Duration::from_millis(700)).repeat(),
                |el, t| el.opacity(0.55 + 0.45 * (t * std::f32::consts::TAU).cos().abs()),
            )
    };
    let fire_from = theme::alpha(theme::amber(), 0.);
    let fire_to = theme::alpha(vivid(theme::accent()), 0.95);
    // One horizontal band of the cover and of the fire; offsetting each band
    // by `wave` shapes the front.
    let band = move |i: usize| {
        let row = (i as f32 + 0.5) / BANDS as f32;
        let top = relative(i as f32 / BANDS as f32);
        let h = relative(1. / BANDS as f32);
        // The skeleton is laid out against the whole card, then clipped by
        // the band, so the front uncovers a still skeleton.
        let laid_out = div()
            .absolute()
            .top(relative(-(i as f32)))
            .h(relative(BANDS as f32))
            .child(skeleton(i))
            .with_animation(
                ("pr-tip-skeleton-place", i),
                Animation::new(Duration::from_millis(TOTAL_MS as u64)),
                move |el, t| {
                    // Band-relative position and width of the whole card.
                    let at = wave(front(t), row).min(0.99);
                    el.left(relative(-at / (1. - at)))
                        .w(relative(1. / (1. - at)))
                },
            );
        let cover = div()
            .absolute()
            .top(top)
            .h(h)
            .right_0()
            .overflow_hidden()
            .bg(cover_bg)
            .child(laid_out)
            .with_animation(
                ("pr-tip-cover", i),
                Animation::new(Duration::from_millis(TOTAL_MS as u64)),
                move |el, t| {
                    let k = front(t);
                    el.left(relative(wave(k, row).min(0.99)))
                        .opacity(if k >= 1. { 0. } else { 1. })
                },
            );
        let fire = div()
            .absolute()
            .top(top)
            .h(h)
            .w(px(FIRE_W))
            .bg(linear_gradient(
                90.,
                linear_color_stop(fire_from, 0.),
                linear_color_stop(fire_to, 1.),
            ))
            .with_animation(
                ("pr-tip-fire", i),
                Animation::new(Duration::from_millis(TOTAL_MS as u64)),
                move |el, t| {
                    let k = front(t);
                    let on = t >= SWEEP.0 && k < 1.;
                    el.left(relative(wave(k, row)))
                        .ml(px(-FIRE_W))
                        .opacity(if on { 1. } else { 0. })
                },
            );
        (cover, fire)
    };
    let (covers, fires): (Vec<_>, Vec<_>) = (0..BANDS).map(band).unzip();
    let sparks = (0..SPARKS).map(|i| {
        // Born along the sweep, each drifts right and up and burns out.
        let born = SWEEP.0 + (SWEEP.1 - SWEEP.0) * i as f32 / SPARKS as f32;
        let row = ((i * 7) % SPARKS) as f32 / SPARKS as f32;
        let size = 2. + (i % 4) as f32 * 0.8;
        let drift = 18. + (i % 4) as f32 * 9.;
        let color = if i % 3 == 0 {
            theme::amber()
        } else {
            vivid(theme::accent())
        };
        div()
            .absolute()
            .size(px(size))
            .rounded_full()
            .bg(color)
            .with_animation(
                ("pr-tip-spark", i),
                Animation::new(Duration::from_millis(TOTAL_MS as u64)),
                move |el, t| {
                    let age = ((t - born) / (SPARK_MS / TOTAL_MS)).clamp(0., 1.);
                    let alive = t >= born && age < 1.;
                    el.left(relative(wave(front(born + 0.02), row).clamp(0., 0.98)))
                        .ml(px(drift * age))
                        .top(relative(row))
                        .mt(px(-10. * age))
                        .opacity(if alive { (1. - age) * 0.95 } else { 0. })
                },
            )
    });
    div()
        .relative()
        .overflow_hidden()
        .child(content)
        .children(covers)
        .children(fires)
        .children(sparks)
        .into_any_element()
}
