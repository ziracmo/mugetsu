//! ⌘O: pick a repository among the recent ones and those found in the
//! folders set in the settings, or browse for another one.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _, px,
};

use crate::i18n::tr;
use crate::theme;

/// Repositories in `folders`: the folders themselves, their children and
/// grandchildren holding a `.git`. Sorted by name, without duplicates.
pub fn find_repos(folders: &[PathBuf]) -> Vec<PathBuf> {
    fn is_repo(p: &Path) -> bool {
        p.join(".git").exists()
    }
    fn subdirs(p: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(p)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                    .map(|e| e.path())
                    .collect()
            })
            .unwrap_or_default()
    }
    let mut out = Vec::new();
    for folder in folders {
        if is_repo(folder) {
            out.push(folder.clone());
            continue;
        }
        for child in subdirs(folder) {
            if is_repo(&child) {
                out.push(child);
            } else {
                out.extend(subdirs(&child).into_iter().filter(|g| is_repo(g)));
            }
        }
    }
    out.sort_by_key(|p| name_of(p).to_lowercase());
    out.dedup();
    out
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.display().to_string())
}

/// Every word of `query` in the repository's path (case-insensitive).
fn matches(path: &Path, query: &str) -> bool {
    let hay = path.display().to_string().to_lowercase();
    query
        .split_whitespace()
        .all(|w| hay.contains(&w.to_lowercase()))
}

type Pick = Rc<dyn Fn(PathBuf, &mut Window, &mut App)>;

pub struct OpenRepoForm {
    input: Entity<InputState>,
    recent: Vec<PathBuf>,
    found: Vec<PathBuf>,
    pick: Pick,
    _input: Subscription,
}

impl OpenRepoForm {
    fn visible(&self, cx: &App) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let query = self.input.read(cx).value().trim().to_string();
        let recent: Vec<PathBuf> = self
            .recent
            .iter()
            .filter(|p| matches(p, &query))
            .cloned()
            .collect();
        let found = self
            .found
            .iter()
            .filter(|p| !recent.contains(p) && matches(p, &query))
            .cloned()
            .collect();
        (recent, found)
    }
}

fn repo_row(index: usize, path: &Path, pick: Pick) -> impl IntoElement {
    let target = path.to_path_buf();
    let icon = crate::settings::get().repo_icons.get(path).cloned();
    h_flex()
        .id(SharedString::from(format!("open-repo-{index}")))
        .test_support()
        .h(px(30.))
        .px_2()
        .gap_2()
        .rounded_md()
        .cursor_pointer()
        .hover(|s| s.bg(theme::raised()))
        .child(match icon {
            Some(emoji) => div().w(px(16.)).child(emoji).into_any_element(),
            None => Icon::new(Lucide::FolderGit2)
                .xsmall()
                .text_color(theme::muted())
                .into_any_element(),
        })
        .child(div().text_sm().child(name_of(path)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(theme::muted())
                .child(path.display().to_string()),
        )
        .on_click(move |_, window, cx| {
            window.close_dialog(cx);
            pick(target.clone(), window, cx);
        })
}

impl Render for OpenRepoForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (recent, found) = self.visible(cx);
        let title = |text: &'static str| {
            div()
                .pt_2()
                .pb_1()
                .px_2()
                .text_xs()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(theme::muted())
                .child(text)
        };
        let empty = recent.is_empty() && found.is_empty();
        let mut i = 0;
        let mut rows = |list: &[PathBuf]| {
            list.iter()
                .map(|p| {
                    i += 1;
                    repo_row(i - 1, p, self.pick.clone()).into_any_element()
                })
                .collect::<Vec<_>>()
        };
        let recent_rows = rows(&recent);
        let found_rows = rows(&found);
        v_flex()
            .gap_2()
            .child(
                Input::new(&self.input).id("open-repo-search").prefix(
                    Icon::new(Lucide::Search)
                        .xsmall()
                        .text_color(theme::muted()),
                ),
            )
            .child(
                v_flex()
                    .id("open-repo-list")
                    .h(px(320.))
                    .overflow_y_scroll()
                    .when(!recent_rows.is_empty(), |s| {
                        s.child(title(tr("Récents", "Recent")))
                            .children(recent_rows)
                    })
                    .when(!found_rows.is_empty(), |s| {
                        s.child(title(tr("Dans vos dossiers", "In your folders")))
                            .children(found_rows)
                    })
                    .when(empty, |s| {
                        s.child(
                            div()
                                .p_4()
                                .text_sm()
                                .text_color(theme::muted())
                                .child(tr("Aucun dépôt trouvé", "No repository found")),
                        )
                    }),
            )
    }
}

