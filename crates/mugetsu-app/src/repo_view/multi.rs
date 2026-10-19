//! Several local branches picked at once (⌘/⌃-click in the sidebar or the
//! history, ⇧-click for a range of commits), then deleted or rebased together.

use std::rc::Rc;

use gix::ObjectId;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, WindowExt as _, h_flex};
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, ClipboardItem, Context, InteractiveElement as _,
    IntoElement, ParentElement as _, Styled as _, WeakEntity, Window, div, px,
};
use mugetsu_git::RefLabel;

use super::RepoView;
use crate::i18n::tr;
use crate::theme;

/// ⌘ on macOS, ⌃ elsewhere (either is accepted everywhere).
pub(super) fn toggles(ev: &ClickEvent) -> bool {
    let m = ev.modifiers();
    m.platform || m.control
}

pub(super) fn extends(ev: &ClickEvent) -> bool {
    ev.modifiers().shift
}

impl RepoView {
    /// The picked branches, in the order the repository lists them.
    pub fn picked_branches(&self) -> Vec<String> {
        self.data
            .refs
            .branches
            .iter()
            .filter(|b| self.picked.contains(&b.name))
            .map(|b| b.name.clone())
            .collect()
    }

    pub(super) fn is_picked(&self, branch: &str) -> bool {
        self.picked.contains(branch)
    }

    /// A commit row is picked when one of its local branches is.
    pub(super) fn commit_picked(&self, id: &ObjectId) -> bool {
        !self.picked.is_empty() && self.local_branches_at(id).any(|b| self.picked.contains(b))
    }

