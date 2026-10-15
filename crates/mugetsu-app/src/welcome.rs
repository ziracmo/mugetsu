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
    img, point, prelude::FluentBuilder as _, px, relative, rgb,
};

use crate::i18n::tr;
use crate::theme;

/// Whole first-launch eclipse, from first light to the app showing through.
pub const ECLIPSE: Duration = Duration::from_millis(2600);
/// Totality, as a fraction of [`ECLIPSE`].
const TOTAL: f32 = 0.62;
const SUN: f32 = 150.;

fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
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

/// Full-window overlay: the moon slides over a warm sun, the corona flares
/// at totality, a diamond ring flashes, then everything fades into the app.
pub fn eclipse() -> AnyElement {
    let sky: Hsla = rgb(0x050507).into();
    let warm: Hsla = rgb(0xffd9a0).into();
    let stars = (0..40usize).map(|i| {
        // Deterministic scatter; each twinkles at its own pace.
        let x = ((i * 37 + 11) % 100) as f32 / 100.;
        let y = ((i * 53 + 7) % 100) as f32 / 100.;
        let size = 1. + (i % 3) as f32 * 0.6;
        let phase = (i % 7) as f32 / 7.;
        div()
            .absolute()
            .left(relative(x))
            .top(relative(y))
            .size(px(size))
            .rounded_full()
            .bg(gpui_kit::white())
            .with_animation(
                ("eclipse-star", i),
                Animation::new(ECLIPSE),
                move |el, t| {
                    // Stars come out as the light goes.
                    let dark = (t / TOTAL).clamp(0., 1.);
                    let twinkle = 0.5 + 0.5 * ((t * 6. + phase) * TAU).sin();
                    el.opacity(dark * twinkle * 0.8)
                },
            )
    });
    let sun = div()
        .absolute()
        .size(px(SUN))
        .rounded_full()
        .bg(warm)
        .with_animation("eclipse-sun", Animation::new(ECLIPSE), move |el, t| {
            let k = (t / TOTAL).clamp(0., 1.);
            // The disc dims, the corona grows into an accent halo.
            let corona = theme::alpha(theme::accent(), 0.35 + 0.6 * k);
            el.shadow(vec![
                glow(
                    theme::alpha(warm, 0.55 * (1. - k) + 0.45),
                    30. + 30. * k,
                    4. + 14. * k,
                ),
                glow(corona, 90. + 120. * k, 10. + 40. * k),
            ])
        });
    let moon = div()
        .absolute()
        // A hair smaller than the sun: a burning ring stays at totality.
        .size(px(SUN - 5.))
        .rounded_full()
        .bg(sky)
        .with_animation("eclipse-moon", Animation::new(ECLIPSE), |el, t| {
            let k = ease_in_out((t / TOTAL).clamp(0., 1.));
            el.left(px(2.5 - (1. - k) * SUN * 1.25))
                .top(px(2.5 - (1. - k) * SUN * 0.18))
        });
    // Diamond ring: a white bead on the rim, just before totality.
    let diamond = div()
        .absolute()
        .size(px(10.))
        .rounded_full()
        .bg(gpui_kit::white())
        .left(px(SUN * 0.86))
        .top(px(SUN * 0.18))
        .with_animation("eclipse-diamond", Animation::new(ECLIPSE), |el, t| {
            let d = 1. - ((t - (TOTAL - 0.04)) / 0.07).abs().min(1.);
            el.opacity(d)
                .shadow(vec![glow(gpui_kit::white(), 18. + 30. * d, 4. + 10. * d)])
        });
    let title = div()
        .absolute()
        .top(px(SUN + 48.))
        .w(px(SUN))
        .flex()
        .justify_center()
        .text_2xl()
        .font_weight(gpui_kit::FontWeight::BOLD)
        .text_color(gpui_kit::white())
        .whitespace_nowrap()
        .child("月 Mugetsu")
        .with_animation("eclipse-title", Animation::new(ECLIPSE), |el, t| {
            el.opacity(((t - TOTAL) / 0.12).clamp(0., 1.))
        });
    div()
        .id("eclipse")
        .test_support()
        .absolute()
        .inset_0()
        .occlude()
        .bg(sky)
        .overflow_hidden()
        .children(stars)
        .child(
            div()
                .absolute()
                .top(relative(0.5))
                .left(relative(0.5))
                .mt(px(-SUN / 2. - 30.))
                .ml(px(-SUN / 2.))
                .size(px(SUN))
                .child(sun)
                .child(moon)
                .child(diamond)
                .child(title),
        )
        .with_animation("eclipse-out", Animation::new(ECLIPSE), |el, t| {
            // Holds totality a beat, then lets the app through.
            el.opacity(1. - ((t - 0.8) / 0.2).clamp(0., 1.))
        })
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
