//! Paints one row of the commit graph with GPU paths.

use gpui_kit::{
    App, BorderStyle, Bounds, Hsla, PathBuilder, Pixels, Styled as _, Window, canvas, point, px,
    quad, transparent_black,
};
use mugetsu_git::GraphRow;

use crate::theme;

pub const LANE_WIDTH: f32 = 14.;
const PADDING: f32 = 6.;
const STROKE: f32 = 1.8;
const NODE_RADIUS: f32 = 4.5;
/// Lanes beyond this are clipped so a pathological repo can't eat the whole row.
pub const MAX_VISIBLE_LANES: u16 = 28;

pub fn column_width(max_lanes: u16) -> Pixels {
    px(PADDING * 2. + LANE_WIDTH * max_lanes.clamp(1, MAX_VISIBLE_LANES) as f32)
}

#[derive(Clone, Copy)]
pub struct NodeStyle {
    pub is_head: bool,
    pub is_merge: bool,
    pub selected: bool,
}

pub fn graph_cell(row: GraphRow, style: NodeStyle) -> impl gpui_kit::IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _cx: &mut App| paint(&row, style, bounds, window),
    )
    .size_full()
}

fn lane_x(bounds: &Bounds<Pixels>, lane: u16) -> Pixels {
    bounds.left() + px(PADDING + LANE_WIDTH * lane as f32 + LANE_WIDTH / 2.)
}

fn segment(window: &mut Window, from: (Pixels, Pixels), to: (Pixels, Pixels), color: Hsla) {
    if from.0 == to.0 {
        // Straight runs are the vast majority: a quad is cheaper and crisper than a path.
        let bounds = Bounds::new(
            point(from.0 - px(STROKE / 2.), from.1),
            gpui_kit::size(px(STROKE), to.1 - from.1),
        );
        window.paint_quad(gpui_kit::fill(bounds, color));
        return;
    }
    let mut path = PathBuilder::stroke(px(STROKE));
    path.move_to(point(from.0, from.1));
    let mid_y = (from.1 + to.1) / 2.;
    path.cubic_bezier_to(point(to.0, to.1), point(from.0, mid_y), point(to.0, mid_y));
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn circle(
    window: &mut Window,
    center: (Pixels, Pixels),
    r: f32,
    fill: Hsla,
    border: Hsla,
    border_w: f32,
) {
    let bounds = Bounds::new(
        point(center.0 - px(r), center.1 - px(r)),
        gpui_kit::size(px(2. * r), px(2. * r)),
    );
    window.paint_quad(quad(
        bounds,
        px(r),
        fill,
        px(border_w),
        border,
        BorderStyle::Solid,
    ));
}

fn paint(row: &GraphRow, style: NodeStyle, bounds: Bounds<Pixels>, window: &mut Window) {
    let top = bounds.top();
    let bottom = bounds.bottom();
    let mid = (top + bottom) / 2.;
    let visible = |l: u16| l < MAX_VISIBLE_LANES;

    for e in row
        .upper
        .iter()
        .filter(|e| visible(e.from) && visible(e.to))
    {
        let c = theme::alpha(theme::lane_color(e.color), 0.85);
        segment(
            window,
            (lane_x(&bounds, e.from), top),
            (lane_x(&bounds, e.to), mid),
            c,
        );
    }
    for e in row
        .lower
        .iter()
        .filter(|e| visible(e.from) && visible(e.to))
    {
        let c = theme::alpha(theme::lane_color(e.color), 0.85);
        segment(
            window,
            (lane_x(&bounds, e.from), mid),
            (lane_x(&bounds, e.to), bottom),
            c,
        );
    }
    if !visible(row.lane) {
        return;
    }
    let center = (lane_x(&bounds, row.lane), mid);
    let color = theme::lane_color(row.color);
    if style.is_head || style.selected {
        // Neon halo.
        circle(
            window,
            center,
            NODE_RADIUS + 4.,
            theme::alpha(color, 0.22),
            transparent_black(),
            0.,
        );
    }
    if style.is_merge {
        circle(window, center, NODE_RADIUS - 0.5, theme::panel(), color, 2.);
    } else {
        let r = if style.is_head {
            NODE_RADIUS + 1.
        } else {
            NODE_RADIUS
        };
        circle(
            window,
            center,
            r,
            color,
            theme::alpha(gpui_kit::white(), 0.35),
            if style.is_head { 1.5 } else { 0. },
        );
    }
}
