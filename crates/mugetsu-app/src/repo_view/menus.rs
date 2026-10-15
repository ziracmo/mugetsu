//! Right-click menus (Fork-like) for branches, remote branches, tags and commits,
//! plus the small text-prompt and confirmation dialogs they open.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gix::ObjectId;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, WindowExt as _};
use gpui_kit::{
    App, AppContext as _, ClickEvent, ClipboardItem, Context, Focusable as _, ParentElement as _,
    WeakEntity, Window, px,
};
use mugetsu_forge::{GithubRepo, PullRequest};
use mugetsu_git::{RefLabel, ResetMode};

use super::{RepoEvent, RepoView};
use crate::pr::PrLookup;

pub type MenuBuilder = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

type Handler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Click handler running `f` on the repository view.
fn act(
    view: &WeakEntity<RepoView>,
    f: impl Fn(&mut RepoView, &mut Window, &mut Context<RepoView>) + 'static,
) -> Handler {
    let view = view.clone();
    Box::new(move |_, window, cx| {
        let _ = view.update(cx, |this, cx| f(this, window, cx));
    })
}

fn item(label: impl Into<gpui_kit::SharedString>, icon: Lucide) -> PopupMenuItem {
    PopupMenuItem::new(label).icon(Icon::new(icon))
}

fn copy(text: String) -> Handler {
    Box::new(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())))
}

/// Everything a menu needs to know about the repository, captured at render time.
#[derive(Clone)]
pub struct MenuContext {
    pub view: WeakEntity<RepoView>,
    pub root: PathBuf,
    pub github: Option<GithubRepo>,
    pub default_remote: Option<String>,
    pub head_branch: Option<String>,
}

/// A local branch as shown in a menu.
#[derive(Clone)]
pub struct BranchMenu {
    pub name: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub is_head: bool,
    pub gone: bool,
    pub pr: Option<PullRequest>,
}

pub fn found_pr(lookup: Option<&PrLookup>) -> Option<PullRequest> {
    match lookup {
        Some(PrLookup::Found(pr)) => Some(pr.clone()),
        _ => None,
    }
}

/// `https://github.com/o/r/compare/branch?expand=1`, or the PR itself when it exists.
fn pr_item(
    ctx: &MenuContext,
    remote: &str,
    branch: &str,
    pr: Option<&PullRequest>,
) -> PopupMenuItem {
    match (pr, &ctx.github) {
        (Some(pr), _) => {
            let url = pr.url.clone();
            item(
                format!("Ouvrir la Pull Request #{}", pr.number),
                Lucide::GitPullRequest,
            )
            .on_click(move |_, _, cx| cx.open_url(&url))
        }
        (None, Some(gh)) => {
            let url = format!(
                "https://github.com/{}/{}/compare/{branch}?expand=1",
                gh.owner, gh.name
            );
            item(
                format!("Créer une Pull Request sur '{remote}'"),
                Lucide::GitPullRequestCreateArrow,
            )
            .on_click(move |_, _, cx| cx.open_url(&url))
        }
        (None, None) => item(
            format!("Créer une Pull Request sur '{remote}'"),
            Lucide::GitPullRequestCreateArrow,
        )
        .disabled(true),
    }
}

fn worktree_path(root: &Path, branch: &str) -> String {
    let repo = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".into());
    root.parent()
        .unwrap_or(root)
        .join(format!("{repo}-{}", branch.replace('/', "-")))
        .to_string_lossy()
        .to_string()
}

