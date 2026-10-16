//! The repository's pulse, pinned under the sidebar: commits per day over the
//! last month, as a smooth line over a fading fill. No axis, no legend; each
//! day answers on hover.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, Bounds, Context,
    InteractiveElement as _, IntoElement, ParentElement as _, PathBuilder, Pixels, Point,
    SharedString, StatefulInteractiveElement as _, Styled as _, Task, Window, canvas, div,
    linear_color_stop, linear_gradient, point, px,
};

use super::RepoView;
use crate::i18n::tr;
use crate::theme;

/// Days shown, today included.
pub const DAYS: usize = 30;
const HEIGHT: f32 = 40.;
/// Room above the highest point and below the baseline for the stroke.
const PAD: f32 = 4.;
const STROKE: f32 = 1.5;
const DOT: f32 = 3.;

/// Commits per day, oldest first, and what they were counted for.
#[derive(Default)]
pub(super) struct Pulse {
    days: Arc<Vec<u32>>,
    /// Refs fingerprint and local day: recount when either moves.
    key: Option<(u64, i64)>,
    _scan: Option<Task<()>>,
}

impl RepoView {
    /// Recounts in the background when the refs or the day changed.
    pub(super) fn scan_pulse(&mut self, cx: &mut Context<Self>) {
        let now = chrono::Utc::now().timestamp();
        let offset = chrono::Local::now().offset().local_minus_utc() as i64;
        let key = (
            self.data.refs.fingerprint,
            (now + offset).div_euclid(86_400),
        );
        if self.pulse.key == Some(key) {
            return;
        }
        self.pulse.key = Some(key);
        let repo = self.repo.clone();
        self.pulse._scan = Some(cx.spawn(async move |this, cx| {
            let days = cx
                .background_spawn(async move { repo.daily_commits(DAYS, now, offset) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(days) = days {
                    this.pulse.days = Arc::new(days);
                    cx.notify();
                }
            });
        }));
    }

    pub fn pulse_days(&self) -> Arc<Vec<u32>> {
        self.pulse.days.clone()
    }

    /// The pulse strip at the foot of the sidebar (nothing until counted).
    pub(super) fn render_pulse(&self) -> Option<AnyElement> {
        let days = self.pulse.days.clone();
        if days.is_empty() {
            return None;
        }
        let n = days.len();
        let max = days.iter().copied().max().unwrap_or(0).max(1) as f32;
        let today = chrono::Local::now().date_naive();
        let columns = days.iter().enumerate().map(|(i, &v)| {
            let ago = (n - 1 - i) as i64;
            let when = if ago == 0 {
                tr("aujourd'hui", "today").to_string()
            } else {
                (today - chrono::Duration::days(ago))
                    .format("%d/%m")
                    .to_string()
            };
            let tip = match v {
                0 => trf!("Aucun commit — {when}", "No commits — {when}"),
                1 => trf!("1 commit — {when}", "1 commit — {when}"),
                _ => trf!("{v} commits — {when}", "{v} commits — {when}"),
            };
            let group = SharedString::from(format!("pulse-day-{i}"));
            let y = PAD + (1. - v as f32 / max) * (HEIGHT - 2. * PAD);
            div()
                .id(("pulse-day", i))
                .group(group.clone())
                .relative()
                .flex_1()
                .h_full()
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                // Hover: a faint guide down to the baseline, and the point lit.
                .child(
                    div()
                        .absolute()
                        .top(px(y))
                        .bottom_0()
                        .left(gpui_kit::relative(0.5))
                        .w(px(1.))
                        .bg(theme::alpha(theme::accent(), 0.35))
                        .opacity(0.)
                        .group_hover(group.clone(), |s| s.opacity(1.)),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(y - DOT))
                        .left(gpui_kit::relative(0.5))
                        .ml(px(-DOT))
                        .size(px(2. * DOT))
                        .rounded_full()
                        .bg(theme::accent())
                        .border_1()
                        .border_color(gpui_kit::white())
                        .opacity(0.)
                        .group_hover(group, |s| s.opacity(1.)),
                )
        });
        let painted = days.clone();
        Some(
            v_flex()
                .id("sidebar-pulse")
                .flex_none()
                .px_3()
                .pt_2()
                .pb_2p5()
                .border_t_1()
                .border_color(theme::hairline())
                .child(
                    div()
                        .relative()
                        .h(px(HEIGHT))
                        .child(
                            canvas(
                                |_, _, _| {},
                                move |bounds, _, window, _: &mut App| {
                                    paint(&painted, bounds, window)
                                },
                            )
                            .size_full(),
                        )
                        .child(h_flex().absolute().inset_0().children(columns)),
                )
                .with_animation(
                    "sidebar-pulse-in",
                    Animation::new(Duration::from_millis(600)),
                    |el, t| el.opacity(t),
                )
                .into_any_element(),
        )
    }
}

/// Day `i`'s point: centred in its column, scaled to the busiest day.
fn points(days: &[u32], bounds: Bounds<Pixels>) -> Vec<Point<Pixels>> {
    let n = days.len().max(1) as f32;
    let max = days.iter().copied().max().unwrap_or(0).max(1) as f32;
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    days.iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = (i as f32 + 0.5) * w / n;
            let y = PAD + (1. - v as f32 / max) * (h - 2. * PAD);
            bounds.origin + point(px(x), px(y))
        })
        .collect()
}

/// Catmull-Rom through the points, as cubic Béziers; control points are kept
/// between their neighbours' heights so the curve never dips under zero.
fn curve(path: &mut PathBuilder, pts: &[Point<Pixels>]) {
    for i in 0..pts.len().saturating_sub(1) {
        let p0 = pts[i.saturating_sub(1)];
        let (p1, p2) = (pts[i], pts[i + 1]);
        let p3 = pts[(i + 2).min(pts.len() - 1)];
        let (lo, hi) = (p1.y.min(p2.y), p1.y.max(p2.y));
        let c1 = point(
            p1.x + (p2.x - p0.x) / 6.,
            (p1.y + (p2.y - p0.y) / 6.).clamp(lo, hi),
        );
        let c2 = point(
            p2.x - (p3.x - p1.x) / 6.,
            (p2.y - (p3.y - p1.y) / 6.).clamp(lo, hi),
        );
        path.cubic_bezier_to(p2, c1, c2);
    }
}

fn paint(days: &[u32], bounds: Bounds<Pixels>, window: &mut Window) {
    let pts = points(days, bounds);
    let (Some(&first), Some(&last)) = (pts.first(), pts.last()) else {
        return;
    };
    // The line runs flat to both edges from the first and last day.
    let start = point(bounds.left(), first.y);
    let end = point(bounds.right(), last.y);
    let accent = theme::accent();

    let mut fill = PathBuilder::fill();
    fill.move_to(point(bounds.left(), bounds.bottom()));
    fill.line_to(start);
    fill.line_to(first);
    curve(&mut fill, &pts);
    fill.line_to(end);
    fill.line_to(point(bounds.right(), bounds.bottom()));
    fill.close();
    if let Ok(path) = fill.build() {
        window.paint_path(
            path,
            linear_gradient(
                180.,
                linear_color_stop(theme::alpha(accent, 0.32), 0.),
                linear_color_stop(theme::alpha(accent, 0.), 1.),
            ),
        );
    }

    let mut line = PathBuilder::stroke(px(STROKE));
    line.move_to(start);
    line.line_to(first);
    curve(&mut line, &pts);
    line.line_to(end);
    if let Ok(path) = line.build() {
        window.paint_path(path, accent);
    }
}
