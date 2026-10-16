//! One file's page (right-click on a file › Blame / File history): every
//! commit that changed it, back to the one that created it (renames
//! followed), with what each did to it; and who last changed each line.

use std::sync::Arc;
use std::time::Duration;

use gix::ObjectId;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::diff::{Diff, DiffFile as UiDiffFile};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, AppContext as _, Context, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    div, prelude::FluentBuilder as _, px, uniform_list,
};
use mugetsu_git::{BlameLine, FileCommit};

use super::{Mode, RepoView};
use crate::i18n::tr;
use crate::{fmt, theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileTab {
    History,
    Blame,
}

/// What the file page shows, loaded in the background.
pub struct FilePage {
    pub path: String,
    /// Blame as of this commit; the working copy when `None`.
    pub rev: Option<ObjectId>,
    pub tab: FileTab,
    pub log: Option<Result<Arc<Vec<FileCommit>>, String>>,
    pub blame: Option<Result<Arc<Vec<BlameLine>>, String>>,
    /// Commit picked in the history list (its diff is shown).
    pub selected: Option<ObjectId>,
    /// View to go back to when the page closes.
    pub back: Mode,
    /// Animation key of the last open.
    pub seq: usize,
}

impl FilePage {
    fn commits(&self) -> &[FileCommit] {
        match &self.log {
            Some(Ok(log)) => log,
            _ => &[],
        }
    }
}

/// First lines of each run of lines from the same commit.
fn chunk_starts(lines: &[BlameLine]) -> Vec<bool> {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| i == 0 || lines[i - 1].id != l.id)
        .collect()
}