pub fn local_branch_menu(menu: PopupMenu, ctx: &MenuContext, b: &BranchMenu) -> PopupMenu {
    let name = b.name.clone();
    let remote = b
        .upstream
        .as_deref()
        .and_then(|u| u.split_once('/').map(|(r, _)| r.to_string()))
        .or_else(|| ctx.default_remote.clone());
    let can_ff = b.upstream.is_some() && !b.gone && b.behind > 0 && b.ahead == 0;
    let forge_branch = crate::pr::forge_branch(&b.name, b.upstream.as_deref(), false);

    let checkout = {
        let name = name.clone();
        item("Checkout", Lucide::Check)
            .disabled(b.is_head)
            .on_click(act(&ctx.view, move |this, window, cx| {
                let name = name.clone();
                this.run_op("Checkout", move |r| r.checkout(&name), window, cx);
            }))
    };
    let worktree = {
        let (name, default) = (name.clone(), worktree_path(&ctx.root, &b.name));
        item("Checkout en worktree…", Lucide::FolderGit2)
            .disabled(b.is_head)
            .on_click(act(&ctx.view, move |this, window, cx| {
                let name = name.clone();
                this.prompt(
                    format!("Checkout de '{name}' en worktree"),
                    "Dossier du worktree",
                    default.clone(),
                    "Créer",
                    move |this, path, window, cx| {
                        let (name, open) = (name.clone(), PathBuf::from(&path));
                        this.run_op_then(
                            "Worktree",
                            move |r| r.worktree_add(&path, &name, false),
                            move |_, _, cx| cx.emit(RepoEvent::OpenWorktree(open)),
                            window,
                            cx,
                        );
                    },
                    window,
                    cx,
                );
            }))
    };
    let fast_forward = {
        let (name, upstream, is_head) = (name.clone(), b.upstream.clone(), b.is_head);
        item(
            format!(
                "Fast-forward vers '{}'",
                b.upstream.as_deref().unwrap_or("upstream")
            ),
            Lucide::FastForward,
        )
        .disabled(!can_ff)
        .on_click(act(&ctx.view, move |this, window, cx| {
            let (name, upstream) = (name.clone(), upstream.clone().unwrap_or_default());
            this.run_op(
                "Fast-forward",
                move |r| r.fast_forward(&name, &upstream, is_head),
                window,
                cx,
            );
        }))
    };
    let push = {
        let (name, remote, has_upstream) = (
            name.clone(),
            remote.clone(),
            b.upstream.is_some() && !b.gone,
        );
        item(
            format!("Push vers '{}'", remote.as_deref().unwrap_or("remote")),
            Lucide::ArrowUpFromLine,
        )
        .disabled(remote.is_none())
        .on_click(act(&ctx.view, move |this, window, cx| {
            let (name, remote) = (name.clone(), remote.clone().unwrap_or_default());
            this.run_op(
                "Push",
                move |r| r.push_branch(&remote, &name, !has_upstream),
                window,
                cx,
            );
        }))
    };
    let rename = {
        let name = name.clone();
        item("Renommer…", Lucide::Pencil).on_click(act(&ctx.view, move |this, window, cx| {
            let old = name.clone();
            this.prompt(
                format!("Renommer '{old}'"),
                "Nouveau nom",
                old.clone(),
                "Renommer",
                move |this, new, window, cx| {
                    let old = old.clone();
                    this.run_op("Renommer", move |r| r.rename_branch(&old, &new), window, cx);
                },
                window,
                cx,
            );
        }))
    };
    let delete = {
        let name = name.clone();
        item("Supprimer…", Lucide::Trash)
            .disabled(b.is_head)
            .on_click(act(&ctx.view, move |this, window, cx| {
                let name = name.clone();
                this.confirm(
                    format!("Supprimer la branche '{name}' ?"),
                    "La branche locale est supprimée. Git refuse si elle n'est pas mergée.".into(),
                    "Supprimer",
                    move |this, window, cx| {
                        let name = name.clone();
                        this.run_op(
                            "Supprimer",
                            move |r| r.delete_branch(&name, false),
                            window,
                            cx,
                        );
                    },
                    window,
                    cx,
                );
            }))
    };

    menu.item(checkout)
        .item(worktree)
        .separator()
        .item(fast_forward)
        .item(push)
        .item(pr_item(
            ctx,
            remote.as_deref().unwrap_or("origin"),
            &forge_branch,
            b.pr.as_ref(),
        ))
        .separator()
        .item(rename)
        .item(delete)
        .separator()
        .item(item("Copier le nom de la branche", Lucide::Copy).on_click(copy(name)))
}

