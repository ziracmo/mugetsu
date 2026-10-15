//! "Copy GitHub link" for the selected commit, a file of its diff, or the
//! lines picked in the diff gutter, with a flourish on whatever was clicked:
//! a ring opening out, sparks and a link glyph rising into the clipboard.

use std::f32::consts::TAU;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::diff::{DiffFile, DiffFileStatus, DiffLineRange, DiffSide};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, ClipboardItem, Context, InteractiveElement as _,
    IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, WeakEntity,
    Window, div, prelude::FluentBuilder, px, relative,
};
use mugetsu_forge::GithubRepo;

use super::RepoView;
use crate::i18n::tr;
use crate::theme;

/// What a link was just copied from (its element plays the flourish).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkSpot {
    Commit,
    File(SharedString),
    Lines,
}

/// Where the lines of a diff selection live on GitHub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinesLink {
    /// One side: a file permalink at the commit (new lines) or its parent (old lines).
    Blob {
        sha: String,
        path: String,
        lines: (usize, usize),
    },
    /// Across both sides: the commit page, anchored in the file's diff.
    Diff {
        path: String,
        from: (bool, usize),
        to: (bool, usize),
    },
}

/// Resolves a gutter selection for commit `sha` (whose diff is against `parent`).
pub fn lines_link(range: &DiffLineRange, sha: &str, parent: Option<&str>) -> Option<LinesLink> {
    let path = range.path().to_string();
    let new = |side| side == DiffSide::Modified;
    if range.side() != range.end_side() {
        return Some(LinesLink::Diff {
            path,
            from: (new(range.side()), range.start()),
            to: (new(range.end_side()), range.end()),
        });
    }
    let sha = if new(range.side()) {
        sha.to_string()
    } else {
        parent?.to_string()
    };
    Some(LinesLink::Blob {
        sha,
        path,
        lines: (
            range.start().min(range.end()),
            range.start().max(range.end()),
        ),
    })
}

impl LinesLink {
    pub fn url(&self, gh: &GithubRepo, commit: &str) -> String {
        match self {
            Self::Blob { sha, path, lines } => gh.blob_url(sha, path, Some(*lines)),
            Self::Diff { path, from, to } => gh.commit_diff_url(commit, path, Some((*from, *to))),
        }
    }

    /// `L12`, `L12–18`, `−L4` for removed lines.
    pub fn label(&self, commit: &str) -> String {
        match self {
            Self::Blob { sha, lines, .. } => {
                let old = if sha == commit { "" } else { "−" };
                match lines {
                    (a, b) if a == b => format!("{old}L{a}"),
                    (a, b) => format!("{old}L{a}–{b}"),
                }
            }
            Self::Diff { from, to, .. } => {
                let side = |(new, n): (bool, usize)| format!("{}{n}", if new { "+" } else { "−" });
                format!("{}…{}", side(*from), side(*to))
            }
        }
    }
}

/// A file of the commit's diff on GitHub: deleted files at the parent.
fn file_url(gh: &GithubRepo, file: &DiffFile, sha: &str, parent: Option<&str>) -> Option<String> {
    if file.status() == DiffFileStatus::Deleted {
        let path = file.original_path().unwrap_or(file.path());
        return Some(gh.blob_url(parent?, path, None));
    }
    Some(gh.blob_url(sha, file.path(), None))
}

impl RepoView {
    pub fn repo_github(&self) -> Option<&GithubRepo> {
        self.data.github.as_ref()
    }

    /// Selected commit and its first parent (the diff's base), as hex.
    fn link_commit(&self) -> Option<(String, Option<String>)> {
        let row = self.data.history.row_of(&self.selected?)?;
        let commit = &self.data.history.commits[row];
        Some((
            commit.id.to_string(),
            commit.parents.first().map(|p| p.to_string()),
        ))
    }