impl RepoView {
    /// Opens the page of `path` on `tab`; blame is as of `rev` (working copy when `None`).
    pub fn open_file(
        &mut self,
        path: String,
        rev: Option<ObjectId>,
        tab: FileTab,
        cx: &mut Context<Self>,
    ) {
        self.menu_open = false;
        self.celebrate_seq += 1;
        let back = match self.mode {
            Mode::File => self.file_page.as_ref().map_or(Mode::History, |p| p.back),
            mode => mode,
        };
        self.file_page = Some(FilePage {
            path: path.clone(),
            rev,
            tab,
            log: None,
            blame: None,
            selected: None,
            back,
            seq: self.celebrate_seq,
        });
        self.file_diff
            .update(cx, |d, cx| d.set_files(Vec::new(), cx));
        self.set_mode(Mode::File, cx);
        let repo = self.repo.clone();
        let seq = self.celebrate_seq;
        self._file_task = cx.spawn(async move |this, cx| {
            let (log, blame) = cx
                .background_spawn(async move {
                    let rev = rev.map(|r| r.to_string());
                    // From HEAD, so newer commits show too, unless `rev` is
                    // on another branch.
                    let start = rev.as_deref().filter(|r| {
                        repo.git()
                            .run(["merge-base", "--is-ancestor", r, "HEAD"])
                            .is_err()
                    });
                    let log = repo.file_log(&path, start).map_err(|e| format!("{e:#}"));
                    let blame = repo
                        .blame(&path, rev.as_deref())
                        .map_err(|e| format!("{e:#}"));
                    (log.map(Arc::new), blame.map(Arc::new))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(page) = this.file_page.as_mut().filter(|p| p.seq == seq) else {
                    return;
                };
                // The commit the page was opened from, else the newest one.
                let first = log.as_ref().ok().and_then(|log| {
                    log.iter()
                        .find(|c| Some(c.id) == rev)
                        .or_else(|| log.first())
                        .map(|c| c.id)
                });
                page.log = Some(log);
                page.blame = Some(blame);
                if let Some(id) = first {
                    this.select_file_commit(id, cx);
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    pub fn file_page(&self) -> Option<&FilePage> {
        self.file_page.as_ref()
    }

    pub fn set_file_tab(&mut self, tab: FileTab, cx: &mut Context<Self>) {
        if let Some(page) = self.file_page.as_mut() {
            page.tab = tab;
            cx.notify();
        }
    }

    fn close_file_page(&mut self, cx: &mut Context<Self>) {
        let back = self.file_page.as_ref().map_or(Mode::History, |p| p.back);
        self.set_mode(back, cx);
    }

    /// Shows what commit `id` did to the file.
    pub fn select_file_commit(&mut self, id: ObjectId, cx: &mut Context<Self>) {
        let Some(page) = self.file_page.as_mut() else {
            return;
        };
        let Some(commit) = page.commits().iter().find(|c| c.id == id).cloned() else {
            return;
        };
        page.selected = Some(id);
        let repo = self.repo.clone();
        self._file_diff_task = cx.spawn(async move |this, cx| {
            let files = cx
                .background_spawn(async move {
                    repo.file_patch(&commit)
                        .map(|patch| UiDiffFile::parse(&patch).unwrap_or_default())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.file_page.as_ref().and_then(|p| p.selected) != Some(id) {
                    return;
                }
                this.file_diff
                    .update(cx, |d, cx| d.set_files(files.unwrap_or_default(), cx));
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Blame line clicked: its commit in the file's history.
    fn show_line_commit(&mut self, id: ObjectId, cx: &mut Context<Self>) {
        if id.is_null() {
            return;
        }
        self.set_file_tab(FileTab::History, cx);
        self.select_file_commit(id, cx);
    }

    fn show_in_history(&mut self, id: ObjectId, cx: &mut Context<Self>) {
        self.set_mode(Mode::History, cx);
        self.select_commit(id, true, cx);
    }

    pub(super) fn render_file_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(page) = self.file_page.as_ref() else {
            return div().into_any_element();
        };
        let body = match page.tab {
            FileTab::History => self.render_file_history(page, cx),
            FileTab::Blame => self.render_blame(page, cx),
        };
        v_flex()
            .id("file-view")
            .test_support()
            .size_full()
            .child(self.render_file_header(page, cx))
            .child(div().flex_1().min_h_0().child(body).with_animation(
                ("file-view-in", page.seq),
                Animation::new(Duration::from_millis(300)),
                |el, t| {
                    let k = 1. - (1. - t).powi(3);
                    el.opacity(k).mt(px(8. * (1. - k)))
                },
            ))
            .into_any_element()
    }

    fn render_file_header(&self, page: &FilePage, cx: &mut Context<Self>) -> AnyElement {
        let (dir, name) = page
            .path
            .rsplit_once('/')
            .map_or(("", page.path.as_str()), |(d, n)| (d, n));
        let count = page.commits().len();
        let tab = |id: &'static str, icon: Lucide, label: String, on: bool| {
            h_flex()
                .id(id)
                .test_support()
                .gap_1p5()
                .px_2p5()
                .h(px(24.))
                .rounded_md()
                .text_xs()
                .cursor_pointer()
                .when(on, |s| {
                    s.bg(theme::alpha(theme::accent(), 0.18))
                        .text_color(theme::accent())
                })
                .when(!on, |s| {
                    s.text_color(theme::muted())
                        .hover(|s| s.bg(theme::raised()))
                })
                .child(Icon::new(icon).xsmall())
                .child(label)
        };
        let history_label = if count > 0 {
            trf!("Historique · {count}", "History · {count}")
        } else {
            tr("Historique", "History").to_string()
        };
        h_flex()
            .flex_none()
            .px_4()
            .py_2()
            .gap_3()
            .bg(theme::panel())
            .border_b_1()
            .border_color(theme::hairline())
            .child(
                Icon::new(Lucide::FileClock)
                    .small()
                    .text_color(theme::accent()),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(
                        div()
                            .flex_none()
                            .text_sm()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(name.to_string()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(dir.to_string()),
                    )
                    .when_some(page.rev, |s, rev| {
                        s.child(
                            div()
                                .flex_none()
                                .px_1p5()
                                .rounded_md()
                                .text_xs()
                                .bg(theme::alpha(theme::accent(), 0.12))
                                .text_color(theme::accent())
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(rev.to_hex_with_len(7).to_string()),
                        )
                    }),
            )
            .child(
                h_flex()
                    .p(px(2.))
                    .gap_0p5()
                    .rounded_lg()
                    .bg(theme::alpha(gpui_kit::white(), 0.05))
                    .border_1()
                    .border_color(theme::hairline())
                    .child(
                        tab(
                            "file-tab-history",
                            Lucide::FileClock,
                            history_label,
                            page.tab == FileTab::History,
                        )
                        .on_click(
                            cx.listener(|this, _, _, cx| this.set_file_tab(FileTab::History, cx)),
                        ),
                    )
                    .child(
                        tab(
                            "file-tab-blame",
                            Lucide::ScanText,
                            "Blame".into(),
                            page.tab == FileTab::Blame,
                        )
                        .on_click(
                            cx.listener(|this, _, _, cx| this.set_file_tab(FileTab::Blame, cx)),
                        ),
                    ),
            )
            .child(
                Button::new("file-close")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(Lucide::X))
                    .tooltip(tr("Fermer", "Close"))
                    .on_click(cx.listener(|this, _, _, cx| this.close_file_page(cx))),
            )
            .into_any_element()
    }

    fn render_file_history(&self, page: &FilePage, cx: &mut Context<Self>) -> AnyElement {
        let commits = match &page.log {
            None => return loading(tr("Lecture de l'historique…", "Reading history…")),
            Some(Err(e)) => return note(e.clone()),
            Some(Ok(log)) if log.is_empty() => {
                return note(tr(
                    "Aucun commit n'a encore touché ce fichier.",
                    "No commit has touched this file yet.",
                ));
            }
            Some(Ok(log)) => log.clone(),
        };
        let now = fmt::now();
        let last = commits.len() - 1;
        let rows = commits.iter().enumerate().map(|(i, c)| {
            let id = c.id;
            let active = page.selected == Some(id);
            let event = match (c.status, &c.old_path) {
                ('R', Some(from)) => Some((
                    Lucide::ArrowRightLeft,
                    theme::violet(),
                    trf!("Renommé depuis {from}", "Renamed from {from}"),
                )),
                ('A', _) => Some((
                    Lucide::FilePlus,
                    theme::lime(),
                    tr("Créé", "Created").into(),
                )),
                ('D', _) => Some((
                    Lucide::FileMinus,
                    theme::red(),
                    tr("Supprimé", "Deleted").into(),
                )),
                _ if i == last => Some((
                    Lucide::FilePlus,
                    theme::lime(),
                    tr("Premier commit connu", "Oldest known commit").into(),
                )),
                _ => None,
            };
            h_flex()
                .id(SharedString::from(format!("file-commit-{}", c.short_id())))
                .test_support()
                .w_full()
                .px_2()
                .py_1p5()
                .gap_2()
                .rounded_md()
                .text_xs()
                .cursor_pointer()
                .when(active, |s| s.bg(theme::alpha(theme::accent(), 0.18)))
                .when(!active, |s| s.hover(|s| s.bg(theme::raised())))
                .child(crate::avatars::avatar(&c.email, &c.author, 20., cx))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(div().truncate().text_sm().child(c.summary.clone()))
                        .child(div().truncate().text_color(theme::muted()).child(format!(
                            "{} · {} · {}",
                            c.short_id(),
                            c.author,
                            fmt::relative_time(c.time, now)
                        )))
                        .when_some(event, |s, (icon, color, text)| {
                            s.child(
                                h_flex()
                                    .gap_1()
                                    .min_w_0()
                                    .text_color(color)
                                    .child(Icon::new(icon).xsmall())
                                    .child(div().truncate().child(text)),
                            )
                        }),
                )
                .when(active, |s| {
                    s.child(
                        Button::new(SharedString::from(format!("file-commit-open-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(Lucide::GitCommitHorizontal))
                            .tooltip(tr("Voir dans l'historique", "Show in history"))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.show_in_history(id, cx)),
                            ),
                    )
                })
                .on_click(cx.listener(move |this, ev: &gpui_kit::ClickEvent, _, cx| {
                    if ev.click_count() >= 2 {
                        this.show_in_history(id, cx);
                    } else {
                        this.select_file_commit(id, cx);
                    }
                }))
        });
        let rows: Vec<_> = rows.collect();
        h_flex()
            .size_full()
            .min_h_0()
            .child(
                v_flex()
                    .id("file-commits")
                    .flex_none()
                    .w(px(320.))
                    .h_full()
                    .p_1()
                    .gap_0p5()
                    .overflow_y_scroll()
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .bg(theme::panel())
                    .children(rows),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .p_2()
                    .child(Diff::new(&self.file_diff).line_number(true)),
            )
            .into_any_element()
    }

    fn render_blame(&self, page: &FilePage, cx: &mut Context<Self>) -> AnyElement {
        let lines = match &page.blame {
            None => return loading(tr("Calcul du blame…", "Computing blame…")),
            Some(Err(e)) => return note(e.clone()),
            Some(Ok(lines)) if lines.is_empty() => {
                return note(tr("Fichier vide.", "Empty file."));
            }
            Some(Ok(lines)) => lines.clone(),
        };
        let starts = Arc::new(chunk_starts(&lines));
        // Age shading: the newest commit glows, the oldest fades out.
        let (oldest, newest) = lines
            .iter()
            .filter(|l| !l.is_uncommitted())
            .fold((i64::MAX, i64::MIN), |(lo, hi), l| {
                (lo.min(l.time), hi.max(l.time))
            });
        let span = (newest - oldest).max(1) as f32;
        let now = fmt::now();
        let mono = cx.theme().mono_font_family.clone();
        let muted = cx.theme().muted_foreground;
        let number_width = px(10. + 7.5 * lines.len().to_string().len() as f32);
        let view = cx.entity().downgrade();
        uniform_list("blame", lines.len(), move |range, _, _| {
            range
                .map(|ix| {
                    let l = &lines[ix];
                    let start = starts[ix];
                    let age = if l.is_uncommitted() {
                        1.
                    } else {
                        (l.time - oldest) as f32 / span
                    };
                    let bar = if l.is_uncommitted() {
                        theme::amber()
                    } else {
                        theme::alpha(theme::accent(), 0.15 + 0.75 * age)
                    };
                    let id = l.id;
                    let view = view.clone();
                    let gutter = h_flex()
                        .flex_none()
                        .w(px(300.))
                        .h_full()
                        .px_2()
                        .gap_2()
                        .text_xs()
                        .text_color(muted)
                        .when(start, |s| {
                            if l.is_uncommitted() {
                                s.child(
                                    div()
                                        .text_color(theme::amber())
                                        .child(tr("Non commité", "Not committed yet")),
                                )
                            } else {
                                s.child(
                                    div()
                                        .flex_none()
                                        .font_family(mono.clone())
                                        .text_color(theme::alpha(theme::accent(), 0.8))
                                        .child(l.id.to_hex_with_len(7).to_string()),
                                )
                                .child(div().flex_1().min_w_0().truncate().child(l.summary.clone()))
                                .child(
                                    div()
                                        .flex_none()
                                        .max_w(px(90.))
                                        .truncate()
                                        .child(l.author.clone()),
                                )
                                .child(div().flex_none().child(fmt::relative_time(l.time, now)))
                            }
                        });
                    h_flex()
                        .id(("blame-line", ix))
                        .test_support()
                        .h(px(20.))
                        .w_full()
                        .when(start && ix > 0, |s| {
                            s.border_t_1().border_color(theme::hairline())
                        })
                        .hover(|s| s.bg(theme::raised()))
                        .when(!l.is_uncommitted(), |s| s.cursor_pointer())
                        .child(div().flex_none().w(px(3.)).h_full().bg(bar))
                        .child(gutter)
                        .child(
                            div()
                                .flex_none()
                                .w(number_width)
                                .pr_2()
                                .text_right()
                                .text_xs()
                                .text_color(theme::muted())
                                .font_family(mono.clone())
                                .child(l.line.to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .font_family(mono.clone())
                                .child(l.text.replace('\t', "    ")),
                        )
                        .on_click(move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| this.show_line_commit(id, cx));
                        })
                        .into_any_element()
                })
                .collect()
        })
        .size_full()
        .into_any_element()
    }
}

fn loading(text: impl Into<SharedString>) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .text_sm()
        .text_color(theme::muted())
        .child(Spinner::new().color(theme::accent()))
        .child(text.into())
        .into_any_element()
}

fn note(text: impl Into<SharedString>) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .px_6()
        .text_sm()
        .text_color(theme::muted())
        .child(text.into())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(n: u8) -> BlameLine {
        BlameLine {
            id: ObjectId::from_bytes_or_panic(&[n; 20]),
            author: String::new(),
            email: String::new(),
            time: 0,
            summary: String::new(),
            line: 0,
            text: String::new(),
        }
    }

    #[test]
    fn a_chunk_starts_where_the_commit_changes() {
        let lines = [line(1), line(1), line(2), line(1)];
        assert_eq!(chunk_starts(&lines), [true, false, true, true]);
    }
}
