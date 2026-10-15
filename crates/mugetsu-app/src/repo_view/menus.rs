//! Right-click menus (Fork-like) for branches, remote branches, tags and commits,
//! plus the small text-prompt and confirmation dialogs they open.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gix::ObjectId;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, WindowExt as _};
use gpui_kit::{App, AppContext as _, ClickEvent, ClipboardItem, Context, WeakEntity, Window};
use mugetsu_forge::{GithubRepo, PullRequest};
use mugetsu_git::{RefLabel, ResetMode};

use super::{RepoEvent, RepoView};
use crate::i18n::tr;
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
        Some(PrLookup::Found(pr)) => Some((**pr).clone()),
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
                trf!(
                    "Ouvrir la Pull Request #{}",
                    "Open Pull Request #{}",
                    pr.number
                ),
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
                trf!(
                    "Créer une Pull Request sur '{remote}'",
                    "Create Pull Request on '{remote}'"
                ),
                Lucide::GitPullRequestCreateArrow,
            )
            .on_click(move |_, _, cx| cx.open_url(&url))
        }
        (None, None) => item(
            trf!(
                "Créer une Pull Request sur '{remote}'",
                "Create Pull Request on '{remote}'"
            ),
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
                this.request_checkout(name.clone(), false, window, cx);
            }))
    };
    let worktree = {
        let (name, default) = (name.clone(), worktree_path(&ctx.root, &b.name));
        item(
            tr("Checkout en worktree…", "Checkout as worktree…"),
            Lucide::FolderGit2,
        )
        .disabled(b.is_head)
        .on_click(act(&ctx.view, move |this, window, cx| {
            let name = name.clone();
            this.prompt(
                trf!(
                    "Checkout de '{name}' en worktree",
                    "Checkout '{name}' as worktree"
                ),
                tr("Dossier du worktree", "Worktree folder"),
                default.clone(),
                tr("Créer", "Create"),
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
            trf!(
                "Fast-forward vers '{}'",
                "Fast-forward to '{}'",
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
        let (name, remote) = (name.clone(), remote.clone());
        item(
            trf!(
                "Push vers '{}'…",
                "Push to '{}'…",
                remote.as_deref().unwrap_or("remote")
            ),
            Lucide::ArrowUpFromLine,
        )
        .disabled(remote.is_none())
        .on_click(act(&ctx.view, move |this, window, cx| {
            this.open_push_dialog(Some(name.clone()), window, cx)
        }))
    };
    let reset_to_upstream = {
        let (name, upstream, is_head) = (name.clone(), b.upstream.clone(), b.is_head);
        item(
            trf!(
                "Reset '{}' sur '{}'…",
                "Reset '{}' to '{}'…",
                b.name,
                b.upstream.as_deref().unwrap_or("upstream")
            ),
            Lucide::RotateCcw,
        )
        .disabled(b.upstream.is_none() || b.gone)
        .on_click(act(&ctx.view, move |this, window, cx| {
            let (name, upstream) = (name.clone(), upstream.clone().unwrap_or_default());
            this.confirm_reset(name, upstream, is_head, window, cx);
        }))
    };
    let rename = {
        let name = name.clone();
        item(tr("Renommer…", "Rename…"), Lucide::Pencil).on_click(act(
            &ctx.view,
            move |this, window, cx| {
                let old = name.clone();
                this.prompt(
                    trf!("Renommer '{old}'", "Rename '{old}'"),
                    tr("Nouveau nom", "New name"),
                    old.clone(),
                    tr("Renommer", "Rename"),
                    move |this, new, window, cx| {
                        let old = old.clone();
                        this.run_op(
                            tr("Renommer", "Rename"),
                            move |r| r.rename_branch(&old, &new),
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
    let delete = {
        let name = name.clone();
        item(tr("Supprimer…", "Delete…"), Lucide::Trash)
            .disabled(b.is_head)
            .on_click(act(&ctx.view, move |this, window, cx| {
                let name = name.clone();
                this.confirm(
                    trf!("Supprimer la branche '{name}' ?", "Delete branch '{name}'?"),
                    tr(
                        "La branche locale est supprimée. Git refuse si elle n'est pas mergée.",
                        "The local branch is deleted. Git refuses if it isn't merged.",
                    )
                    .into(),
                    tr("Supprimer", "Delete"),
                    move |this, window, cx| {
                        let name = name.clone();
                        this.run_op(
                            tr("Supprimer", "Delete"),
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
        .item(reset_to_upstream)
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
        .item(
            item(
                tr("Copier le nom de la branche", "Copy branch name"),
                Lucide::Copy,
            )
            .on_click(copy(name)),
        )
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
            this.request_checkout(full.clone(), true, window, cx);
        }))
    };
    let delete = {
        let (remote, branch) = (remote.clone(), branch.clone());
        item(
            trf!("Supprimer de '{remote}'…", "Delete from '{remote}'…"),
            Lucide::Trash,
        )
        .on_click(act(&ctx.view, move |this, window, cx| {
            let (remote, branch) = (remote.clone(), branch.clone());
            this.confirm(
                trf!(
                    "Supprimer '{branch}' sur '{remote}' ?",
                    "Delete '{branch}' on '{remote}'?"
                ),
                tr(
                    "La branche est supprimée sur le serveur pour tout le monde.",
                    "The branch is deleted on the server for everyone.",
                )
                .into(),
                tr("Supprimer", "Delete"),
                move |this, window, cx| {
                    let (remote, branch) = (remote.clone(), branch.clone());
                    this.run_op(
                        tr("Supprimer", "Delete"),
                        move |r| r.delete_remote_branch(&remote, &branch),
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
        .separator()
        .item(pr_item(ctx, &remote, &branch, pr))
        .item(delete)
        .separator()
        .item(
            item(
                tr("Copier le nom de la branche", "Copy branch name"),
                Lucide::Copy,
            )
            .on_click(copy(full)),
        )
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
            trf!(
                "Push vers '{}'",
                "Push to '{}'",
                remote.as_deref().unwrap_or("remote")
            ),
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
        item(tr("Supprimer…", "Delete…"), Lucide::Trash).on_click(act(
            &ctx.view,
            move |this, window, cx| {
                let tag = tag.clone();
                this.confirm(
                    trf!("Supprimer le tag '{tag}' ?", "Delete tag '{tag}'?"),
                    tr(
                        "Le tag local est supprimé (pas celui du remote).",
                        "The local tag is deleted (not the remote one).",
                    )
                    .into(),
                    tr("Supprimer", "Delete"),
                    move |this, window, cx| {
                        let tag = tag.clone();
                        this.run_op(
                            tr("Supprimer tag", "Delete tag"),
                            move |r| r.delete_tag(&tag),
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
        .item(push)
        .separator()
        .item(delete)
        .separator()
        .item(item(tr("Copier le nom du tag", "Copy tag name"), Lucide::Copy).on_click(copy(tag)))
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
        item(
            tr("Nouvelle branche…", "New branch…"),
            Lucide::GitBranchPlus,
        )
        .on_click(act(&ctx.view, move |this, window, cx| {
            let sha = sha.clone();
            this.prompt(
                tr("Nouvelle branche", "New branch").into(),
                tr("Nom de la branche", "Branch name"),
                String::new(),
                tr("Créer et checkout", "Create and checkout"),
                move |this, name, window, cx| {
                    let sha = sha.clone();
                    this.run_op(
                        tr("Nouvelle branche", "New branch"),
                        move |r| r.create_branch(&name, Some(&sha), true),
                        window,
                        cx,
                    );
                },
                window,
                cx,
            );
        }))
    };
    let new_tag = {
        let sha = sha.clone();
        item(tr("Nouveau tag…", "New tag…"), Lucide::Tag).on_click(act(
            &ctx.view,
            move |this, window, cx| {
                let sha = sha.clone();
                this.prompt(
                    tr("Nouveau tag", "New tag").into(),
                    tr("Nom du tag", "Tag name"),
                    String::new(),
                    tr("Créer", "Create"),
                    move |this, name, window, cx| {
                        let sha = sha.clone();
                        this.run_op(
                            tr("Nouveau tag", "New tag"),
                            move |r| r.create_tag(&name, &sha, None),
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
    menu = menu.item(new_branch).item(new_tag).separator();

    if let Some(head) = ctx.head_branch.clone() {
        let view = ctx.view.clone();
        let sha = sha.clone();
        menu = menu.submenu_with_icon(
            Some(Icon::new(Lucide::RotateCcw)),
            trf!("Reset '{head}' ici", "Reset '{head}' here"),
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
                                trf!("Reset --hard de '{head}' ?", "Reset --hard '{head}'?"),
                                tr(
                                    "Les modifications non commitées seront perdues.",
                                    "Uncommitted changes will be lost.",
                                )
                                .into(),
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
                    tr(
                        "Soft : garde l'index et les fichiers",
                        "Soft: keep index and files",
                    ),
                    ResetMode::Soft,
                ))
                .item(reset(
                    tr("Mixed : garde les fichiers", "Mixed: keep files"),
                    ResetMode::Mixed,
                ))
                .item(reset(
                    tr("Hard : écrase tout…", "Hard: discard everything…"),
                    ResetMode::Hard,
                ))
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
        item(
            tr("Enregistrer en patch…", "Save as patch…"),
            Lucide::FileDown,
        )
        .on_click(act(&ctx.view, move |this, window, cx| {
            this.save_patch(sha.clone(), short.clone(), window, cx)
        }))
    };
    menu.item(simple(
        tr("Checkout du commit", "Checkout commit"),
        Lucide::GitCommitHorizontal,
        |r, s, _| r.checkout_detached(s),
    ))
    .item(simple("Cherry-pick", Lucide::Cherry, |r, s, m| {
        r.cherry_pick(s, m)
    }))
    .item(simple("Revert", Lucide::Undo2, |r, s, m| r.revert(s, m)))
    .item(save_patch)
    .separator()
    .item(item(tr("Copier le SHA", "Copy SHA"), Lucide::Copy).on_click(copy(sha)))
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
        crate::dialogs::prompt(title, placeholder, initial, ok, action, window, cx);
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
        crate::dialogs::confirm(title, description, ok, action, window, cx);
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