/// `remote_branch` is `origin/feat/x`.
pub fn remote_branch_menu(
    menu: PopupMenu,
    ctx: &MenuContext,
    remote_branch: &str,
    pr: Option<&PullRequest>,
) -> PopupMenu {
    let (remote, branch) = remote_branch
        .split_once('/')
        .map(|(r, b)| (r.to_string(), b.to_string()))
        .unwrap_or_default();
    let full = remote_branch.to_string();
    let checkout = {
        let full = full.clone();
        item("Checkout", Lucide::Check).on_click(act(&ctx.view, move |this, window, cx| {
            let full = full.clone();
            this.run_op("Checkout", move |r| r.checkout_remote(&full), window, cx);
        }))
    };
    let delete = {
        let (remote, branch) = (remote.clone(), branch.clone());
        item(format!("Supprimer de '{remote}'…"), Lucide::Trash).on_click(act(
            &ctx.view,
            move |this, window, cx| {
                let (remote, branch) = (remote.clone(), branch.clone());
                this.confirm(
                    format!("Supprimer '{branch}' sur '{remote}' ?"),
                    "La branche est supprimée sur le serveur pour tout le monde.".into(),
                    "Supprimer",
                    move |this, window, cx| {
                        let (remote, branch) = (remote.clone(), branch.clone());
                        this.run_op(
                            "Supprimer",
                            move |r| r.delete_remote_branch(&remote, &branch),
                            window,
                            cx,
                        );
                    },
                    window,
                    cx,
                );
            },
        ))
    };
    menu.item(checkout)
        .separator()
        .item(pr_item(ctx, &remote, &branch, pr))
        .item(delete)
        .separator()
        .item(item("Copier le nom de la branche", Lucide::Copy).on_click(copy(full)))
}

pub fn tag_menu(menu: PopupMenu, ctx: &MenuContext, tag: &str) -> PopupMenu {
    let tag = tag.to_string();
    let remote = ctx.default_remote.clone();
    let checkout = {
        let tag = tag.clone();
        item("Checkout", Lucide::Check).on_click(act(&ctx.view, move |this, window, cx| {
            let tag = tag.clone();
            this.run_op("Checkout", move |r| r.checkout_detached(&tag), window, cx);
        }))
    };
    let push = {
        let (tag, remote) = (tag.clone(), remote.clone());
        item(
            format!("Push vers '{}'", remote.as_deref().unwrap_or("remote")),
            Lucide::ArrowUpFromLine,
        )
        .disabled(remote.is_none())
        .on_click(act(&ctx.view, move |this, window, cx| {
            let (tag, remote) = (tag.clone(), remote.clone().unwrap_or_default());
            this.run_op("Push tag", move |r| r.push_tag(&remote, &tag), window, cx);
        }))
    };
    let delete = {
        let tag = tag.clone();
        item("Supprimer…", Lucide::Trash).on_click(act(&ctx.view, move |this, window, cx| {
            let tag = tag.clone();
            this.confirm(
                format!("Supprimer le tag '{tag}' ?"),
                "Le tag local est supprimé (pas celui du remote).".into(),
                "Supprimer",
                move |this, window, cx| {
                    let tag = tag.clone();
                    this.run_op("Supprimer tag", move |r| r.delete_tag(&tag), window, cx);
                },
                window,
                cx,
            );
        }))
    };
    menu.item(checkout)
        .item(push)
        .separator()
        .item(delete)
        .separator()
        .item(item("Copier le nom du tag", Lucide::Copy).on_click(copy(tag)))
}

