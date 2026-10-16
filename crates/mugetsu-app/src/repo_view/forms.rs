//! Sheets: checkout (track a remote branch, handle local changes)
//! and push (target branch, tags, force-with-lease).

use std::sync::{Arc, LazyLock};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AppContext as _, Context, Entity, Focusable as _, Image,
    ImageFormat, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window, div, img, prelude::FluentBuilder as _,
    px,
};
use mugetsu_git::{LocalChanges, PushOptions};

use super::RepoView;
use super::sidebar::default_remote;
use crate::i18n::tr;
use crate::theme;

/// Send-off played on the reset sheet before it closes.
const RESET_SENDOFF: std::time::Duration = std::time::Duration::from_millis(280);

pub(crate) static APP_ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../assets/icon/mugetsu.png").to_vec(),
    ))
});

/// Icon, title and subtitle at the top of a sheet.
fn header(title: &'static str, subtitle: &'static str) -> impl IntoElement {
    h_flex()
        .gap_4()
        .items_start()
        .child(img(APP_ICON.clone()).flex_none().size(px(56.)))
        .child(
            v_flex()
                .gap_0p5()
                .pt_1()
                .child(
                    div()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(div().text_sm().text_color(theme::muted()).child(subtitle)),
        )
}

/// "Label:  value" row with right-aligned labels.
fn field(label: &'static str, value: impl IntoElement) -> impl IntoElement {
    h_flex()
        .gap_3()
        .items_start()
        .child(
            div()
                .w(px(130.))
                .flex_none()
                .pt(px(3.))
                .text_right()
                .text_sm()
                .text_color(theme::muted())
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(value))
}

fn branch_text(name: String) -> impl IntoElement {
    h_flex()
        .gap_1p5()
        .pt(px(3.))
        .text_sm()
        .child(
            Icon::new(Lucide::GitBranch)
                .xsmall()
                .text_color(theme::muted()),
        )
        .child(name)
}

/// Checkbox row (whole row clickable).
fn toggle(
    id: &'static str,
    label: &'static str,
    checked: bool,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
) -> impl IntoElement {
    h_flex()
        .id(id)
        .test_support()
        .gap_2()
        .cursor_pointer()
        .text_sm()
        .child(
            div()
                .size(px(14.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .map(|s| {
                    if checked {
                        s.bg(theme::accent()).border_color(theme::accent()).child(
                            Icon::new(Lucide::Check)
                                .xsmall()
                                .text_color(gpui_kit::white()),
                        )
                    } else {
                        s.border_color(theme::alpha(gpui_kit::white(), 0.25))
                            .bg(theme::alpha(gpui_kit::white(), 0.04))
                    }
                }),
        )
        .child(label)
        .on_click(on_click)
}

fn footer(ok_id: &'static str, ok: &'static str) -> DialogFooter {
    DialogFooter::new()
        .child(DialogClose::new().trigger(|b| b.ghost().label(tr("Annuler", "Cancel"))))
        .child(DialogAction::new().child(Button::new(ok_id).primary().label(ok)))
}

pub struct CheckoutForm {
    /// `origin/x` when checking out a remote branch.
    remote: Option<String>,
    /// Local branch to switch to.
    local: String,
    /// Name field, when the local branch has to be created.
    local_input: Option<Entity<InputState>>,
    dirty: bool,
    changes: LocalChanges,
}

impl CheckoutForm {
    fn local_name(&self, cx: &gpui_kit::App) -> String {
        match &self.local_input {
            Some(input) => input.read(cx).value().trim().to_string(),
            None => self.local.clone(),
        }
    }
}

impl Render for CheckoutForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let options = [
            (LocalChanges::Keep, tr("Ne rien changer", "Don't change")),
            (
                LocalChanges::Stash,
                tr("Stash et réappliquer", "Stash and reapply"),
            ),
            (
                LocalChanges::Discard,
                tr("Annuler les changements", "Discard"),
            ),
        ];
        let selected = options.iter().position(|(c, _)| *c == self.changes);
        let mut body = v_flex().gap_2();
        if let Some(remote) = &self.remote {
            body = body.child(field(
                tr("Branche distante :", "Remote branch:"),
                branch_text(remote.clone()),
            ));
        }
        body = body.child(field(
            tr("Branche locale :", "Local branch:"),
            match &self.local_input {
                Some(input) => Input::new(input)
                    .id("checkout-local")
                    .small()
                    .prefix(
                        Icon::new(Lucide::GitBranch)
                            .xsmall()
                            .text_color(theme::muted()),
                    )
                    .into_any_element(),
                None => branch_text(self.local.clone()).into_any_element(),
            },
        ));
        if self.dirty {
            body = body.child(field(
                tr("Changements locaux :", "Local changes:"),
                RadioGroup::vertical("checkout-changes")
                    .children(
                        options
                            .iter()
                            .map(|(c, label)| Radio::new(format!("changes-{c:?}")).label(*label)),
                    )
                    .selected_index(selected)
                    .on_click(cx.listener(move |this, ix: &usize, _, cx| {
                        this.changes = options[*ix].0;
                        cx.notify();
                    })),
            ));
        }
        let (title, subtitle) = if self.remote.is_some() {
            (
                tr("Suivre la branche distante", "Track Remote Branch"),
                tr(
                    "Crée une branche locale qui suit la branche distante",
                    "Creates a new local branch which tracks the remote branch",
                ),
            )
        } else {
            (
                tr("Checkout", "Checkout"),
                tr(
                    "La copie de travail contient des changements non commités",
                    "The working copy has uncommitted changes",
                ),
            )
        };
        v_flex().gap_4().child(header(title, subtitle)).child(body)
    }
}

pub struct PushForm {
    /// Local branches with their upstream.
    branches: Vec<(String, Option<String>)>,
    branch: usize,
    remotes: Vec<String>,
    remote: usize,
    tags: bool,
    force: bool,
}

impl PushForm {
    /// Remote branch name: the upstream's on that remote, else the local name.
    fn target(&self) -> (String, String, bool) {
        let (name, upstream) = &self.branches[self.branch];
        let remote = self.remotes[self.remote].clone();
        let tracked = upstream
            .as_deref()
            .and_then(|u| u.strip_prefix(&format!("{remote}/")))
            .map(str::to_string);
        let is_default = tracked.is_some();
        (remote, tracked.unwrap_or_else(|| name.clone()), is_default)
    }

    fn target_label(&self) -> String {
        let (remote, branch, is_default) = self.target();
        if is_default {
            format!("{} ({remote}/{branch})", tr("défaut", "default"))
        } else {
            format!("{remote}/{branch}")
        }
    }
}

impl Render for PushForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let form = cx.entity().downgrade();
        let names: Vec<String> = self.branches.iter().map(|(n, _)| n.clone()).collect();
        let current = self.branch;
        let branch_picker = Button::new("push-branch")
            .outline()
            .small()
            .w_full()
            .icon(Icon::new(Lucide::GitBranch))
            .label(names[current].clone())
            .dropdown_menu({
                let form = form.clone();
                move |mut menu, _, _| {
                    for (i, name) in names.iter().enumerate() {
                        let form = form.clone();
                        menu = menu.item(
                            PopupMenuItem::new(name.clone())
                                .checked(i == current)
                                .on_click(move |_, _, cx| {
                                    let _ = form.update(cx, |f, cx| {
                                        f.branch = i;
                                        cx.notify();
                                    });
                                }),
                        );
                    }
                    menu
                }
            });
        let remotes = self.remotes.clone();
        let remote_current = self.remote;
        let remote_picker = Button::new("push-remote")
            .outline()
            .small()
            .w_full()
            .icon(Icon::new(Lucide::Cloud))
            .label(self.target_label())
            .dropdown_menu(move |mut menu, _, _| {
                for (i, name) in remotes.iter().enumerate() {
                    let form = form.clone();
                    menu = menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(i == remote_current)
                            .on_click(move |_, _, cx| {
                                let _ = form.update(cx, |f, cx| {
                                    f.remote = i;
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            });
        v_flex()
            .gap_4()
            .child(header(
                tr("Push", "Push"),
                tr(
                    "Envoie tes commits locaux sur le dépôt distant",
                    "Push your local changes to the remote repository",
                ),
            ))
            .child(
                v_flex()
                    .gap_2()
                    .child(field(tr("Branche :", "Branch:"), branch_picker))
                    .child(field(tr("Vers :", "To:"), remote_picker))
                    .child(field(
                        "",
                        v_flex()
                            .gap_1p5()
                            .pt_1()
                            .child(toggle(
                                "push-tags",
                                tr("Pousser tous les tags", "Push all tags"),
                                self.tags,
                                cx.listener(|this, _, _, cx| {
                                    this.tags = !this.tags;
                                    cx.notify();
                                }),
                            ))
                            .child(toggle(
                                "push-force",
                                tr(
                                    "Force push (--force-with-lease)",
                                    "Force push (--force-with-lease)",
                                ),
                                self.force,
                                cx.listener(|this, _, _, cx| {
                                    this.force = !this.force;
                                    cx.notify();
                                }),
                            )),
                    )),
            )
    }
}

impl RepoView {
    /// Checks out `name` (a local branch, or `remote/branch`). Goes straight
    /// through when nothing needs deciding, otherwise asks.
    pub fn request_checkout(
        &mut self,
        name: String,
        is_remote: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty = self.data.uncommitted() > 0;
        let (local, remote) = if is_remote {
            let local = name
                .split_once('/')
                .map_or(name.clone(), |(_, b)| b.to_string());
            (local, Some(name))
        } else {
            (name, None)
        };
        // A local branch checked out in another worktree: open that worktree.
        if !is_remote && let Some(path) = self.other_worktrees().remove(&local) {
            cx.emit(super::RepoEvent::OpenWorktree(path));
            return;
        }
        let exists = self.data.refs.branches.iter().any(|b| b.name == local);
        // Double-click on the upstream of the checked-out branch: offer to
        // align the branch on it instead.
        if let Some(upstream) = remote.as_deref()
            && let Some(head) = self
                .data
                .refs
                .branches
                .iter()
                .find(|b| b.is_head && b.upstream.as_deref() == Some(upstream))
        {
            let (name, upstream) = (head.name.clone(), upstream.to_string());
            if head.ahead == 0 && head.behind == 0 && !dirty {
                return;
            }
            self.confirm_reset(name, upstream, true, None, window, cx);
            return;
        }
        // Double-click on a remote branch whose local counterpart exists and
        // points elsewhere: offer to reset the local branch on it.
        if let Some(upstream) = remote.as_deref()
            && let Some(b) = self.data.refs.branches.iter().find(|b| b.name == local)
            && !self.other_worktrees().contains_key(&local)
        {
            let target = (self.data.refs.remote_branches.iter())
                .find(|r| r.name == upstream)
                .map(|r| r.target);
            let aligned = target == Some(b.target);
            let upstream = upstream.to_string();
            if b.is_head {
                if !(aligned && !dirty) {
                    self.confirm_reset(local, upstream, true, None, window, cx);
                }
                return;
            }
            if !aligned {
                let changes = if dirty {
                    LocalChanges::Stash
                } else {
                    LocalChanges::Keep
                };
                self.confirm_reset(local, upstream, false, Some(changes), window, cx);
                return;
            }
        }
        if self.data.refs.head_branch.as_deref() == Some(local.as_str()) && exists {
            return;
        }
        if exists && !dirty {
            self.checkout_branch(local, None, LocalChanges::Keep, window, cx);
            return;
        }
        let local_input = (!exists).then(|| {
            let value = local.clone();
            cx.new(|cx| InputState::new(window, cx).default_value(value))
        });
        let form = cx.new(|_| CheckoutForm {
            remote: if exists { None } else { remote },
            local,
            local_input,
            dirty,
            changes: if dirty {
                LocalChanges::Stash
            } else {
                LocalChanges::Keep
            },
        });
        let view = cx.entity().downgrade();
        let tracking = !exists;
        let focus_once = std::cell::Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if focus_once.replace(false)
                && let Some(input) = &form.read(cx).local_input
            {
                input.focus_handle(cx).focus(window, cx);
            }
            let (form, view) = (form.clone(), view.clone());
            dialog
                .bg(crate::theme::sheet())
                .w(px(520.))
                .child(form.clone())
                .footer(footer(
                    "checkout-ok",
                    if tracking {
                        tr("Suivre", "Track")
                    } else {
                        tr("Checkout", "Checkout")
                    },
                ))
                .on_ok(move |_, window, cx| {
                    let f = form.read(cx);
                    let local = f.local_name(cx);
                    if local.is_empty() {
                        return false;
                    }
                    let (track, changes) = (f.remote.clone(), f.changes);
                    let _ = view.update(cx, |this, cx| {
                        this.checkout_branch(local, track, changes, window, cx)
                    });
                    true
                })
        });
    }

    /// Opens the push sheet, on `branch` or the checked-out one.
    pub fn open_push_dialog(
        &mut self,
        branch: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let refs = &self.data.refs;
        let remotes: Vec<String> = refs.remotes.iter().map(|r| r.name.clone()).collect();
        let branches: Vec<(String, Option<String>)> = refs
            .branches
            .iter()
            .map(|b| {
                (
                    b.name.clone(),
                    b.upstream.clone().filter(|_| !b.upstream_gone),
                )
            })
            .collect();
        let wanted = branch.or_else(|| refs.head_branch.clone());
        let Some(index) = wanted.and_then(|w| branches.iter().position(|(n, _)| *n == w)) else {
            return;
        };
        if remotes.is_empty() {
            window.push_notification(
                gpui_kit::component::notification::Notification::warning(tr(
                    "Aucun remote configuré",
                    "No remote configured",
                )),
                cx,
            );
            return;
        }
        // The upstream's remote, else origin, else the first one.
        let remote = branches[index]
            .1
            .as_deref()
            .and_then(|u| remotes.iter().position(|r| u.starts_with(&format!("{r}/"))))
            .or_else(|| default_remote(refs).and_then(|d| remotes.iter().position(|r| *r == d)))
            .unwrap_or(0);
        let form = cx.new(|_| PushForm {
            branches,
            branch: index,
            remotes,
            remote,
            tags: false,
            force: false,
        });
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let (form, view) = (form.clone(), view.clone());
            dialog
                .bg(crate::theme::sheet())
                .w(px(560.))
                .child(form.clone())
                .footer(footer("push-ok", tr("Push", "Push")))
                .on_ok(move |_, window, cx| {
                    let f = form.read(cx);
                    let (remote, remote_branch, _) = f.target();
                    let local = f.branches[f.branch].0.clone();
                    let options = PushOptions {
                        set_upstream: f.branches[f.branch].1.is_none(),
                        force_with_lease: f.force,
                        tags: f.tags,
                    };
                    let pushed = local.clone();
                    let _ = view.update(cx, |this, cx| {
                        this.run_op_then(
                            "Push",
                            move |r| r.push_to(&remote, &local, &remote_branch, options),
                            move |this, _, cx| this.celebrate_push(pushed, cx),
                            window,
                            cx,
                        )
                    });
                    true
                })
        });
    }
}

