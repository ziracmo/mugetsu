//! Rendered before/after preview of an SVG in the Changes view, above its text diff.
//! Anything that fails to load or render is simply left out.

use std::sync::{Arc, LazyLock};

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, IntoElement, ObjectFit, ParentElement as _, RenderImage, Size, Styled as _,
    StyledImage as _, SvgRenderer, SvgSize, div, img, px,
};
use mugetsu_git::Repo;

use crate::i18n::tr;
use crate::theme;

/// Longest side of the rasterised preview, in device pixels (sharp on retina).
const RENDER_PX: f32 = 1024.;
/// Bigger files aren't worth rasterising for a preview.
const MAX_BYTES: usize = 8 << 20;

pub(super) struct SvgPreview {
    before: Option<Arc<RenderImage>>,
    after: Option<Arc<RenderImage>>,
}

pub(super) fn is_svg(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("svg"))
}

/// Rasterises `bytes`, or `None` when it isn't an SVG resvg can draw.
pub(super) fn rasterize(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    static RENDERER: LazyLock<SvgRenderer> = LazyLock::new(|| SvgRenderer::new(Arc::new(())));
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return None;
    }
    let size = usvg::Tree::from_data(bytes, &usvg::Options::default())
        .ok()?
        .size();
    let width = RENDER_PX * (size.width() / size.height()).min(1.);
    let parsed = RENDERER.parse_svg(bytes).ok()?;
    let width = gpui_kit::DevicePixels(width.max(1.) as i32);
    RENDERER
        .render_parsed(&parsed, SvgSize::Size(Size::new(width, width)))
        .ok()
}

impl SvgPreview {
    /// Before/after of `path` for the Changes view (index→worktree, or HEAD→index when
    /// `staged`). `None` when neither side renders. Blocking: call off the main thread.
    pub(super) fn load(repo: &Repo, path: &str, staged: bool) -> Option<Self> {
        if !is_svg(path) {
            return None;
        }
        let after = if staged {
            repo.base_blob(false, path)
        } else {
            std::fs::read(repo.root().join(path)).ok()
        };
        let preview = Self {
            before: repo.base_blob(staged, path).as_deref().and_then(rasterize),
            after: after.as_deref().and_then(rasterize),
        };
        (preview.before.is_some() || preview.after.is_some()).then_some(preview)
    }

    pub(super) fn render(&self, cx: &App) -> AnyElement {
        let side = |label: &'static str, image: &Option<Arc<RenderImage>>| {
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(label.to_uppercase()),
                )
                .child(
                    div()
                        .h(px(180.))
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(theme::raised())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(match image {
                            Some(image) => img(image.clone())
                                .size_full()
                                .object_fit(ObjectFit::Contain)
                                .into_any_element(),
                            None => div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("—")
                                .into_any_element(),
                        }),
                )
        };
        h_flex()
            .flex_none()
            .gap_2()
            .px_2()
            .pt_2()
            .child(side(tr("Avant", "Before"), &self.before))
            .child(side(tr("Après", "After"), &self.after))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_valid_svg_and_skips_broken_ones() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="32"><rect width="16" height="32" fill="red"/></svg>"#;
        let image = rasterize(svg).expect("valid svg renders");
        let size = image.size(0);
        assert_eq!(i32::from(size.height), RENDER_PX as i32);
        assert_eq!(i32::from(size.width), RENDER_PX as i32 / 2);
        assert!(rasterize(b"<svg").is_none());
        assert!(rasterize(b"not an svg").is_none());
        assert!(rasterize(b"").is_none());
        assert!(is_svg("icons/Logo.SVG"));
        assert!(!is_svg("svg/readme.md"));
    }
}
