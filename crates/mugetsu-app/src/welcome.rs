//! First impressions: the eclipse played on the very first launch, and the
//! welcome screen shown while no repository is open.

use std::f32::consts::TAU;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, BoxShadow, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, div,
    img, linear_color_stop, linear_gradient, point, prelude::FluentBuilder as _, px, relative, rgb,
};

use crate::i18n::tr;
use crate::theme;

/// Whole opening eclipse, from first light to the app showing through.
pub const ECLIPSE: Duration = Duration::from_millis(3400);
/// Totality, as a fraction of [`ECLIPSE`].
const TOTAL: f32 = 0.56;
const SUN: f32 = 150.;
const STARS: usize = 70;
const SPARKS: usize = 48;
const BEADS: usize = 7;

fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}

fn ease_out(t: f32) -> f32 {
    1. - (1. - t).powi(3)
}

/// Progress of `t` through the `[a, b]` window, clamped to `0..=1`.
fn span(t: f32, a: f32, b: f32) -> f32 {
    ((t - a) / (b - a)).clamp(0., 1.)
}

fn glow(color: Hsla, blur: f32, spread: f32) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.), px(0.)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    }
}

/// Full-window overlay: dusk turns to night, the moon slides over a warm
/// sun, Baily's beads and a diamond ring flash, the corona flares with
/// shockwaves and sparks, the name writes itself, then a bloom of light
/// opens onto the app. `seq` keys the animations so it can be replayed.
pub fn eclipse(seq: u64) -> AnyElement {
    let sky: Hsla = rgb(0x050507).into();
    let warm: Hsla = rgb(0xffd9a0).into();
    let white = gpui_kit::white();
    let c = SUN / 2.;
    let anim = || Animation::new(ECLIPSE);

    // Dusk fades out as the moon takes the light.
    let dusk = div()
        .absolute()
        .inset_0()
        .bg(linear_gradient(
            180.,
            linear_color_stop(rgb(0x0b1030), 0.),
            linear_color_stop(rgb(0x3b2342), 1.),
        ))
        .with_animation("eclipse-dusk", anim(), |el, t| {
            el.opacity(1. - span(t, 0., TOTAL))
        });
    let stars = (0..STARS).map(|i| {
        // Deterministic scatter; each twinkles at its own pace.
        let x = ((i * 37 + 11) % 101) as f32 / 101.;
        let y = ((i * 53 + 7) % 97) as f32 / 97.;
        let size = 1. + (i % 3) as f32 * 0.7;
        let phase = (i % 7) as f32 / 7.;
        let speed = 4. + (i % 5) as f32;
        div()
            .absolute()
            .left(relative(x))
            .top(relative(y))
            .size(px(size))
            .rounded_full()
            .bg(white)
            .with_animation(("eclipse-star", i), anim(), move |el, t| {
                // Stars come out as the light goes, one after the other.
                let dark = span(t, 0.15 + phase * 0.3, TOTAL);
                let twinkle = 0.45 + 0.55 * ((t * speed + phase) * TAU).sin().abs();
                el.opacity(dark * twinkle * 0.9)
            })
    });
    let sun = div()
        .absolute()
        .size(px(SUN))
        .rounded_full()
        .bg(warm)
        .with_animation("eclipse-sun", anim(), move |el, t| {
            let k = span(t, 0., TOTAL);
            let flare = span(t, TOTAL, TOTAL + 0.08) * (1. - span(t, TOTAL + 0.1, 0.9));
            // The corona breathes once the disc is covered.
            let breathe = 0.5 + 0.5 * (t * 14.).sin();
            let corona = theme::alpha(theme::accent(), 0.35 + 0.6 * k);
            el.shadow(vec![
                glow(
                    theme::alpha(warm, 0.55 * (1. - k) + 0.45),
                    30. + 30. * k,
                    4. + 14. * k,
                ),
                glow(
                    corona,
                    90. + 120. * k + 60. * flare,
                    10. + 40. * k + 20. * breathe * k,
                ),
                glow(
                    theme::alpha(white, 0.35 * flare),
                    160. + 80. * flare,
                    20. * flare,
                ),
            ])
        });
    let moon = div()
        .absolute()
        // A hair smaller than the sun: a burning ring stays at totality.
        .size(px(SUN - 5.))
        .rounded_full()
        .bg(sky)
        .with_animation("eclipse-moon", anim(), |el, t| {
            let k = ease_in_out(span(t, 0., TOTAL));
            el.left(px(2.5 - (1. - k) * SUN * 1.25))
                .top(px(2.5 - (1. - k) * SUN * 0.18))
        });
    // Baily's beads: the last light through lunar valleys, on the trailing rim.
    let beads = (0..BEADS).map(|i| {
        let a = (-55. + 110. * i as f32 / (BEADS - 1) as f32).to_radians();
        let size = 3. + (i % 3) as f32 * 1.5;
        let phase = i as f32 * 1.7;
        div()
            .absolute()
            .left(px(c + (c - 1.) * a.cos() - size / 2.))
            .top(px(c + (c - 1.) * a.sin() - size / 2.))
            .size(px(size))
            .rounded_full()
            .bg(white)
            .with_animation(("eclipse-bead", i), anim(), move |el, t| {
                let on = span(t, TOTAL - 0.09, TOTAL - 0.05) * (1. - span(t, TOTAL - 0.02, TOTAL));
                let flicker = 0.6 + 0.4 * (t * 90. + phase).sin();
                el.opacity(on * flicker).shadow(vec![glow(
                    theme::alpha(warm, 0.9),
                    8. + 6. * on,
                    1.,
                )])
            })
    });
    // Diamond ring: a white bead on the rim, just before totality.
    let diamond = div()
        .absolute()
        .size(px(10.))
        .rounded_full()
        .bg(white)
        .left(px(SUN * 0.86))
        .top(px(SUN * 0.18))
        .with_animation("eclipse-diamond", anim(), move |el, t| {
            let d = 1. - ((t - (TOTAL - 0.03)) / 0.06).abs().min(1.);
            el.opacity(d).shadow(vec![
                glow(white, 18. + 40. * d, 4. + 14. * d),
                glow(theme::alpha(theme::accent(), 0.6 * d), 80. * d, 10. * d),
            ])
        });
    // Shockwaves rolling out of totality.
    let rings = (0..3usize).map(|i| {
        let start = TOTAL + 0.02 + i as f32 * 0.07;
        div().absolute().rounded_full().border_2().with_animation(
            ("eclipse-ring", i),
            anim(),
            move |el, t| {
                let p = ease_out(span(t, start, start + 0.32));
                let size = SUN + p * SUN * (5. + i as f32);
                let fade = if p > 0. { 1. - p } else { 0. };
                el.size(px(size))
                    .left(px(c - size / 2.))
                    .top(px(c - size / 2.))
                    .border_color(theme::alpha(theme::accent(), 0.7 * fade))
                    .shadow(vec![glow(
                        theme::alpha(theme::accent(), 0.4 * fade),
                        24.,
                        0.,
                    )])
            },
        )
    });
    // Sparks thrown off the corona.
    let sparks = (0..SPARKS).map(|i| {
        let a = i as f32 / SPARKS as f32 * TAU + (i % 5) as f32 * 0.09;
        let reach = 90. + ((i * 29) % 11) as f32 * 22.;
        let size = 2. + (i % 3) as f32;
        let delay = (i % 4) as f32 * 0.015;
        div()
            .absolute()
            .size(px(size))
            .rounded_full()
            .bg(if i % 3 == 0 { warm } else { white })
            .with_animation(("eclipse-spark", i), anim(), move |el, t| {
                let p = ease_out(span(t, TOTAL + delay, TOTAL + delay + 0.3));
                let r = c + 4. + p * reach;
                let on = if p > 0. { 1. - p } else { 0. };
                el.left(px(c + r * a.cos() - size / 2.))
                    .top(px(c + r * a.sin() - size / 2.))
                    .opacity(on)
                    .shadow(vec![glow(theme::alpha(theme::accent(), on), 6., 1.)])
            })
    });
    // The name writes itself, letter by letter, under the ring.
    let letters = "Mugetsu".chars().enumerate().map(|(i, ch)| {
        let start = TOTAL + 0.06 + i as f32 * 0.022;
        div()
            .child(ch.to_string())
            .with_animation(("eclipse-letter", i), anim(), move |el, t| {
                let k = ease_out(span(t, start, start + 0.09));
                el.opacity(k).mt(px(12. * (1. - k)))
            })
    });
    let title = v_flex()
        .absolute()
        .top(px(SUN + 34.))
        .left(px(c - 180.))
        .w(px(360.))
        .items_center()
        .gap_1()
        .child(
            div()
                .text_xl()
                .text_color(theme::accent())
                .child("月")
                .with_animation("eclipse-kanji", anim(), |el, t| {
                    let k = span(t, TOTAL + 0.02, TOTAL + 0.1);
                    el.opacity(k)
                }),
        )
        .child(
            h_flex()
                .h(px(40.))
                .gap(px(3.))
                .text_3xl()
                .font_weight(gpui_kit::FontWeight::BOLD)
                .text_color(white)
                .children(letters),
        )
        .child(
            div()
                .text_sm()
                .text_color(theme::muted())
                .whitespace_nowrap()
                .child(tr("Git, sans attendre.", "Git, without the wait."))
                .with_animation("eclipse-tagline", anim(), |el, t| {
                    el.opacity(span(t, TOTAL + 0.2, TOTAL + 0.28))
                }),
        );
    // Bloom: a burst of light opening onto the app.
    let bloom =
        div()
            .absolute()
            .rounded_full()
            .with_animation("eclipse-bloom", anim(), move |el, t| {
                let p = ease_out(span(t, 0.84, 1.));
                let size = SUN + p * 2600.;
                let on = if p > 0. { 1. - p * 0.6 } else { 0. };
                el.size(px(size))
                    .left(px(c - size / 2.))
                    .top(px(c - size / 2.))
                    .bg(theme::alpha(theme::accent(), 0.25 * on))
                    .shadow(vec![glow(theme::alpha(white, 0.45 * on), 120., 30.)])
            });
    let scene = div()
        .id(("eclipse-run", seq as usize))
        .absolute()
        .inset_0()
        .bg(sky)
        .overflow_hidden()
        .child(dusk)
        .children(stars)
        .child(
            div()
                .absolute()
                .top(relative(0.5))
                .left(relative(0.5))
                .mt(px(-c - 40.))
                .ml(px(-c))
                .size(px(SUN))
                .children(rings)
                .child(sun)
                .child(moon)
                .children(beads)
                .child(diamond)
                .children(sparks)
                .child(title)
                .child(bloom),
        )
        .with_animation("eclipse-out", anim(), |el, t| {
            // Holds totality a beat, then lets the app through.
            el.opacity(1. - span(t, 0.86, 1.))
        });
    div()
        .id("eclipse")
        .test_support()
        .absolute()
        .inset_0()
        .occlude()
        .child(scene)
        .into_any_element()
}

