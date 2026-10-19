//! Preview of the window a tab dragged out of its window will open: a
//! transparent overlay over the display drawing that window, faded, under
//! the pointer (the drag itself is only drawn inside its own window).

use gpui_kit::component::h_flex;
use gpui_kit::{
    AppContext as _, Bounds, Context, DisplayId, IntoElement, ParentElement as _, Pixels, Render,
    SharedString, Styled as _, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, div, prelude::FluentBuilder as _, px,
};

use crate::theme;
use crate::workspace::TITLE_HEIGHT;

pub struct TabGhost {
    /// The future window, relative to the display; none while the pointer
    /// is back over the window the tab comes from.
    window: Option<Bounds<Pixels>>,
    /// The tab in that window, relative to it.
    tab: Bounds<Pixels>,
    name: SharedString,
    branch: SharedString,
}

/// Opens the (empty) overlay over `display`.
pub fn open(
    display: (DisplayId, Bounds<Pixels>),
    tab: Bounds<Pixels>,
    name: SharedString,
    branch: SharedString,
    cx: &mut gpui_kit::App,
) -> Option<WindowHandle<TabGhost>> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(display.1)),
        titlebar: None,
        focus: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: Some(display.0),
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    cx.open_window(options, |_, cx| {
        cx.new(|_| TabGhost {
            window: None,
            tab,
            name,
            branch,
        })
    })
    .ok()
}

impl TabGhost {
    pub fn show(&mut self, window: Option<Bounds<Pixels>>, cx: &mut Context<Self>) {
        if self.window != window {
            self.window = window;
            cx.notify();
        }
    }
}

impl Render for TabGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let tab = self.tab;
        div().size_full().when_some(self.window, |s, b| {
            s.child(
                div()
                    .absolute()
                    .left(b.origin.x)
                    .top(b.origin.y)
                    .w(b.size.width)
                    .h(b.size.height)
                    .rounded(px(12.))
                    .border_1()
                    .border_color(theme::alpha(theme::accent(), 0.55))
                    .bg(theme::alpha(theme::sheet(), 0.45))
                    .shadow_lg()
                    .child(
                        div()
                            .relative()
                            .h(px(TITLE_HEIGHT))
                            .rounded_t(px(12.))
                            .border_b_1()
                            .border_color(theme::hairline())
                            .bg(theme::alpha(theme::sheet(), 0.5))
                            .child(
                                h_flex()
                                    .absolute()
                                    .left(tab.origin.x)
                                    .top(tab.origin.y)
                                    .w(tab.size.width)
                                    .h(tab.size.height)
                                    .px_2()
                                    .gap_1p5()
                                    .items_baseline()
                                    .justify_center()
                                    .rounded_lg()
                                    .bg(theme::alpha(gpui_kit::white(), 0.11))
                                    .border_1()
                                    .border_color(theme::glass_edge())
                                    .shadow_md()
                                    .text_sm()
                                    .line_height(tab.size.height)
                                    .text_color(gpui_kit::white())
                                    .child(
                                        div()
                                            .flex_none()
                                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                            .child(self.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_xs()
                                            .text_color(theme::muted())
                                            .child(self.branch.clone()),
                                    ),
                            ),
                    ),
            )
        })
    }
}