/// Commit row menu: a submenu per ref on the commit, then commit actions.
#[allow(clippy::too_many_arguments)]
pub fn commit_menu(
    mut menu: PopupMenu,
    ctx: &MenuContext,
    id: ObjectId,
    is_merge: bool,
    labels: &[RefLabel],
    branches: &[BranchMenu],
    remote_prs: &[(String, Option<PullRequest>)],
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    for label in labels {
        match label {
            RefLabel::Branch { name, .. } => {
                if let Some(b) = branches.iter().find(|b| &b.name == name).cloned() {
                    let ctx = ctx.clone();
                    menu = menu.submenu_with_icon(
                        Some(Icon::new(Lucide::GitBranch)),
                        name.clone(),
                        window,
                        cx,
                        move |m, _, _| local_branch_menu(m, &ctx, &b),
                    );
                }
            }
            RefLabel::Remote(name) => {
                let ctx = ctx.clone();
                let name = name.clone();
                let pr = remote_prs
                    .iter()
                    .find(|(n, _)| *n == name)
                    .and_then(|(_, pr)| pr.clone());
                menu = menu.submenu_with_icon(
                    Some(Icon::new(Lucide::Cloud)),
                    name.clone(),
                    window,
                    cx,
                    move |m, _, _| remote_branch_menu(m, &ctx, &name, pr.as_ref()),
                );
            }
            RefLabel::Tag(name) => {
                let ctx = ctx.clone();
                let name = name.clone();
                menu = menu.submenu_with_icon(
                    Some(Icon::new(Lucide::Tag)),
                    name.clone(),
                    window,
                    cx,
                    move |m, _, _| tag_menu(m, &ctx, &name),
                );
            }
            RefLabel::DetachedHead => {}
        }
    }
    if !labels.is_empty() {
        menu = menu.separator();
    }

    let sha = id.to_string();
    let short = id.to_hex_with_len(7).to_string();
    let new_branch = {
        let sha = sha.clone();
        item("Nouvelle branche…", Lucide::GitBranchPlus).on_click(act(
            &ctx.view,
            move |this, window, cx| {
                let sha = sha.clone();
                this.prompt(
                    "Nouvelle branche".into(),
                    "Nom de la branche",
                    String::new(),
                    "Créer et checkout",
                    move |this, name, window, cx| {
                        let sha = sha.clone();
                        this.run_op(
                            "Nouvelle branche",
                            move |r| r.create_branch(&name, Some(&sha), true),
                            window,
                            cx,
                        );
                    },
                    window,
                    cx,
                );
            },
        ))
    };
    let new_tag = {
        let sha = sha.clone();
        item("Nouveau tag…", Lucide::Tag).on_click(act(&ctx.view, move |this, window, cx| {
            let sha = sha.clone();
            this.prompt(
                "Nouveau tag".into(),
                "Nom du tag",
                String::new(),
                "Créer",
                move |this, name, window, cx| {
                    let sha = sha.clone();
                    this.run_op(
                        "Nouveau tag",
                        move |r| r.create_tag(&name, &sha, None),
                        window,
                        cx,
                    );
                },
                window,
                cx,
            );
        }))
    };
    menu = menu.item(new_branch).item(new_tag).separator();

    if let Some(head) = ctx.head_branch.clone() {
        let view = ctx.view.clone();
        let sha = sha.clone();
        menu = menu.submenu_with_icon(
            Some(Icon::new(Lucide::RotateCcw)),
            format!("Reset '{head}' ici"),
            window,
            cx,
            move |m, _, _| {
                let reset = |label: &'static str, mode: ResetMode| {
                    let sha = sha.clone();
                    let head = head.clone();
                    PopupMenuItem::new(label).on_click(act(&view, move |this, window, cx| {
                        let sha = sha.clone();
                        let run =
                            move |this: &mut RepoView,
                                  window: &mut Window,
                                  cx: &mut Context<RepoView>| {
                                let sha = sha.clone();
                                this.run_op("Reset", move |r| r.reset(&sha, mode), window, cx);
                            };
                        if mode == ResetMode::Hard {
                            this.confirm(
                                format!("Reset --hard de '{head}' ?"),
                                "Les modifications non commitées seront perdues.".into(),
                                "Reset",
                                run,
                                window,
                                cx,
                            );
                        } else {
                            run(this, window, cx);
                        }
                    }))
                };
                m.item(reset(
                    "Soft : garde l'index et les fichiers",
                    ResetMode::Soft,
                ))
                .item(reset("Mixed : garde les fichiers", ResetMode::Mixed))
                .item(reset("Hard : écrase tout…", ResetMode::Hard))
            },
        );
    }

    let simple = |label: &'static str,
                  icon: Lucide,
                  op: fn(&mugetsu_git::Repo, &str, bool) -> anyhow::Result<()>| {
        let sha = sha.clone();
        item(label, icon).on_click(act(&ctx.view, move |this, window, cx| {
            let sha = sha.clone();
            this.run_op(label, move |r| op(r, &sha, is_merge), window, cx);
        }))
    };
    let save_patch = {
        let (sha, short) = (sha.clone(), short.clone());
        item("Enregistrer en patch…", Lucide::FileDown)
            .on_click(act(&ctx.view, move |this, window, cx| {
                this.save_patch(sha.clone(), short.clone(), window, cx)
            }))
    };
    menu.item(simple(
        "Checkout du commit",
        Lucide::GitCommitHorizontal,
        |r, s, _| r.checkout_detached(s),
    ))
    .item(simple("Cherry-pick", Lucide::Cherry, |r, s, m| {
        r.cherry_pick(s, m)
    }))
    .item(simple("Revert", Lucide::Undo2, |r, s, m| r.revert(s, m)))
    .item(save_patch)
    .separator()
    .item(item("Copier le SHA", Lucide::Copy).on_click(copy(sha)))
}

