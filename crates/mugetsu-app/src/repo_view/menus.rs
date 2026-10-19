//! Right-click menus for branches, remote branches, tags and commits,
//! plus the small text-prompt and confirmation dialogs they open.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gix::ObjectId;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{Icon, WindowExt as _};
use gpui_kit::{
    App, AppContext as _, ClickEvent, ClipboardItem, Context, WeakEntity, Window,
    prelude::FluentBuilder as _,
};
use mugetsu_forge::{ForgeRepo, GithubRepo, PullRequest};
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
    /// Set for GitHub remotes only (GitHub links).
    pub github: Option<GithubRepo>,
    pub forge: Option<ForgeRepo>,
    pub default_remote: Option<String>,
    pub head_branch: Option<String>,
    pub head: Option<ObjectId>,
    /// A rebase is stopped mid-way.
    pub rebasing: bool,
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
    /// The branch heads a PR of a stack: it can be restacked.
    pub in_stack: bool,
}

pub fn found_pr(lookup: Option<&PrLookup>) -> Option<PullRequest> {
    match lookup {
        Some(PrLookup::Found(pr)) => Some((**pr).clone()),
        _ => None,
    }
}

/// The forge's "new pull / merge request" page, or the PR itself when it exists.
fn pr_item(
    ctx: &MenuContext,
    remote: &str,
    branch: &str,
    pr: Option<&PullRequest>,
) -> PopupMenuItem {
    let noun = ctx
        .forge
        .as_ref()
        .map_or("Pull Request", |f| f.kind.pr_noun());
    match (pr, &ctx.forge) {
        (Some(pr), forge) => {
            let url = pr.url.clone();
            let number = forge
                .as_ref()
                .map_or_else(|| format!("#{}", pr.number), |f| f.kind.pr_ref(pr.number));
            item(
                trf!("Ouvrir la {noun} {number}", "Open {noun} {number}"),
                Lucide::GitPullRequest,
            )
            .on_click(move |_, _, cx| cx.open_url(&url))
        }
        (None, Some(forge)) => {
            let url = forge.new_pr_url(branch);
            item(
                trf!(
                    "Créer une {noun} sur '{remote}'",
                    "Create {noun} on '{remote}'"
                ),
                Lucide::GitPullRequestCreateArrow,
            )
            .on_click(move |_, _, cx| cx.open_url(&url))
        }
        (None, None) => item(
            trf!(
                "Créer une {noun} sur '{remote}'",
                "Create {noun} on '{remote}'"
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
    let head = ctx.head_branch.clone().unwrap_or_else(|| "HEAD".into());
    let merge = {
        let name = name.clone();
        item(
            trf!(
                "Merger '{}' dans '{head}'",
                "Merge '{}' into '{head}'",
                b.name
            ),
            Lucide::GitMerge,
        )
        .disabled(b.is_head)
        .on_click(act(&ctx.view, move |this, window, cx| {
            let name = name.clone();
            this.run_op(
                "Merge",
                move |r| {
                    r.merge(&name)
                        .and_then(super::conflicts::stopped_on_conflicts)
                },
                window,
                cx,
            );
        }))
    };
    let rebase = {
        let name = name.clone();
        item(
            trf!(
                "Rebaser '{head}' sur '{}'",
                "Rebase '{head}' onto '{}'",
                b.name
            ),
            Lucide::GitBranch,
        )
        .disabled(b.is_head || ctx.head_branch.is_none())
        .on_click(act(&ctx.view, move |this, window, cx| {
            this.rebase_head_onto(name.clone(), window, cx)
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
            this.confirm_reset(name, upstream, is_head, None, window, cx);
        }))
    };
    let restack = {
        let head = b.pr.as_ref().map(|p| p.head.clone()).unwrap_or_default();
        item(
            tr("Restacker la stack…", "Restack the stack…"),
            Lucide::RefreshCw,
        )
        .disabled(!b.in_stack)
        .on_click(act(&ctx.view, move |this, window, cx| {
            this.confirm_restack(head.clone(), window, cx)
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
                this.request_delete_branch(name.clone(), window, cx)
            }))
    };

    let menu = menu
        .item(checkout)
        .item(worktree)
        .separator()
        .item(merge)
        .item(rebase)
        .separator()
        .item(fast_forward)
        .item(push)
        .item(reset_to_upstream)
        .item(pr_item(
            ctx,
            remote.as_deref().unwrap_or("origin"),
            &forge_branch,
            b.pr.as_ref(),
        ));
    let menu = if b.in_stack { menu.item(restack) } else { menu };
    menu.separator().item(rename).item(delete).separator().item(
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
            this.request_delete_remote_branch(remote.clone(), branch.clone(), window, cx)
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

/// A file in a list (working-copy changes, a commit's files).
pub struct FileMenu {
    pub path: String,
    /// Blame as of this commit; the working copy when `None`.
    pub rev: Option<ObjectId>,
    /// Git has lines to blame (not untracked, not deleted).
    pub blamable: bool,
}

pub fn file_menu(
    menu: PopupMenu,
    view: &WeakEntity<RepoView>,
    root: &Path,
    file: &FileMenu,
) -> PopupMenu {
    use super::file_view::FileTab;
    let open = |tab: FileTab| {
        let (path, rev) = (file.path.clone(), file.rev);
        act(view, move |this, _, cx| {
            this.open_file(path.clone(), rev, tab, cx)
        })
    };
    menu.item(
        item("Blame", Lucide::ScanText)
            .disabled(!file.blamable)
            .on_click(open(FileTab::Blame)),
    )
    .item(
        item(
            tr("Historique du fichier", "File history"),
            Lucide::FileClock,
        )
        .on_click(open(FileTab::History)),
    )
    .separator()
    .item(item(tr("Copier le chemin", "Copy path"), Lucide::Copy).on_click(copy(file.path.clone())))
    .item(
        item(
            tr("Copier le chemin complet", "Copy full path"),
            Lucide::Copy,
        )
        .on_click(copy(root.join(&file.path).to_string_lossy().into_owned())),
    )
}

/// A linked worktree as shown in a menu.
#[derive(Clone)]
pub struct WorktreeMenu {
    pub path: String,
    /// Branch, or "(detached)".
    pub label: String,
    pub dead: Option<mugetsu_git::DeadWorktree>,
}

pub fn worktree_menu(menu: PopupMenu, ctx: &MenuContext, wt: &WorktreeMenu) -> PopupMenu {
    let exists = Path::new(&wt.path).exists();
    let open = {
        let path = PathBuf::from(&wt.path);
        item(
            tr("Ouvrir dans un onglet", "Open in a tab"),
            Lucide::FolderOpen,
        )
        .disabled(!exists)
        .on_click(act(&ctx.view, move |_, _, cx| {
            cx.emit(RepoEvent::OpenWorktree(path.clone()))
        }))
    };
    let delete = {
        let wt = wt.clone();
        item(tr("Supprimer le worktree…", "Remove worktree…"), Lucide::Trash).on_click(act(
            &ctx.view,
            move |this, window, cx| {
                let (path, label) = (wt.path.clone(), wt.label.clone());
                let dirty = wt.dead.as_ref().is_some_and(|d| d.dirty);
                let description = match &wt.dead {
                    _ if !Path::new(&path).exists() => tr(
                        "Le dossier n'existe plus : git oublie ce worktree. La branche reste.",
                        "The folder no longer exists: git forgets this worktree. The branch stays.",
                    )
                    .to_string(),
                    Some(d) => {
                        let size = crate::fmt::human_size(d.bytes);
                        if dirty {
                            trf!(
                                "{path} ({size}) est supprimé, modifications non commitées comprises. La branche reste.",
                                "{path} ({size}) is deleted, uncommitted changes included. The branch stays."
                            )
                        } else {
                            trf!(
                                "{path} ({size}) est supprimé. La branche reste.",
                                "{path} ({size}) is deleted. The branch stays."
                            )
                        }
                    }
                    None => trf!(
                        "{path} est supprimé (refusé s'il a des modifications non commitées). La branche reste.",
                        "{path} is deleted (refused if it has uncommitted changes). The branch stays."
                    ),
                };
                let force = wt.dead.is_some();
                this.confirm(
                    trf!(
                        "Supprimer le worktree « {label} » ?",
                        "Remove worktree '{label}'?"
                    ),
                    description,
                    tr("Supprimer", "Remove"),
                    move |this, window, cx| {
                        let path = path.clone();
                        this.run_op(
                            tr("Supprimer worktree", "Remove worktree"),
                            move |r| {
                                if force {
                                    r.worktree_purge(&path)
                                } else {
                                    r.worktree_remove(&path)
                                }
                            },
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
    menu.item(open).separator().item(delete).separator().item(
        item(tr("Copier le chemin", "Copy path"), Lucide::Copy).on_click(copy(wt.path.clone())),
    )
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

    if let Some(head) = ctx.head_branch.clone() {
        let sha = sha.clone();
        menu = menu.item(
            item(
                trf!("Rebaser '{head}' ici", "Rebase '{head}' here"),
                Lucide::GitBranch,
            )
            .disabled(ctx.rebasing || ctx.head == Some(id))
            .on_click(act(&ctx.view, move |this, window, cx| {
                this.rebase_head_onto(sha.clone(), window, cx)
            })),
        );
        menu = menu.item(
            item(
                trf!(
                    "Rebase interactif de '{head}' ici…",
                    "Interactive rebase '{head}' here…"
                ),
                Lucide::ListOrdered,
            )
            .disabled(ctx.rebasing || ctx.head == Some(id))
            .on_click(act(&ctx.view, move |this, window, cx| {
                this.open_rebase(id, window, cx)
            })),
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
    .item(item(tr("Copier le SHA", "Copy SHA"), Lucide::Copy).on_click(copy(sha.clone())))
    .when_some(ctx.github.clone(), |menu, gh| {
        let url = gh.commit_url(&sha);
        menu.item(
            item(
                tr("Copier le lien GitHub", "Copy GitHub Link"),
                Lucide::Link,
            )
            .on_click(act(&ctx.view, move |this, _, cx| {
                if this.selected != Some(id) {
                    this.select_commit(id, false, cx);
                }
                this.copy_link(super::links::LinkSpot::Commit, url.clone(), cx)
            })),
        )
        .item(
            item(
                tr("Ouvrir sur GitHub", "Open on GitHub"),
                Lucide::ExternalLink,
            )
            .on_click({
                let url = gh.commit_url(&sha);
                move |_, _, cx| cx.open_url(&url)
            }),
        )
    })
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
                        crate::toast::error(tr("Patch : échec", "Patch failed"), &e, None),
                        cx,
                    )
                });
            }
        })
        .detach();
    }
}