    /// Copies `url` and plays the flourish on `spot`.
    pub(super) fn copy_link(&mut self, spot: LinkSpot, url: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(url));
        self.celebrate_seq += 1;
        let seq = self.celebrate_seq;
        self.link_copied = Some((spot, seq));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.link_copied.as_ref().is_some_and(|(_, s)| *s == seq) {
                    this.link_copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn copy_commit_link(&mut self, cx: &mut Context<Self>) {
        let (Some(gh), Some((sha, _))) = (self.data.github.clone(), self.link_commit()) else {
            return;
        };
        self.copy_link(LinkSpot::Commit, gh.commit_url(&sha), cx);
    }

    fn copy_lines_link(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(url) = self.lines_url() else {
            return false;
        };
        self.copy_link(LinkSpot::Lines, url.0, cx);
        true
    }

    /// ⌥⌘C: the selected lines' link, else the commit's.
    pub(super) fn copy_github_link(&mut self, cx: &mut Context<Self>) {
        if !self.copy_lines_link(cx) {
            self.copy_commit_link(cx);
        }
    }

    /// Link and label of the lines selected in the commit's diff.
    fn lines_url(&self) -> Option<(String, String)> {
        let gh = self.data.github.as_ref()?;
        let range = self.line_selection.as_ref()?;
        let (sha, parent) = self.link_commit()?;
        let link = lines_link(range, &sha, parent.as_deref())?;
        Some((link.url(gh, &sha), link.label(&sha)))
    }

    /// Pretends the repository is on GitHub (no network in tests).
    #[cfg(test)]
    pub fn set_github(&mut self, gh: GithubRepo, cx: &mut Context<Self>) {
        std::sync::Arc::make_mut(&mut self.data).github = Some(gh);
        cx.notify();
    }

    #[cfg(test)]
    pub fn set_line_selection(&mut self, range: DiffLineRange, cx: &mut Context<Self>) {
        self.line_selection = Some(range);
        self.line_selection_seq += 1;
        cx.notify();
    }

    #[cfg(test)]
    pub fn link_feedback(&self) -> Option<LinkSpot> {
        self.link_copied.as_ref().map(|(s, _)| s.clone())
    }

    fn flourish_seq(&self, spot: &LinkSpot) -> Option<usize> {
        self.link_copied
            .as_ref()
            .filter(|(s, _)| s == spot)
            .map(|(_, seq)| *seq)
    }

    /// "GitHub" chip beside the SHA in the commit header.
    pub(super) fn render_commit_link(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.data.github.as_ref()?;
        let done = self.flourish_seq(&LinkSpot::Commit);
        Some(
            link_chip("copy-commit-link", done)
                .child(match done {
                    Some(seq) => copied_label(seq).into_any_element(),
                    None => div().child("GitHub").into_any_element(),
                })
                .tooltip(|window, cx| {
                    Tooltip::new(tr(
                        "Copier le lien GitHub du commit",
                        "Copy commit GitHub link",
                    ))
                    .key_binding(gpui_kit::component::kbd::Kbd::binding_for_action(
                        &super::CopyGithubLink,
                        Some("Repo"),
                        window,
                    ))
                    .build(window, cx)
                })
                .on_click(cx.listener(|this, _, _, cx| this.copy_commit_link(cx)))
                .into_any_element(),
        )
    }

    /// Floating pill over the diff while lines are selected in its gutter.
    pub(super) fn render_lines_link(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (_, label) = self.lines_url()?;
        let done = self.flourish_seq(&LinkSpot::Lines);
        let key = self.line_selection_seq;
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    link_chip("copy-lines-link", done)
                        .px_3()
                        .py_1()
                        .rounded_full()
                        .bg(theme::popover())
                        .border_1()
                        .border_color(theme::glass_edge())
                        .shadow_lg()
                        .child(match done {
                            Some(seq) => copied_label(seq).into_any_element(),
                            None => h_flex()
                                .gap_1p5()
                                .child(
                                    div()
                                        .text_color(gpui_kit::white())
                                        .child(tr("Copier le lien GitHub", "Copy GitHub link")),
                                )
                                .child(
                                    div()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .text_color(theme::muted())
                                        .child(label),
                                )
                                .into_any_element(),
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.copy_lines_link(cx);
                        })),
                )
                // Springs up from below when a selection appears.
                .with_animation(
                    ("lines-link-in", key),
                    Animation::new(Duration::from_millis(320)),
                    |el, t| {
                        let k = spring(t);
                        el.opacity(t).bottom(px(12. - 14. * (1. - k)))
                    },
                )
                .into_any_element(),
        )
    }

    /// Link button at the end of each file header of the commit's diff.
    pub(super) fn file_link_renderer(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl Fn(&DiffFile, &mut Window, &mut App) -> AnyElement + 'static> {
        let gh = self.data.github.clone()?;
        let (sha, parent) = self.link_commit()?;
        let view: WeakEntity<Self> = cx.entity().downgrade();
        let copied = self.link_copied.clone();
        Some(move |file: &DiffFile, _: &mut Window, _: &mut App| {
            let path = file.path().clone();
            let done = copied
                .as_ref()
                .filter(|(s, _)| *s == LinkSpot::File(path.clone()))
                .map(|(_, seq)| *seq);
            let url = file_url(&gh, file, &sha, parent.as_deref());
            let view = view.clone();
            link_chip(SharedString::from(format!("copy-file-link-{path}")), done)
                .when_some(done, |s, seq| s.child(copied_label(seq)))
                .when(url.is_none(), |s| s.opacity(0.4))
                .tooltip(|window, cx| {
                    Tooltip::new(tr(
                        "Copier le lien GitHub du fichier",
                        "Copy file GitHub link",
                    ))
                    .build(window, cx)
                })
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    if let Some(url) = url.clone() {
                        let path = path.clone();
                        let _ = view
                            .update(cx, |this, cx| this.copy_link(LinkSpot::File(path), url, cx));
                    }
                })
                .into_any_element()
        })
    }
}

