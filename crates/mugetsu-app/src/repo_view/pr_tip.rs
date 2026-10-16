//! Hover card of a PR, shown almost at once: a short shimmering skeleton,
//! then the card fades in while a liquid gradient flows around its border
//! and settles back to the plain edge.

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

/// Skeleton alone, before the card shows.
const SKELETON_MS: f32 = 240.;
/// Skeleton giving way to the card.
const REVEAL_MS: f32 = 140.;
/// Liquid border, from the end of loading until it settles.
const BORDER_MS: f32 = 1100.;
/// Whole entrance.
const TOTAL_MS: f32 = SKELETON_MS + BORDER_MS;
/// End of loading, as a fraction of the entrance.
const LOADED: f32 = SKELETON_MS / TOTAL_MS;
/// Border thickness.
const EDGE: f32 = 1.5;

/// The tooltip view for `pr`: its own frame replaces the tooltip's border
/// and padding so the border can be animated.
pub fn view(pr: &PullRequest, window: &mut Window, cx: &mut App) -> AnyView {
    let pr = pr.clone();
    Tooltip::element(move |_, cx| card(&pr, cx))
        .p_0()
        .border_0()
        .build(window, cx)
}

/// The accent pushed toward full saturation.
fn vivid(c: gpui_kit::Hsla) -> gpui_kit::Hsla {
    gpui_kit::Hsla {
        s: (c.s * 1.5 + 0.2).min(1.),
        l: c.l.clamp(0.5, 0.62),
        ..c
    }
}

/// 0..1 progress of the border animation at entrance time `t`.
fn border(t: f32) -> f32 {
    ((t - LOADED) / (1. - LOADED)).clamp(0., 1.)
}

/// How strongly the liquid shows over the plain edge at progress `k`: quick
/// rise, then a long ebb.
fn glow(k: f32) -> f32 {
    if k <= 0. || k >= 1. {
        0.
    } else if k < 0.15 {
        k / 0.15
    } else {
        let e = (k - 0.15) / 0.85;
        1. - e * e * (3. - 2. * e)
    }
}

fn card(pr: &PullRequest, cx: &mut App) -> AnyElement {
    let popover = cx.theme().tokens.popover;
    let edge = cx.theme().border;
    let radius = cx.theme().radius;
    let content = pr_card(pr, cx);
    let bar = |w: f32, h: f32| {
        div()
            .w(relative(w))
            .h(px(h))
            .rounded_md()
            .bg(theme::alpha(gpui_kit::white(), 0.08))
    };
    let skeleton = v_flex()
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
            "pr-tip-skeleton",
            Animation::new(Duration::from_millis(700)).repeat(),
            |el, t| el.opacity(0.55 + 0.45 * (t * std::f32::consts::TAU).cos().abs()),
        );
    let entrance = || Animation::new(Duration::from_millis(TOTAL_MS as u64));
    let reveal = REVEAL_MS / TOTAL_MS;
    let cover = div()
        .absolute()
        .inset_0()
        .bg(popover)
        .child(skeleton)
        .with_animation("pr-tip-cover", entrance(), move |el, t| {
            let k = ((t - LOADED) / reveal).clamp(0., 1.);
            el.opacity(1. - k * k)
        });
    let accent = vivid(theme::accent());
    // Two gradients turning against each other at different speeds: where
    // they cross, the colours mix and seem to flow around the edge.
    let liquid =
        |id: &'static str, turns: f32, from: gpui_kit::Hsla, to: gpui_kit::Hsla, a: f32| {
            div().absolute().inset_0().rounded(radius).with_animation(
                id,
                entrance(),
                move |el, t| {
                    let k = border(t);
                    // Fast, then slowing down as it settles.
                    let spin = 1. - (1. - k).powi(3);
                    el.bg(linear_gradient(
                        40. + 360. * turns * spin,
                        linear_color_stop(from, 0.),
                        linear_color_stop(to, 1.),
                    ))
                    .opacity(a * glow(k))
                },
            )
        };
    let clear = theme::alpha(accent, 0.);
    div()
        .relative()
        .rounded(radius)
        .p(px(EDGE))
        .bg(edge)
        .child(liquid(
            "pr-tip-liquid-a",
            1.25,
            accent,
            theme::magenta(),
            1.,
        ))
        .child(liquid(
            "pr-tip-liquid-b",
            -0.8,
            clear,
            theme::alpha(theme::amber(), 0.9),
            0.85,
        ))
        .child(
            div()
                .relative()
                .rounded(radius - px(EDGE))
                .bg(popover)
                .py_0p5()
                .px_2()
                .child(div().relative().child(content).child(cover)),
        )
        .into_any_element()
}