/// A ref drawn as a chip in the reset sheet.
fn ref_pill(name: &str, icon: Lucide, color: gpui_kit::Hsla) -> impl IntoElement {
    h_flex()
        .flex_none()
        .max_w(px(190.))
        .gap_1()
        .px_2()
        .h(px(24.))
        .rounded_md()
        .border_1()
        .border_color(theme::alpha(color, 0.5))
        .bg(theme::alpha(color, 0.14))
        .text_sm()
        .text_color(color)
        .child(Icon::new(icon).xsmall())
        .child(div().truncate().child(name.to_string()))
}

/// Loss / gain line of the reset sheet.
fn stat(icon: Lucide, color: gpui_kit::Hsla, text: String) -> impl IntoElement {
    h_flex()
        .gap_2()
        .text_sm()
        .child(Icon::new(icon).xsmall().text_color(color))
        .child(text)
}

impl RepoView {
    /// Centred sheet before resetting `branch` onto its upstream: what is lost,
    /// what comes in, then a danger button.
    pub fn confirm_reset(
        &mut self,
        branch: String,
        upstream: String,
        is_head: bool,
        checkout: Option<LocalChanges>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Counts are only known against the branch's own upstream.
        let counts = self
            .data
            .refs
            .branches
            .iter()
            .find(|b| b.name == branch)
            .filter(|b| b.upstream.as_deref() == Some(upstream.as_str()))
            .map(|b| (b.ahead, b.behind));
        let (ahead, behind) = counts.unwrap_or((0, 0));
        let dirty = if is_head { self.data.uncommitted() } else { 0 };
        let view = cx.entity().downgrade();
        // Set once "Reset" is pressed: the sheet plays its send-off, then closes.
        let launched = std::rc::Rc::new(std::cell::Cell::new(false));
        window.open_dialog(cx, move |dialog, window, _| {
            let going = launched.get();
            let launched = launched.clone();
            // Centred vertically (the sheet is about 330px tall).
            let top = ((window.viewport_size().height - px(330.)) / 2.).max(px(40.));
            let (name, target, view) = (branch.clone(), upstream.clone(), view.clone());
            let mut stats = v_flex().gap_1p5();
            if ahead > 0 {
                stats = stats.child(stat(
                    Lucide::CircleX,
                    theme::red(),
                    trf!(
                        "{ahead} commit(s) local(aux) non poussé(s) perdu(s)",
                        "{ahead} unpushed local commit(s) lost"
                    ),
                ));
            }
            if behind > 0 {
                stats = stats.child(stat(
                    Lucide::CircleCheck,
                    theme::lime(),
                    trf!(
                        "{behind} commit(s) du remote récupéré(s)",
                        "{behind} remote commit(s) brought in"
                    ),
                ));
            }
            if dirty > 0 {
                stats = stats.child(stat(
                    Lucide::CircleAlert,
                    theme::amber(),
                    trf!(
                        "{dirty} fichier(s) modifié(s) non commité(s) perdu(s)",
                        "{dirty} uncommitted changed file(s) lost"
                    ),
                ));
            }
            if counts.is_some() && ahead == 0 && behind == 0 && dirty == 0 {
                stats = stats.child(stat(
                    Lucide::Check,
                    theme::muted(),
                    tr(
                        "Déjà aligné, rien ne change",
                        "Already aligned, nothing changes",
                    )
                    .into(),
                ));
            }
            dialog
                .bg(theme::sheet())
                .w(px(500.))
                .margin_top(top)
                .child(
                    v_flex()
                        .gap_4()
                        .child(
                            h_flex()
                                .gap_3()
                                .child(
                                    div()
                                        .flex_none()
                                        .size(px(44.))
                                        .rounded_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .bg(theme::alpha(theme::red(), 0.16))
                                        .border_1()
                                        .border_color(theme::alpha(theme::red(), 0.45))
                                        .child(if going {
                                            // Winds back a full turn.
                                            Icon::new(Lucide::RotateCcw)
                                                .size(px(20.))
                                                .text_color(theme::red())
                                                .with_animation(
                                                    "reset-spin",
                                                    Animation::new(RESET_SENDOFF)
                                                        .with_easing(gpui_kit::ease_out_quint()),
                                                    |icon, t| {
                                                        icon.rotate(gpui_kit::radians(
                                                            -t * std::f32::consts::TAU,
                                                        ))
                                                    },
                                                )
                                                .into_any_element()
                                        } else {
                                            Icon::new(Lucide::RotateCcw)
                                                .size(px(20.))
                                                .text_color(theme::red())
                                                .into_any_element()
                                        }),
                                )
                                .child(
                                    v_flex()
                                        .gap_0p5()
                                        .child(
                                            div()
                                                .text_lg()
                                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                                .child(tr(
                                                    "Aligner sur le remote",
                                                    "Reset to remote",
                                                )),
                                        )
                                        .child(div().text_sm().text_color(theme::muted()).child(
                                            if checkout.is_some() {
                                                tr(
                                                    "La branche prend exactement l'état du remote, puis est checkoutée.",
                                                    "The branch takes exactly the remote's state, then is checked out.",
                                                )
                                            } else {
                                                tr(
                                                    "La branche prend exactement l'état du remote.",
                                                    "The branch takes exactly the remote's state.",
                                                )
                                            },
                                        )),
                                ),
                        )
                        // local ──→ remote
                        .child(
                            h_flex()
                                .justify_center()
                                .gap_3()
                                .p_3()
                                .relative()
                                .overflow_hidden()
                                .rounded_lg()
                                .bg(theme::alpha(gpui_kit::white(), 0.04))
                                .border_1()
                                .border_color(theme::hairline())
                                .child(if going {
                                    // The local branch is pulled onto the remote one.
                                    div()
                                        .relative()
                                        .child(ref_pill(&name, Lucide::GitBranch, theme::accent()))
                                        .with_animation(
                                            "reset-pull",
                                            Animation::new(RESET_SENDOFF)
                                                .with_easing(gpui_kit::ease_in_out),
                                            |el, t| el.left(px(70. * t)).opacity(1. - t),
                                        )
                                        .into_any_element()
                                } else {
                                    ref_pill(&name, Lucide::GitBranch, theme::accent())
                                        .into_any_element()
                                })
                                .child(
                                    Icon::new(Lucide::ArrowLeft)
                                        .small()
                                        .text_color(theme::muted()),
                                )
                                .child(ref_pill(&target, Lucide::Cloud, theme::violet()))
                                .when(going, |s| s.child(super::shimmer::sweep(1))),
                        )
                        .child(stats),
                )
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new()
                                .trigger(|b| b.ghost().label(tr("Annuler", "Cancel"))),
                        )
                        .when_some(checkout, |footer, changes| {
                            // Switch to the local branch as it is, without the reset.
                            let (name, view) = (name.clone(), view.clone());
                            footer.child(
                                Button::new("reset-checkout-only")
                                    .ghost()
                                    .label(tr("Checkout seul", "Just check out"))
                                    .on_click(move |_, window, cx| {
                                        window.close_dialog(cx);
                                        let name = name.clone();
                                        let _ = view.update(cx, |this, cx| {
                                            this.checkout_branch(name, None, changes, window, cx)
                                        });
                                    }),
                            )
                        })
                        .child(
                            DialogAction::new().child(
                                Button::new("reset-ok")
                                    .danger()
                                    .icon(Icon::new(Lucide::RotateCcw))
                                    .label(tr("Reset", "Reset")),
                            ),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    if launched.replace(true) {
                        return false;
                    }
                    window.refresh();
                    let (name, target, view) = (name.clone(), target.clone(), view.clone());
                    window
                        .spawn(cx, async move |cx| {
                            cx.background_executor().timer(RESET_SENDOFF).await;
                            let _ = cx.update(|window, cx| {
                                window.close_dialog(cx);
                                let _ = view.update(cx, |this, cx| match checkout {
                                    // Moves the branch first: the checkout then lands on it.
                                    Some(changes) => this.run_op(
                                        "Reset",
                                        move |r| {
                                            r.reset_branch(&name, &target, false)?;
                                            r.switch_to(&name, None, changes)
                                        },
                                        window,
                                        cx,
                                    ),
                                    None => this.run_op(
                                        "Reset",
                                        move |r| r.reset_branch(&name, &target, is_head),
                                        window,
                                        cx,
                                    ),
                                });
                            });
                        })
                        .detach();
                    false
                })
        });
    }
}