/// Clickable link glyph; once `done`, a check with the flourish around it.
fn link_chip(
    id: impl Into<gpui_kit::ElementId>,
    done: Option<usize>,
) -> impl ParentElement + Styled + StatefulInteractiveElement + IntoElement + FluentBuilder {
    h_flex()
        .id(id)
        .test_support()
        .relative()
        .flex_none()
        .gap_1()
        .items_center()
        .cursor_pointer()
        .text_xs()
        .text_color(theme::accent())
        .hover(|s| s.text_color(theme::magenta()))
        .map(|s| match done {
            Some(seq) => s
                .text_color(theme::lime())
                .child(Icon::new(Lucide::Check).xsmall().with_animation(
                    ("link-check", seq),
                    Animation::new(Duration::from_millis(380)),
                    |el, t| el.opacity(t.min(0.5) * 2.).mt(px(4. * (1. - spring(t)))),
                ))
                .child(flourish(seq)),
            None => s.child(Icon::new(Lucide::Link).xsmall()),
        })
}

fn copied_label(seq: usize) -> impl IntoElement {
    div()
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .child(tr("Lien copié !", "Link copied!"))
        .with_animation(
            ("link-copied", seq),
            Animation::new(Duration::from_millis(260)).with_easing(gpui_kit::ease_out_quint()),
            |el, t| el.opacity(t).ml(px(6. * (1. - t))),
        )
}

/// Overshoots a little before settling (0 → ~1.08 → 1).
fn spring(t: f32) -> f32 {
    // easeOutBack
    let (c1, t) = (1.70158, t.clamp(0., 1.) - 1.);
    1. + (c1 + 1.) * t.powi(3) + c1 * t.powi(2)
}

const SPARKS: usize = 10;