impl RepoView {
    /// Opens a one-field dialog; `action` gets the trimmed, non-empty value.
    #[allow(clippy::too_many_arguments)]
    pub fn prompt(
        &mut self,
        title: String,
        placeholder: &'static str,
        initial: String,
        ok: &'static str,
        action: impl Fn(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(initial)
        });
        let view = cx.entity().downgrade();
        let action = Rc::new(action);
        let focus_once = Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if focus_once.replace(false) {
                input.focus_handle(cx).focus(window, cx);
            }
            let (input_ok, view, action) = (input.clone(), view.clone(), action.clone());
            dialog
                .title(title.clone())
                .w(px(460.))
                .child(Input::new(&input))
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().trigger(|b| b.ghost().label("Annuler")))
                        .child(
                            DialogAction::new().child(Button::new("prompt-ok").primary().label(ok)),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let value = input_ok.read(cx).value().trim().to_string();
                    if value.is_empty() {
                        return false;
                    }
                    let _ = view.update(cx, |this, cx| action(this, value, window, cx));
                    true
                })
        });
    }

    /// Asks before a destructive action.
    pub fn confirm(
        &mut self,
        title: String,
        description: String,
        ok: &'static str,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity().downgrade();
        let action = Rc::new(action);
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (view, action) = (view.clone(), action.clone());
            alert
                .title(title.clone())
                .description(description.clone())
                .confirm()
                .ok_text(ok)
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Annuler")
                .on_ok(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| action(this, window, cx));
                    true
                })
        });
    }

    fn save_patch(
        &mut self,
        sha: String,
        short: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dir = dirs::download_dir().unwrap_or_else(|| self.repo.root().to_path_buf());
        let picked = cx.prompt_for_new_path(&dir, Some(&format!("{short}.patch")));
        let repo = self.repo.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = picked.await else {
                return;
            };
            let result = cx
                .background_spawn(async move {
                    let patch = repo.format_patch(&sha)?;
                    std::fs::write(&path, patch)?;
                    anyhow::Ok(())
                })
                .await;
            if let Err(e) = result {
                let _ = this.update_in(cx, |_, window, cx| {
                    window.push_notification(
                        gpui_kit::component::notification::Notification::error(format!(
                            "Patch : {e:#}"
                        )),
                        cx,
                    )
                });
            }
        })
        .detach();
    }
}