    fn local_branches_at<'a>(&'a self, id: &ObjectId) -> impl Iterator<Item = &'a String> {
        self.data
            .decorations
            .get(id)
            .into_iter()
            .flatten()
            .filter_map(|l| match l {
                RefLabel::Branch { name, .. } => Some(name),
                _ => None,
            })
    }

    pub(super) fn clear_picks(&mut self, cx: &mut Context<Self>) {
        if !self.picked.is_empty() {
            self.picked.clear();
            cx.notify();
        }
        self.pick_anchor = None;
    }

    /// The branch selected with a plain click joins the first ⌘-click.
    fn seed_picks(&mut self) {
        if self.picked.is_empty()
            && let Some(name) = self
                .selected_ref
                .as_ref()
                .and_then(|r| r.strip_prefix("branch:"))
        {
            self.picked.insert(name.to_string());
        }
    }

    /// In the history: the selected commit's branches join the first ⌘-click.
    fn seed_picks_from_commit(&mut self) {
        if self.picked.is_empty()
            && let Some(id) = self.selected
        {
            let names: Vec<String> = self.local_branches_at(&id).cloned().collect();
            self.picked.extend(names);
        }
    }

    /// ⌘/⌃-click on a sidebar branch.
    pub(super) fn toggle_branch_pick(&mut self, name: &str, cx: &mut Context<Self>) {
        self.seed_picks();
        if !self.picked.remove(name) {
            self.picked.insert(name.to_string());
        }
        self.selected_ref = None;
        cx.notify();
    }

    /// ⌘/⌃-click on a history row: its local branches go in or out together.
    pub(super) fn toggle_commit_pick(&mut self, id: ObjectId, cx: &mut Context<Self>) {
        let names: Vec<String> = self.local_branches_at(&id).cloned().collect();
        self.pick_anchor = Some(id);
        if names.is_empty() {
            return;
        }
        self.seed_picks_from_commit();
        if names.iter().all(|n| self.picked.contains(n)) {
            for n in &names {
                self.picked.remove(n);
            }
        } else {
            self.picked.extend(names);
        }
        self.selected_ref = None;
        cx.notify();
    }

    /// ⇧-click on a history row: every local branch between the last clicked
    /// row (or the selected commit) and this one.
    pub(super) fn pick_commit_range(&mut self, id: ObjectId, cx: &mut Context<Self>) {
        let history = &self.data.history;
        let Some(to) = history.row_of(&id) else {
            return;
        };
        let from = self
            .pick_anchor
            .or(self.selected)
            .and_then(|a| history.row_of(&a))
            .unwrap_or(to);
        let (lo, hi) = (from.min(to), from.max(to));
        let ids: Vec<ObjectId> = history.commits[lo..=hi].iter().map(|c| c.id).collect();
        let names: Vec<String> = ids
            .iter()
            .flat_map(|id| self.local_branches_at(id).cloned().collect::<Vec<_>>())
            .collect();
        self.picked.extend(names);
        self.selected_ref = None;
        cx.notify();
    }

    /// Picked branches that no longer exist are dropped after a reload.
    pub(super) fn prune_picks(&mut self) {
        let refs = &self.data.refs;
        self.picked
            .retain(|n| refs.branches.iter().any(|b| &b.name == n));
    }

    /// "N branches picked" bar over the history, with the batch actions.
    pub(super) fn render_picks_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let n = self.picked.len();
        (n > 0).then(|| {
            h_flex()
                .id("picks-bar")
                .test_support()
                .h(px(30.))
                .flex_none()
                .px_3()
                .gap_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(theme::alpha(theme::accent(), 0.08))
                .child(
                    Icon::new(Lucide::GitBranch)
                        .xsmall()
                        .text_color(theme::accent()),
                )
                .child(div().text_sm().text_color(theme::accent()).child(trf!(
                    "{n} branche(s) sélectionnée(s)",
                    "{n} branch(es) selected"
                )))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("⌘/⌃-clic pour en ajouter", "⌘/⌃-click to add more")),
                )
                .child(div().flex_1())
                .child(
                    Button::new("picks-rebase")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::GitBranch))
                        .label(tr("Rebaser sur…", "Rebase onto…"))
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.prompt_rebase_picked(window, cx)
                            }),
                        ),
                )
                .child(
                    Button::new("picks-delete")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::Trash))
                        .label(tr("Supprimer…", "Delete…"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.confirm_delete_picked(window, cx)
                        })),
                )
                .child(
                    Button::new("picks-clear")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(Lucide::X))
                        .on_click(cx.listener(|this, _, _, cx| this.clear_picks(cx))),
                )
                .into_any_element()
        })
    }

    /// Asks for the branch to rebase the picked ones onto.
    pub fn prompt_rebase_picked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = self.picked_branches();
        let n = picked.len();
        if n == 0 {
            return;
        }
        let initial = self
            .data
            .refs
            .head_branch
            .clone()
            .filter(|h| !picked.contains(h))
            .unwrap_or_default();
        // Asked first, so the prompt's answer runs the rebase straight away.
        self.with_local_changes(
            move |this, changes, window, cx| {
                this.prompt(
                    trf!("Rebaser {n} branche(s) sur…", "Rebase {n} branch(es) onto…"),
                    tr("Branche ou révision cible", "Target branch or revision"),
                    initial,
                    tr("Rebaser", "Rebase"),
                    move |this, onto, window, cx| {
                        let names: Vec<String> =
                            picked.iter().filter(|b| **b != onto).cloned().collect();
                        this.rebase_picked(onto, names, changes, window, cx);
                    },
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub fn rebase_picked(
        &mut self,
        onto: String,
        names: Vec<String>,
        changes: mugetsu_git::LocalChanges,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if names.is_empty() {
            return;
        }
        self.run_op_then(
            "Rebase",
            move |r| r.rebase_branches(&onto, &names, changes),
            |this, _, cx| this.clear_picks(cx),
            window,
            cx,
        );
    }

    /// Confirms, then deletes the picked branches (the checked-out one stays).
    pub fn confirm_delete_picked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let head = self.data.refs.head_branch.clone();
        let names: Vec<String> = self
            .picked_branches()
            .into_iter()
            .filter(|b| Some(b) != head.as_ref())
            .collect();
        let n = names.len();
        if n == 0 {
            return;
        }
        let list = names.join(", ");
        self.confirm(
            trf!("Supprimer {n} branche(s) ?", "Delete {n} branch(es)?"),
            trf!(
                "{list}\n\nLes branches locales sont supprimées. Git refuse celles qui ne sont pas mergées : vous pourrez forcer ensuite.",
                "{list}\n\nThe local branches are deleted. Git refuses the unmerged ones: you can force them afterwards."
            ),
            tr("Supprimer", "Delete"),
            move |this, window, cx| this.delete_branches(names.clone(), false, window, cx),
            window,
            cx,
        );
    }

    /// `git branch -d` (or `-D`) on each; refusals end in one toast, with a
    /// button to force the unmerged ones.
    pub fn delete_branches(
        &mut self,
        names: Vec<String>,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() || names.is_empty() {
            return;
        }
        self.busy = Some("Delete");
        let repo = self.repo.clone();
        self._op = cx.spawn_in(window, async move |this, cx| {
            let out = cx
                .background_spawn(async move { repo.delete_branches(&names, force) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = None;
                for b in &out.deleted {
                    this.picked.remove(b);
                }
                if !out.failed.is_empty() {
                    let msg = out
                        .failed
                        .iter()
                        .map(|(b, e)| format!("{b}: {e}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let action = (!out.unmerged.is_empty()).then(|| {
                        let view = cx.entity().downgrade();
                        let unmerged = out.unmerged.clone();
                        let action: crate::toast::Action = (
                            tr("Forcer la suppression…", "Force delete…").into(),
                            Rc::new(move |window, cx| {
                                let unmerged = unmerged.clone();
                                let _ = view.update(cx, |this, cx| {
                                    this.confirm_force_delete_many(unmerged, window, cx)
                                });
                            }),
                        );
                        action
                    });
                    let n = out.failed.len();
                    window.push_notification(
                        crate::toast::error(
                            trf!(
                                "{n} branche(s) non supprimée(s)",
                                "{n} branch(es) not deleted"
                            ),
                            &anyhow::anyhow!(msg),
                            action,
                        ),
                        cx,
                    );
                }
                this.reload(cx);
            });
        });
        cx.notify();
    }

    fn confirm_force_delete_many(
        &mut self,
        names: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = names.len();
        let list = names.join(", ");
        self.confirm(
            trf!(
                "Forcer la suppression de {n} branche(s) ?",
                "Force delete {n} branch(es)?"
            ),
            trf!(
                "{list}\n\nCes branches ne sont pas mergées : leurs commits qui ne sont sur aucune autre branche seront perdus.",
                "{list}\n\nThese branches aren't merged: their commits that are on no other branch will be lost."
            ),
            tr("Forcer la suppression", "Force delete"),
            move |this, window, cx| this.delete_branches(names.clone(), true, window, cx),
            window,
            cx,
        );
    }
}

/// Right-click on a picked branch (sidebar row or history row) while
/// several are picked: the batch actions.
pub(super) fn picks_menu(
    menu: PopupMenu,
    view: &WeakEntity<RepoView>,
    names: &[String],
) -> PopupMenu {
    let n = names.len();
    let on = |f: fn(&mut RepoView, &mut Window, &mut Context<RepoView>)| {
        let view = view.clone();
        move |_: &ClickEvent, window: &mut Window, cx: &mut gpui_kit::App| {
            let _ = view.update(cx, |this, cx| f(this, window, cx));
        }
    };
    let text = names.join("\n");
    menu.label(trf!("{n} branches sélectionnées", "{n} branches selected"))
        .item(
            PopupMenuItem::new(trf!(
                "Rebaser {n} branches sur…",
                "Rebase {n} branches onto…"
            ))
            .icon(Icon::new(Lucide::GitBranch))
            .on_click(on(|this, window, cx| this.prompt_rebase_picked(window, cx))),
        )
        .item(
            PopupMenuItem::new(trf!("Supprimer {n} branches…", "Delete {n} branches…"))
                .icon(Icon::new(Lucide::Trash))
                .on_click(on(|this, window, cx| {
                    this.confirm_delete_picked(window, cx)
                })),
        )
        .separator()
        .item(
            PopupMenuItem::new(tr("Copier les noms", "Copy names"))
                .icon(Icon::new(Lucide::Copy))
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                }),
        )
        .separator()
        .item(
            PopupMenuItem::new(tr("Désélectionner", "Clear selection"))
                .icon(Icon::new(Lucide::X))
                .on_click(on(|this, _, cx| this.clear_picks(cx))),
        )
}