/// Opens the picker; `pick` opens the chosen path, `browse` the native dialog.
pub fn open(
    window: &mut Window,
    cx: &mut App,
    pick: impl Fn(PathBuf, &mut Window, &mut App) + 'static,
    browse: impl Fn(&mut Window, &mut App) + 'static,
) {
    let settings = crate::settings::get();
    let pick: Pick = Rc::new(pick);
    let input = cx.new(|cx| {
        InputState::new(window, cx).placeholder(tr("Rechercher un dépôt", "Search a repository"))
    });
    let form = cx.new(|cx: &mut Context<OpenRepoForm>| {
        let sub = cx.subscribe(
            &input,
            |this: &mut OpenRepoForm, _, ev: &InputEvent, cx| match ev {
                InputEvent::PressEnter { .. } => {
                    let (recent, found) = this.visible(cx);
                    if let Some(first) = recent.into_iter().chain(found).next() {
                        let pick = this.pick.clone();
                        cx.defer(move |cx| {
                            if let Some(window) = cx.active_window() {
                                let _ = window.update(cx, |_, window, cx| {
                                    window.close_dialog(cx);
                                    pick(first, window, cx);
                                });
                            }
                        });
                    }
                }
                _ => cx.notify(),
            },
        );
        OpenRepoForm {
            input: input.clone(),
            recent: settings.recent.clone(),
            found: Vec::new(),
            pick: pick.clone(),
            _input: sub,
        }
    });
    // Scanning touches the disk: done in the background.
    let folders = settings.repo_folders.clone();
    let scan = cx.background_spawn(async move { find_repos(&folders) });
    let weak = form.downgrade();
    cx.spawn(async move |cx| {
        let found = scan.await;
        let _ = weak.update(cx, |f, cx| {
            f.found = found;
            cx.notify();
        });
    })
    .detach();

    let browse = Rc::new(browse);
    let focus_once = std::cell::Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if focus_once.replace(false) {
            input.focus_handle(cx).focus(window, cx);
        }
        let browse = browse.clone();
        dialog
            .bg(crate::theme::sheet())
            .title(tr("Ouvrir un dépôt", "Open Repository"))
            .w(px(620.))
            .child(form.clone())
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("open-browse")
                            .outline()
                            .small()
                            .icon(Icon::new(Lucide::FolderOpen))
                            .label(tr("Parcourir…", "Browse…"))
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                browse(window, cx);
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        DialogClose::new().trigger(|b| b.ghost().label(tr("Annuler", "Cancel"))),
                    ),
            )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_repositories_two_levels_deep() {
        let root = tempfile::tempdir().unwrap();
        let mk = |rel: &str| {
            let p = root.path().join(rel);
            std::fs::create_dir_all(p.join(".git")).unwrap();
            p
        };
        let a = mk("alpha");
        let b = mk("work/Beta");
        std::fs::create_dir_all(root.path().join("work/plain")).unwrap();
        mk("work/deep/too/far");
        mk(".hidden");
        let found = find_repos(&[root.path().to_path_buf()]);
        assert_eq!(found, [a.clone(), b]);
        assert_eq!(
            find_repos(std::slice::from_ref(&a)),
            vec![a.clone()],
            "a repo folder itself"
        );
        assert!(matches(&a, "ALP"));
        assert!(!matches(&a, "beta"));
    }
}
