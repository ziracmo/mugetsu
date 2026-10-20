//! Side panel of a commit's details: the changed files as a tree, each with
//! an icon for its kind of change. A click scrolls the diff to the file, a
//! click on a folder folds or unfolds it; scrolling the diff lights up the
//! file being read.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::diff::DiffFileStatus;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
    px,
};

use std::collections::HashSet;

use super::RepoView;
use crate::theme;

/// One line of the tree: a folder (with its full path), or a file (its index
/// in the diff).
#[derive(Debug, PartialEq, Eq)]
pub enum TreeRow {
    Dir {
        depth: usize,
        name: String,
        /// Full folder path, e.g. `src/ui`.
        path: String,
    },
    File {
        depth: usize,
        name: String,
        index: usize,
    },
}

/// Lays out `paths` as a tree, folders first at each level, sorted by name.
pub fn tree(paths: &[&str]) -> Vec<TreeRow> {
    let mut order: Vec<usize> = (0..paths.len()).collect();
    // Sorting by (dirs…, then a file marker) puts folders before files.
    let key = |p: &str| {
        let mut parts: Vec<String> = p.split('/').map(|s| format!("0{s}")).collect();
        if let Some(last) = parts.last_mut() {
            last.replace_range(0..1, "1");
        }
        parts
    };
    order.sort_by_key(|&i| key(paths[i]));
    let mut rows = Vec::new();
    let mut open: Vec<&str> = Vec::new();
    for i in order {
        let parts: Vec<&str> = paths[i].split('/').collect();
        let (name, dirs) = parts.split_last().unwrap_or((&"", &[]));
        let common = open.iter().zip(dirs).take_while(|(a, b)| a == b).count();
        open.truncate(common);
        for (depth, dir) in dirs.iter().enumerate().skip(common) {
            rows.push(TreeRow::Dir {
                depth,
                name: dir.to_string(),
                path: dirs[..=depth].join("/"),
            });
            open.push(dir);
        }
        rows.push(TreeRow::File {
            depth: dirs.len(),
            name: name.to_string(),
            index: i,
        });
    }
    rows
}

/// Drops the rows inside folded folders (the folders themselves stay).
pub fn unfolded(rows: Vec<TreeRow>, folded: &HashSet<String>) -> Vec<TreeRow> {
    let mut hidden_below: Option<usize> = None;
    rows.into_iter()
        .filter(|row| {
            let depth = match row {
                TreeRow::Dir { depth, .. } | TreeRow::File { depth, .. } => *depth,
            };
            if hidden_below.is_some_and(|d| depth > d) {
                return false;
            }
            hidden_below = match row {
                TreeRow::Dir { path, .. } if folded.contains(path) => Some(depth),
                _ => None,
            };
            true
        })
        .collect()
}

pub fn status_look(status: DiffFileStatus) -> (Lucide, Hsla) {
    match status {
        DiffFileStatus::Added => (Lucide::FilePlus, theme::lime()),
        DiffFileStatus::Deleted => (Lucide::FileMinus, theme::red()),
        DiffFileStatus::Renamed | DiffFileStatus::Copied => {
            (Lucide::ArrowRightLeft, theme::violet())
        }
        DiffFileStatus::Modified => (Lucide::FilePen, theme::amber()),
        _ => (Lucide::FileDiff, theme::muted()),
    }
}

impl RepoView {
    /// Scrolls the file tree to `path`, the file read in the diff.
    pub(super) fn reveal_commit_file(&self, path: Option<&str>, cx: &App) {
        let Some(path) = path else {
            return;
        };
        let diff = self.commit_diff.read(cx);
        let paths: Vec<&str> = diff.files().iter().map(|f| f.path().as_ref()).collect();
        // Rows as shown, folded folders hiding their files.
        let row = unfolded(tree(&paths), &self.commit_folded)
            .iter()
            .position(|r| matches!(r, TreeRow::File { index, .. } if paths[*index] == path));
        if let Some(row) = row {
            self.commit_files_scroll.scroll_to_item(row);
        }
    }