/// Overlay centred on a `relative()` parent: an accent ring opening out,
/// sparks thrown around, and a link glyph rising into the clipboard.
pub fn flourish(seq: usize) -> AnyElement {
    let ring = div()
        .absolute()
        .rounded_full()
        .border_2()
        .border_color(theme::accent())
        .with_animation(
            ("link-ring", seq),
            Animation::new(Duration::from_millis(620)).with_easing(gpui_kit::ease_out_quint()),
            |el, t| {
                let r = 6. + 18. * t;
                el.size(px(r * 2.))
                    .left(px(-r))
                    .top(px(-r))
                    .opacity(0.9 * (1. - t))
            },
        );
    let sparks = (0..SPARKS).map(move |i| {
        let angle = TAU * i as f32 / SPARKS as f32 + 0.3;
        let reach = 18. + (i % 3) as f32 * 7.;
        let (dx, dy) = (angle.cos() * reach * 1.3, angle.sin() * reach);
        let color = if i % 2 == 0 {
            theme::accent()
        } else {
            theme::lane_color(i as u16)
        };
        div()
            .absolute()
            .size(px(3.))
            .rounded_full()
            .bg(color)
            .with_animation(
                ("link-spark", seq * SPARKS + i),
                Animation::new(Duration::from_millis(700 + (i % 4) as u64 * 50))
                    .with_easing(gpui_kit::ease_out_quint()),
                move |el, t| {
                    el.left(px(dx * t - 1.5))
                        .top(px(dy * t - 1.5))
                        .opacity(1. - t * t)
                },
            )
    });
    let glyph = div()
        .absolute()
        .child(Icon::new(Lucide::Link).xsmall().text_color(theme::accent()))
        .with_animation(
            ("link-rise", seq),
            Animation::new(Duration::from_millis(900)).with_easing(gpui_kit::ease_out_quint()),
            |el, t| {
                el.left(px(-6.)).top(px(-6. - 26. * t)).opacity(if t < 0.2 {
                    t * 5.
                } else {
                    1. - (t - 0.2) / 0.8
                })
            },
        );
    div()
        .absolute()
        .top(relative(0.5))
        .left(px(6.))
        .size_0()
        .child(ring)
        .children(sparks)
        .child(glyph)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_selections_become_github_links() {
        let gh = GithubRepo {
            owner: "o".into(),
            name: "r".into(),
        };
        let new = DiffLineRange::new("src/a.rs", DiffSide::Modified, 9, 3);
        let link = lines_link(&new, "c0ffee", Some("bead")).unwrap();
        assert_eq!(
            link.url(&gh, "c0ffee"),
            "https://github.com/o/r/blob/c0ffee/src/a.rs#L3-L9"
        );
        assert_eq!(link.label("c0ffee"), "L3–9");

        let old = DiffLineRange::new("src/a.rs", DiffSide::Original, 4, 4);
        let link = lines_link(&old, "c0ffee", Some("bead")).unwrap();
        assert_eq!(
            link.url(&gh, "c0ffee"),
            "https://github.com/o/r/blob/bead/src/a.rs#L4"
        );
        assert_eq!(link.label("c0ffee"), "−L4");
        assert_eq!(lines_link(&old, "c0ffee", None), None, "root commit");

        let both =
            DiffLineRange::new("a.rs", DiffSide::Original, 4, 6).with_end_side(DiffSide::Modified);
        let link = lines_link(&both, "c0ffee", Some("bead")).unwrap();
        assert!(link.url(&gh, "c0ffee").ends_with("L4-R6"));
        assert_eq!(link.label("c0ffee"), "−4…+6");
    }

    #[test]
    fn spring_settles_at_one() {
        assert_eq!(spring(0.), 0.);
        assert!((spring(1.) - 1.).abs() < 1e-6);
        assert!((0..=20).any(|i| spring(i as f32 / 20.) > 1.), "overshoots");
    }
}
