//! Fork-like sheets: checkout (track a remote branch, handle local changes)
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
    AppContext as _, Context, Entity, Focusable as _, Image, ImageFormat, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
    div, img, prelude::FluentBuilder as _, px,
};
use mugetsu_git::{LocalChanges, PushOptions};

use super::RepoView;
use super::sidebar::default_remote;
use crate::i18n::tr;
use crate::theme;

static APP_ICON: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../assets/icon/mugetsu.png").to_vec(),
    ))
});

/// Icon, title and subtitle at the top of a sheet, like Fork's.
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
    /// through when nothing needs deciding, otherwise asks like Fork does.
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
            let description = if dirty {
                tr(
                    "La branche revient exactement au remote. Les commits locaux non poussés et les modifications non commitées seront perdus.",
                    "The branch is set exactly to the remote. Unpushed local commits and uncommitted changes will be lost.",
                )
            } else {
                tr(
                    "La branche revient exactement au remote. Les commits locaux non poussés seront perdus.",
                    "The branch is set exactly to the remote. Unpushed local commits will be lost.",
                )
            };
            self.confirm(
                trf!(
                    "Aligner '{name}' sur '{upstream}' ?",
                    "Reset '{name}' to '{upstream}'?"
                ),
                description.into(),
                tr("Reset", "Reset"),
                move |this, window, cx| {
                    let (name, upstream) = (name.clone(), upstream.clone());
                    this.run_op(
                        "Reset",
                        move |r| r.reset_branch(&name, &upstream, true),
                        window,
                        cx,
                    );
                },
                window,
                cx,
            );
            return;
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
                    let _ = view.update(cx, |this, cx| {
                        this.run_op(
                            "Push",
                            move |r| r.push_to(&remote, &local, &remote_branch, options),
                            window,
                            cx,
                        )
                    });
                    true
                })
        });
    }
}