    pub(super) fn render_commit_files(&self, cx: &mut Context<Self>) -> AnyElement {
        let diff = self.commit_diff.read(cx);
        let files: Vec<(SharedString, DiffFileStatus, Option<SharedString>)> = diff
            .files()
            .iter()
            .map(|f| (f.path().clone(), f.status(), f.original_path().cloned()))
            .collect();
        let paths: Vec<&str> = files.iter().map(|f| f.0.as_ref()).collect();
        // The file read in the diff (scroll spy), else the one clicked.
        let active = diff.active_file().or(self.commit_file.as_ref()).cloned();
        let (view, root, rev) = (
            cx.entity().downgrade(),
            self.repo.root().to_path_buf(),
            self.selected,
        );
        let rows = unfolded(tree(&paths), &self.commit_folded);
        let rows = rows.into_iter().map(|row| {
            let base = h_flex()
                .h(px(22.))
                .gap_1p5()
                .pr_2()
                .rounded_md()
                .text_xs()
                .whitespace_nowrap();
            match row {
                TreeRow::Dir { depth, name, path } => {
                    let folded = self.commit_folded.contains(&path);
                    base.id(SharedString::from(format!("commit-dir-{path}")))
                        .test_support()
                        .pl(px(8. + depth as f32 * 12.))
                        .cursor_pointer()
                        .text_color(theme::muted())
                        .hover(|s| s.bg(theme::raised()))
                        .child(
                            Icon::new(if folded {
                                Lucide::ChevronRight
                            } else {
                                Lucide::ChevronDown
                            })
                            .xsmall(),
                        )
                        .child(
                            Icon::new(if folded {
                                Lucide::Folder
                            } else {
                                Lucide::FolderOpen
                            })
                            .xsmall(),
                        )
                        .child(name)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.commit_folded.remove(&path) {
                                this.commit_folded.insert(path.clone());
                            }
                            cx.notify();
                        }))
                        .into_any_element()
                }
                TreeRow::File { depth, name, index } => {
                    let (path, status, original) = files[index].clone();
                    let (icon, color) = status_look(status);
                    let active = active.as_ref() == Some(&path);
                    let tip = match original.filter(|o| *o != path) {
                        Some(from) => format!("{from} → {path}"),
                        None => path.to_string(),
                    };
                    let menu = super::menus::FileMenu {
                        path: path.to_string(),
                        rev,
                        blamable: status != DiffFileStatus::Deleted,
                    };
                    base.id(SharedString::from(format!("commit-file-{path}")))
                        .test_support()
                        .pl(px(8. + depth as f32 * 12.))
                        .cursor_pointer()
                        .when(active, |s| s.bg(theme::alpha(theme::accent(), 0.22)))
                        .when(!active, |s| s.hover(|s| s.bg(theme::raised())))
                        .child(Icon::new(icon).xsmall().text_color(color))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .when(status == DiffFileStatus::Deleted, |s| s.line_through())
                                .child(name),
                        )
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(tip.clone())
                                .build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.commit_file = Some(path.clone());
                            this.commit_diff
                                .update(cx, |d, cx| d.scroll_to_file(&path, cx));
                            cx.notify();
                        }))
                        .context_menu({
                            let (view, root) = (view.clone(), root.clone());
                            move |m, _, _| super::menus::file_menu(m, &view, &root, &menu)
                        })
                        .into_any_element()
                }
            }
        });
        let side = self.details_side;
        v_flex()
            .id("commit-files")
            .flex_none()
            .p_1()
            .overflow_y_scroll()
            .track_scroll(&self.commit_files_scroll)
            .map(|s| {
                if side {
                    s.w_full().max_h(px(180.)).border_b_1()
                } else {
                    s.w(px(240.)).h_full().border_r_1()
                }
            })
            .border_color(cx.theme().border)
            .bg(theme::panel())
            .children(rows)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_tree_with_folders_first() {
        let rows = tree(&["src/b.rs", "README.md", "src/ui/a.rs", "src/a.rs", "z/y.rs"]);
        let shown: Vec<String> = rows
            .iter()
            .map(|r| match r {
                TreeRow::Dir { depth, name, .. } => format!("{depth}{name}/"),
                TreeRow::File { depth, name, .. } => format!("{depth}{name}"),
            })
            .collect();
        assert_eq!(
            shown,
            [
                "0src/",
                "1ui/",
                "2a.rs",
                "1a.rs",
                "1b.rs",
                "0z/",
                "1y.rs",
                "0README.md"
            ]
        );
        assert!(matches!(rows[2], TreeRow::File { index: 2, .. }));
        assert!(matches!(&rows[1], TreeRow::Dir { path, .. } if path == "src/ui"));
    }

    #[test]
    fn folded_folders_hide_their_contents() {
        let paths = ["src/b.rs", "README.md", "src/ui/a.rs", "src/a.rs", "z/y.rs"];
        let shown = |folded: &[&str]| -> Vec<String> {
            let folded = folded.iter().map(|s| s.to_string()).collect();
            unfolded(tree(&paths), &folded)
                .iter()
                .map(|r| match r {
                    TreeRow::Dir { name, .. } => format!("{name}/"),
                    TreeRow::File { name, .. } => name.clone(),
                })
                .collect()
        };
        assert_eq!(
            shown(&["src/ui"]),
            ["src/", "ui/", "a.rs", "b.rs", "z/", "y.rs", "README.md"]
        );
        assert_eq!(shown(&["src"]), ["src/", "z/", "y.rs", "README.md"]);
        assert_eq!(shown(&["src", "z"]), ["src/", "z/", "README.md"]);
    }
}
