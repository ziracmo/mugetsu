//! Rebasing over uncommitted changes: asks whether to stash them and reapply
//! them after, or to discard them, instead of failing.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::{App, Context, ParentElement as _, Styled as _, Window, div, px};
use mugetsu_git::LocalChanges;

use super::RepoView;
use crate::i18n::tr;
use crate::theme;

type Then = Box<dyn FnOnce(&mut RepoView, LocalChanges, &mut Window, &mut Context<RepoView>)>;

impl RepoView {
    /// Runs `then` right away (with [`LocalChanges::Keep`]) when no tracked
    /// file is changed, otherwise once the user picked stash or discard.
    pub(super) fn with_local_changes(
        &mut self,
        then: impl FnOnce(&mut Self, LocalChanges, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = self.data.tracked_changes();
        if n == 0 {
            then(self, LocalChanges::Keep, window, cx);
            return;
        }
        let view = cx.entity().downgrade();
        let then = Rc::new(Cell::new(Some(Box::new(then) as Then)));
        window.open_dialog(cx, move |dialog, _, _| {
            let run = {
                let (view, then) = (view.clone(), then.clone());
                // Deferred past this dialog's closing, so `then` may open
                // another one.
                move |changes, window: &mut Window, cx: &mut App| {
                    let (view, then) = (view.clone(), then.take());
                    window.defer(cx, move |window, cx| {
                        if let Some(then) = then {
                            let _ = view.update(cx, |this, cx| then(this, changes, window, cx));
                        }
                    });
                }
            };
            let discard = run.clone();
            dialog
                .bg(theme::sheet())
                .title(tr("Changements non commités", "Uncommitted changes"))
                .w(px(480.))
                .child(div().text_sm().text_color(theme::muted()).child(trf!(
                    "{n} fichier(s) modifié(s). Les mettre de côté (stash) et les réappliquer \
                     après le rebase est recommandé ; les supprimer est définitif.",
                    "{n} changed file(s). Stashing them and reapplying them after the rebase \
                     is recommended; discarding them can't be undone."
                )))
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new()
                                .trigger(|b| b.ghost().label(tr("Annuler", "Cancel"))),
                        )
                        .child(
                            Button::new("rebase-changes-discard")
                                .danger()
                                .label(tr("Supprimer", "Discard"))
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    discard(LocalChanges::Discard, window, cx);
                                }),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("rebase-changes-stash")
                                    .primary()
                                    .label(tr("Stash et réappliquer", "Stash and reapply")),
                            ),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    run(LocalChanges::Stash, window, cx);
                    true
                })
        });
    }

    /// Rebases HEAD onto `rev`, asking first about uncommitted changes.
    pub(super) fn rebase_head_onto(
        &mut self,
        rev: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_local_changes(
            move |this, changes, window, cx| {
                this.run_op_then(
                    "Rebase",
                    move |r| {
                        r.rebase(&rev, changes)
                            .and_then(super::conflicts::stopped_on_conflicts)
                    },
                    |this, _, _| this.rebased(),
                    window,
                    cx,
                )
            },
            window,
            cx,
        );
    }
}