type OnOpen = Rc<dyn Fn(&mut Window, &mut App)>;
type OnRecent = Rc<dyn Fn(PathBuf, &mut Window, &mut App)>;

/// Shown while no repository is open: the moon, a call to open one, the
/// recent repositories and a few shortcuts, sliding in one after the other.
pub fn empty_state(recent: &[PathBuf], on_open: OnOpen, on_recent: OnRecent) -> AnyElement {
    let enter = |el: gpui_kit::Stateful<gpui_kit::Div>, i: usize| {
        let delay = 90. * i as f32;
        el.with_animation(
            ("welcome-in", i),
            Animation::new(Duration::from_millis(500 + delay as u64)),
            move |el, t| {
                let k = ((t * (500. + delay) - delay) / 500.).clamp(0., 1.);
                let k = 1. - (1. - k).powi(3);
                el.opacity(k).mt(px(14. * (1. - k)))
            },
        )
    };
    let hero = v_flex()
        .id("welcome-hero")
        .items_center()
        .gap_2()
        .child(
            div()
                .relative()
                .size(px(96.))
                .mb_2()
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .m(px(14.))
                        .rounded_full()
                        .with_animation(
                            "welcome-glow",
                            Animation::new(Duration::from_millis(2800))
                                .repeat()
                                .with_easing(gpui_kit::bounce(gpui_kit::ease_in_out)),
                            |el, t| {
                                el.shadow(vec![glow(
                                    theme::alpha(theme::accent(), 0.25 + 0.3 * t),
                                    40.,
                                    6. + 8. * t,
                                )])
                            },
                        ),
                )
                .child(img(crate::repo_view::forms::APP_ICON.clone()).size_full()),
        )
        .child(
            div()
                .text_3xl()
                .font_weight(gpui_kit::FontWeight::BOLD)
                .text_color(gpui_kit::white())
                .child("Mugetsu"),
        )
        .child(div().text_sm().text_color(theme::muted()).child(tr(
            "Ouvrez un dépôt Git pour commencer.",
            "Open a Git repository to get started.",
        )));
    let open = h_flex()
        .id("open-first")
        .test_support()
        .gap_2()
        .px_4()
        .h(px(36.))
        .rounded_lg()
        .cursor_pointer()
        .bg(theme::accent())
        .text_color(gpui_kit::white())
        .text_sm()
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .shadow(vec![glow(theme::alpha(theme::accent(), 0.35), 18., 0.)])
        .hover(|s| s.bg(theme::alpha(theme::accent(), 0.85)))
        .child(Icon::new(Lucide::FolderOpen).small())
        .child(tr("Ouvrir un dépôt…", "Open repository…"))
        .child(
            div()
                .ml_1()
                .px_1p5()
                .rounded_md()
                .text_xs()
                .bg(theme::alpha(gpui_kit::white(), 0.2))
                .child("⌘O"),
        )
        .on_click(move |_, window, cx| on_open(window, cx));
    let recents = recent.iter().take(6).enumerate().map(|(i, path)| {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        let shown = crate::app_menu::display_path(path);
        let icon = crate::repo_icon::of(path);
        let (path, on_recent) = (path.clone(), on_recent.clone());
        h_flex()
            .id(("welcome-recent", i))
            .w_full()
            .h(px(40.))
            .px_3()
            .gap_3()
            .rounded_lg()
            .cursor_pointer()
            .hover(|s| s.bg(theme::raised()))
            .child(
                div()
                    .size(px(26.))
                    .flex_none()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::alpha(gpui_kit::white(), 0.05))
                    .child(match icon {
                        Some(icon) => crate::repo_icon::view(&icon, 15.),
                        None => Icon::new(Lucide::FolderGit2)
                            .small()
                            .text_color(theme::muted())
                            .into_any_element(),
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().truncate().child(name))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted())
                            .truncate()
                            .child(shown),
                    ),
            )
            .child(
                Icon::new(Lucide::ChevronRight)
                    .xsmall()
                    .text_color(theme::muted()),
            )
            .on_click(move |_, window, cx| on_recent(path.clone(), window, cx))
    });
    let has_recent = !recent.is_empty();
    let shortcut = |keys: &'static str, label: &'static str| {
        h_flex()
            .gap_1p5()
            .child(
                div()
                    .px_1p5()
                    .rounded_md()
                    .border_1()
                    .border_color(theme::hairline())
                    .bg(theme::alpha(gpui_kit::white(), 0.04))
                    .child(keys),
            )
            .child(label)
    };
    v_flex()
        .id("welcome")
        .test_support()
        .size_full()
        .items_center()
        .justify_center()
        .gap_6()
        .child(enter(hero, 0))
        .child(enter(h_flex().id("welcome-open").child(open), 1))
        .when(has_recent, |s| {
            s.child(enter(
                v_flex()
                    .id("welcome-recents")
                    .w(px(440.))
                    .p_2()
                    .rounded_xl()
                    .border_1()
                    .border_color(theme::hairline())
                    .bg(theme::alpha(gpui_kit::white(), 0.025))
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .text_xs()
                            .text_color(theme::muted())
                            .child(tr("Récents", "Recent")),
                    )
                    .children(recents),
                2,
            ))
        })
        .child(enter(
            h_flex()
                .id("welcome-keys")
                .gap_5()
                .text_xs()
                .text_color(theme::muted())
                .child(shortcut("⌘K", tr("Commandes", "Commands")))
                .child(shortcut("⌘O", tr("Ouvrir", "Open")))
                .child(shortcut("⇧⌘N", tr("Nouvelle fenêtre", "New window")))
                .child(shortcut("⌃Tab", tr("Changer d'onglet", "Switch tab")))
                .child(shortcut("⌘,", tr("Réglages", "Settings"))),
            3,
        ))
        .into_any_element()
}
