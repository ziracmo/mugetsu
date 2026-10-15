//! Mugetsu > À propos de Mugetsu: icon, name, version and links, like the
//! standard macOS About window.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, TitleBar, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyWindowHandle, App, AppContext as _, BoxShadow, Context,
    Global, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    WindowBounds, WindowOptions, div, img, prelude::FluentBuilder as _, px, size,
};
use std::time::Duration;

use crate::i18n::tr;
use crate::theme;

pub const REPO_URL: &str = "https://github.com/ziracmo/mugetsu";

struct OpenAboutWindow(AnyWindowHandle);
impl Global for OpenAboutWindow {}

/// "0.1.0 (abc1234)", the commit only when the build knew it.
pub fn version() -> String {
    let commit = env!("MUGETSU_COMMIT");
    let v = env!("CARGO_PKG_VERSION");
    if commit.is_empty() {
        v.to_string()
    } else {
        format!("{v} ({commit})")
    }
}

/// Opens the window, or brings the existing one to the front.
pub fn open(cx: &mut App) {
    if let Some(handle) = cx.try_global::<OpenAboutWindow>().map(|g| g.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(340.), px(400.)), cx)),
        is_resizable: false,
        is_minimizable: false,
        window_background: gpui_kit::WindowBackgroundAppearance::Blurred,
        ..TitleBar::window_options()
    };
    if let Ok(handle) = gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title(tr("À propos de Mugetsu", "About Mugetsu"));
        cx.new(|_| AboutView)
    }) {
        cx.set_global(OpenAboutWindow(handle.0));
    }
}

pub struct AboutView;

impl Render for AboutView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let date = env!("MUGETSU_DATE");
        let link = |id: &'static str, icon: Lucide, label: &'static str, url: String| {
            Button::new(id)
                .outline()
                .small()
                .icon(Icon::new(icon))
                .label(label)
                .on_click(move |_, _, cx| cx.open_url(&url))
        };
        v_flex()
            .size_full()
            .text_color(cx.theme().foreground)
            .child(TitleBar::new())
            .child(
                v_flex()
                    .id("about")
                    .test_support()
                    .flex_1()
                    .items_center()
                    .px_6()
                    .pb_6()
                    .gap_1()
                    .child(
                        // A soft accent glow breathing behind the icon.
                        div()
                            .relative()
                            .size(px(112.))
                            .mb_3()
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .rounded_full()
                                    .m(px(18.))
                                    .with_animation(
                                        "about-glow",
                                        Animation::new(Duration::from_millis(2600))
                                            .repeat()
                                            .with_easing(gpui_kit::bounce(gpui_kit::ease_in_out)),
                                        |el, t| {
                                            el.shadow(vec![BoxShadow {
                                                color: theme::alpha(
                                                    theme::accent(),
                                                    0.25 + 0.35 * t,
                                                ),
                                                offset: gpui_kit::point(px(0.), px(0.)),
                                                blur_radius: px(36.),
                                                spread_radius: px(6. + 6. * t),
                                                inset: false,
                                            }])
                                        },
                                    ),
                            )
                            .child(img(crate::repo_view::forms::APP_ICON.clone()).size_full()),
                    )
                    .child(
                        div()
                            .text_xl()
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .child("Mugetsu"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::muted())
                            .child(format!("Version {}", version())),
                    )
                    .when(!date.is_empty(), |s| {
                        s.child(
                            div()
                                .text_xs()
                                .text_color(theme::alpha(theme::muted(), 0.8))
                                .child(date),
                        )
                    })
                    .child(div().text_xs().text_color(theme::muted()).mt_2().child(tr(
                        "Un client Git rapide, écrit en Rust.",
                        "A fast Git client, written in Rust.",
                    )))
                    .child(div().flex_1())
                    .child(
                        h_flex()
                            .gap_2()
                            .child(link(
                                "about-github",
                                Lucide::Github,
                                "GitHub",
                                REPO_URL.to_string(),
                            ))
                            .child(link(
                                "about-releases",
                                Lucide::Tag,
                                tr("Notes de version", "Release notes"),
                                format!("{REPO_URL}/releases"),
                            )),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_starts_with_the_crate_version() {
        assert!(super::version().starts_with(env!("CARGO_PKG_VERSION")));
    }
}
