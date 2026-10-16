//! ⌘O: pick a repository among the recent ones, those found in the folders
//! set in the settings and those of the connected accounts (cloned on the
//! spot), paste a URL to clone, or browse for another one.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, BoxShadow, Context, Entity,
    Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    linear_color_stop, linear_gradient, point, prelude::FluentBuilder as _, px,
};
use mugetsu_forge::{ForgeKind, RemoteRepo};
use mugetsu_git::{ClonePhase, CloneProgress};

use crate::i18n::tr;
use crate::pr::AccountRepos;
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
type Browse = Rc<dyn Fn(&mut Window, &mut App)>;

/// Remote repositories listed without a query, per account.
const REMOTE_LIMIT: usize = 40;
/// Height of the list (and of the clone view replacing it).
const LIST_H: f32 = 340.;
const BAR_W: f32 = 340.;

fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

/// easeOutBack: a little overshoot before settling.
fn back(t: f32) -> f32 {
    let (c1, t) = (1.70158, t.clamp(0., 1.) - 1.);
    1. + (c1 + 1.) * t.powi(3) + c1 * t.powi(2)
}

/// What gets cloned: a remote repository or a pasted URL.
#[derive(Clone, Debug)]
struct CloneTarget {
    name: String,
    url: String,
}

impl CloneTarget {
    fn remote(repo: &RemoteRepo) -> Self {
        let https = crate::settings::get().clone_over_https();
        Self {
            name: repo.name().to_string(),
            url: if https {
                repo.https_url.clone()
            } else {
                repo.ssh_url.clone()
            },
        }
    }

    fn url(url: &str) -> Option<Self> {
        Some(Self {
            name: mugetsu_git::clone_name(url)?,
            url: url.trim().to_string(),
        })
    }
}

enum CloneState {
    Running,
    Done,
    Failed(String),
}

struct CloneJob {
    id: usize,
    target: CloneTarget,
    dest: PathBuf,
    progress: CloneProgress,
    /// Where the progress bar's current animation starts.
    bar_from: f32,
    bar_seq: usize,
    state: CloneState,
    cancel: Arc<AtomicBool>,
    /// Folder picked for this clone while no workspace folder is set: offered
    /// as one once the clone is done.
    offer: Option<PathBuf>,
}

/// The workspace folder's copy of `name`, when it is already there.
fn local_clone(name: &str) -> Option<PathBuf> {
    let dir = crate::settings::get().clone_dir?.join(name);
    dir.join(".git").exists().then_some(dir)
}

/// `parent/name`, or `name-2`, `name-3`… when taken.
fn free_dest(parent: &Path, name: &str) -> PathBuf {
    let first = parent.join(name);
    if !first.exists() {
        return first;
    }
    (2..1000)
        .map(|i| parent.join(format!("{name}-{i}")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// Repositories of one account matching the query.
struct RemoteGroup {
    kind: ForgeKind,
    title: String,
    repos: Vec<RemoteRepo>,
    hidden: usize,
    error: Option<String>,
}

struct Visible {
    recent: Vec<PathBuf>,
    found: Vec<PathBuf>,
    /// The query is a URL to clone.
    url: Option<CloneTarget>,
    remote: Vec<RemoteGroup>,
}

pub struct OpenRepoForm {
    input: Entity<InputState>,
    recent: Vec<PathBuf>,
    found: Vec<PathBuf>,
    /// `None` while the first listing loads.
    remote: Option<Vec<AccountRepos>>,
    clone: Option<CloneJob>,
    next_job: usize,
    pick: Pick,
    _input: Subscription,
}

impl OpenRepoForm {
    fn visible(&self, cx: &App) -> Visible {
        let query = self.input.read(cx).value().trim().to_string();
        let url = mugetsu_git::is_clone_url(&query)
            .then(|| CloneTarget::url(&query))
            .flatten();
        let query = if url.is_some() { String::new() } else { query };
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
        let remote = if url.is_some() {
            Vec::new()
        } else {
            self.remote
                .iter()
                .flatten()
                .map(|account| {
                    let all: Vec<RemoteRepo> = account
                        .repos
                        .as_ref()
                        .map(|list| {
                            list.iter()
                                .filter(|r| matches(Path::new(&r.full_name), &query))
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default();
                    let limit = if query.is_empty() {
                        REMOTE_LIMIT
                    } else {
                        REMOTE_LIMIT * 3
                    };
                    let hidden = all.len().saturating_sub(limit);
                    let mut title = account.kind.label().to_string();
                    if !matches!(
                        account.host.as_str(),
                        "github.com" | "gitlab.com" | "bitbucket.org"
                    ) {
                        title = format!("{title} ({})", account.host);
                    }
                    if let Some(login) = &account.login {
                        title = format!("{title} · {login}");
                    }
                    RemoteGroup {
                        kind: account.kind,
                        title,
                        repos: all.into_iter().take(limit).collect(),
                        hidden,
                        error: account.repos.as_ref().err().cloned(),
                    }
                })
                .filter(|g| !g.repos.is_empty() || (g.error.is_some() && query.is_empty()))
                .collect()
        };
        Visible {
            recent,
            found,
            url,
            remote,
        }
    }

    /// Enter: clone the pasted URL, else the first local match, else the
    /// first remote one.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.clone.is_some() {
            return;
        }
        let v = self.visible(cx);
        if let Some(target) = v.url {
            self.start_clone(target, window, cx);
        } else if let Some(first) = v.recent.into_iter().chain(v.found).next() {
            window.close_dialog(cx);
            (self.pick)(first, window, cx);
        } else if let Some(repo) = v.remote.iter().flat_map(|g| g.repos.first()).next() {
            let repo = repo.clone();
            self.pick_remote(&repo, window, cx);
        }
    }

    /// Opens the workspace folder's copy when there is one, else clones.
    fn pick_remote(&mut self, repo: &RemoteRepo, window: &mut Window, cx: &mut Context<Self>) {
        match local_clone(repo.name()) {
            Some(path) => {
                window.close_dialog(cx);
                (self.pick)(path, window, cx);
            }
            None => self.start_clone(CloneTarget::remote(repo), window, cx),
        }
    }

    /// Clones into the workspace folder, or asks where first.
    fn start_clone(&mut self, target: CloneTarget, window: &mut Window, cx: &mut Context<Self>) {
        let settings = crate::settings::get();
        if let Some(dir) = settings.clone_dir.clone()
            && !settings.clone_ask
        {
            let dest = free_dest(&dir, &target.name);
            self.run_clone(target, dest, None, window, cx);
            return;
        }
        let offer = settings.clone_dir.is_none();
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Cloner ici", "Clone here").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(parent) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    let dest = free_dest(&parent, &target.name);
                    this.run_clone(target, dest, offer.then_some(parent), window, cx);
                });
            }
        })
        .detach();
    }

    fn run_clone(
        &mut self,
        target: CloneTarget,
        dest: PathBuf,
        offer: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.next_job += 1;
        let id = self.next_job;
        let cancel = Arc::new(AtomicBool::new(false));
        self.clone = Some(CloneJob {
            id,
            target: target.clone(),
            dest: dest.clone(),
            progress: CloneProgress::START,
            bar_from: 0.,
            bar_seq: 0,
            state: CloneState::Running,
            cancel: cancel.clone(),
            offer,
        });
        cx.notify();

        type Shared = Arc<Mutex<(CloneProgress, Option<Result<(), String>>)>>;
        let shared: Shared = Arc::new(Mutex::new((CloneProgress::START, None)));
        let (bg, bg_cancel, url, bg_dest) =
            (shared.clone(), cancel.clone(), target.url, dest.clone());
        cx.background_spawn(async move {
            let result = mugetsu_git::clone(&url, &bg_dest, &bg_cancel, |p| {
                bg.lock().unwrap().0 = p;
            });
            bg.lock().unwrap().1 = Some(result.map_err(|e| e.to_string()));
        })
        .detach();

        let pick = self.pick.clone();
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(80))
                    .await;
                let (progress, result) = {
                    let guard = shared.lock().unwrap();
                    (guard.0, guard.1.clone())
                };
                let _ = this.update(cx, |f, cx| f.on_progress(id, progress, cx));
                let Some(result) = result else { continue };
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                match result {
                    Ok(()) => {
                        // Waits for an answer to the workspace folder offer.
                        let stay = this
                            .update(cx, |f, cx| f.on_cloned(id, cx))
                            .unwrap_or(false);
                        if stay {
                            break;
                        }
                        let alive = this.upgrade().is_some();
                        if alive {
                            // Lets the finished state show.
                            cx.background_executor()
                                .timer(Duration::from_millis(650))
                                .await;
                        }
                        let _ = handle.update(cx, |_, window, cx| {
                            if this.upgrade().is_some() {
                                window.close_dialog(cx);
                            }
                            pick(dest, window, cx);
                        });
                    }
                    Err(e) => {
                        let _ = this.update(cx, |f, cx| {
                            if let Some(job) = f.clone.as_mut().filter(|j| j.id == id) {
                                job.state = CloneState::Failed(e);
                                cx.notify();
                            }
                        });
                    }
                }
                break;
            }
        })
        .detach();
    }

    fn on_progress(&mut self, id: usize, progress: CloneProgress, cx: &mut Context<Self>) {
        let Some(job) = self.clone.as_mut().filter(|j| j.id == id) else {
            return;
        };
        if job.progress != progress {
            if progress.overall() - job.progress.overall() > 0.004 {
                job.bar_from = job.progress.overall();
                job.bar_seq += 1;
            }
            job.progress = progress;
            cx.notify();
        }
    }

    /// True when the view stays up to ask about the workspace folder.
    fn on_cloned(&mut self, id: usize, cx: &mut Context<Self>) -> bool {
        let Some(job) = self.clone.as_mut().filter(|j| j.id == id) else {
            return false;
        };
        job.bar_from = job.progress.overall();
        job.bar_seq += 1;
        job.progress = CloneProgress {
            phase: ClonePhase::Checkout,
            percent: 100,
        };
        job.state = CloneState::Done;
        cx.notify();
        job.offer.is_some()
    }

    fn cancel_clone(&mut self, cx: &mut Context<Self>) {
        if let Some(job) = self.clone.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        cx.notify();
    }

    /// Answer to the offer: optionally remember the folder, then open.
    fn open_cloned(&mut self, remember: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(job) = self.clone.take() else { return };
        if remember && let Some(dir) = job.offer {
            crate::settings::update(cx, |s| {
                s.clone_dir = Some(dir);
                s.clone_ask = false;
            });
        }
        window.close_dialog(cx);
        (self.pick)(job.dest, window, cx);
    }

    pub fn cloning(&self) -> bool {
        self.clone.is_some()
    }

    fn render_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let v = self.visible(cx);
        let title = |text: SharedString, icon: Option<Lucide>| {
            h_flex()
                .pt_2()
                .pb_1()
                .px_2()
                .gap_1p5()
                .text_xs()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(theme::muted())
                .when_some(icon, |s, icon| s.child(Icon::new(icon).xsmall()))
                .child(text)
        };
        let mut i = 0;
        let mut rows = |list: &[PathBuf]| {
            list.iter()
                .map(|p| {
                    i += 1;
                    repo_row(i - 1, p, self.pick.clone()).into_any_element()
                })
                .collect::<Vec<_>>()
        };
        let recent_rows = rows(&v.recent);
        let found_rows = rows(&v.found);
        let loading = self.remote.is_none();
        let no_account = self.remote.as_ref().is_some_and(|r| r.is_empty());
        let empty = recent_rows.is_empty()
            && found_rows.is_empty()
            && v.remote.is_empty()
            && v.url.is_none()
            && !loading;
        let mut list = v_flex()
            .id("open-repo-list")
            .h(px(LIST_H))
            .overflow_y_scroll();
        if let Some(target) = v.url {
            list = list
                .child(title(tr("Cloner", "Clone").into(), None))
                .child(url_row(&target, cx));
        }
        list = list
            .when(!recent_rows.is_empty(), |s| {
                s.child(title(tr("Récents", "Recent").into(), None))
                    .children(recent_rows)
            })
            .when(!found_rows.is_empty(), |s| {
                s.child(title(
                    tr("Dans vos dossiers", "In your folders").into(),
                    None,
                ))
                .children(found_rows)
            });
        let mut n = 0;
        for group in v.remote {
            let icon = match group.kind {
                ForgeKind::Github => Lucide::Github,
                _ => Lucide::Cloud,
            };
            list = list.child(title(group.title.into(), Some(icon)));
            if let Some(err) = group.error {
                list = list.child(
                    h_flex()
                        .px_2()
                        .gap_2()
                        .h(px(26.))
                        .text_xs()
                        .text_color(theme::red())
                        .child(Icon::new(Lucide::CircleAlert).xsmall())
                        .child(div().flex_1().min_w_0().truncate().child(err)),
                );
            }
            for repo in &group.repos {
                list = list.child(remote_row(n, repo, cx));
                n += 1;
            }
            if group.hidden > 0 {
                list = list.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(theme::muted())
                        .child(crate::trf!(
                            "… et {} autres : tapez pour filtrer",
                            "… and {} more: type to filter",
                            group.hidden
                        )),
                );
            }
        }
        if loading {
            list = list
                .child(title(
                    tr("Dépôts distants", "Remote repositories").into(),
                    Some(Lucide::Cloud),
                ))
                .children((0..3).map(skeleton_row));
        } else if no_account {
            list = list.child(
                h_flex()
                    .mt_2()
                    .mx_2()
                    .p_3()
                    .gap_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme::hairline())
                    .bg(theme::alpha(gpui_kit::white(), 0.02))
                    .child(
                        Icon::new(Lucide::CloudDownload)
                            .small()
                            .text_color(theme::alpha(theme::accent(), 0.8)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .whitespace_normal()
                            .text_xs()
                            .text_color(theme::muted())
                            .child(tr(
                                "Collez l'URL d'un dépôt pour le cloner. Avec un compte GitHub, GitLab ou Bitbucket (Réglages › Comptes), vos dépôts distants s'affichent ici.",
                                "Paste a repository URL to clone it. With a GitHub, GitLab or Bitbucket account (Settings › Accounts), your remote repositories show up here.",
                            )),
                    ),
            );
        }
        if empty {
            list = list.child(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(theme::muted())
                    .child(tr("Aucun dépôt trouvé", "No repository found")),
            );
        }
        list.into_any_element()
    }

    fn render_clone(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(job) = &self.clone else {
            return div().into_any_element();
        };
        let id = job.id;
        let accent = theme::accent();
        let overall = job.progress.overall();
        let (from, seq) = (job.bar_from, job.bar_seq);
        let failed = matches!(job.state, CloneState::Failed(_));
        let done = matches!(job.state, CloneState::Done);

        // Emblem: download cloud with a halo, a check once done, alert on error.
        let halo = |i: usize| {
            div()
                .absolute()
                .rounded_full()
                .border_1()
                .border_color(theme::alpha(accent, 0.6))
                .with_animation(
                    ("clone-halo", id * 10 + i),
                    Animation::new(Duration::from_millis(1800)).repeat(),
                    move |el, t| {
                        let t = (t + i as f32 * 0.5) % 1.;
                        let size = 56. + 40. * ease_out(t);
                        el.size(px(size))
                            .left(px((96. - size) / 2.))
                            .top(px((96. - size) / 2.))
                            .opacity(0.7 * (1. - t))
                    },
                )
        };
        let emblem = div()
            .relative()
            .size(px(96.))
            .flex()
            .items_center()
            .justify_center()
            .when(!failed && !done, |s| s.child(halo(0)).child(halo(1)))
            .child(
                div()
                    .size(px(56.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::alpha(
                        if failed {
                            theme::red()
                        } else if done {
                            theme::lime()
                        } else {
                            accent
                        },
                        0.14,
                    ))
                    .border_1()
                    .border_color(theme::alpha(
                        if failed {
                            theme::red()
                        } else if done {
                            theme::lime()
                        } else {
                            accent
                        },
                        0.45,
                    ))
                    .shadow(vec![BoxShadow {
                        color: theme::alpha(
                            if failed {
                                theme::red()
                            } else if done {
                                theme::lime()
                            } else {
                                accent
                            },
                            0.3,
                        ),
                        offset: point(px(0.), px(0.)),
                        blur_radius: px(24.),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                    .child(if done {
                        Icon::new(Lucide::Check)
                            .text_color(theme::lime())
                            .with_animation(
                                ("clone-done", id),
                                Animation::new(Duration::from_millis(520)),
                                |el, t| el.size(px(26. * back(t).max(0.))),
                            )
                            .into_any_element()
                    } else if failed {
                        Icon::new(Lucide::CircleAlert)
                            .size(px(24.))
                            .text_color(theme::red())
                            .into_any_element()
                    } else {
                        Icon::new(Lucide::CloudDownload)
                            .size(px(24.))
                            .text_color(accent)
                            .with_animation(
                                ("clone-bob", id),
                                Animation::new(Duration::from_millis(1400)).repeat(),
                                |el, t| {
                                    let k = (t * std::f32::consts::TAU).sin();
                                    el.mt(px(3. * k))
                                },
                            )
                            .into_any_element()
                    }),
            );

        let bar = div()
            .relative()
            .w(px(BAR_W))
            .h(px(6.))
            .rounded_full()
            .overflow_hidden()
            .bg(theme::alpha(gpui_kit::white(), 0.07))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .h_full()
                    .rounded_full()
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(
                            theme::alpha(if done { theme::lime() } else { accent }, 0.55),
                            0.,
                        ),
                        linear_color_stop(if done { theme::lime() } else { accent }, 1.),
                    ))
                    .with_animation(
                        ("clone-bar", id * 100_000 + seq),
                        Animation::new(Duration::from_millis(320)),
                        move |el, t| {
                            let k = from + (overall - from) * ease_out(t);
                            el.w(px(BAR_W * k.max(0.015)))
                        },
                    ),
            )
            .when(!failed && !done, |s| {
                // Light sweeping across the track.
                s.child(
                    div()
                        .absolute()
                        .top_0()
                        .h_full()
                        .w(px(70.))
                        .bg(linear_gradient(
                            90.,
                            linear_color_stop(theme::alpha(gpui_kit::white(), 0.), 0.),
                            linear_color_stop(theme::alpha(gpui_kit::white(), 0.22), 1.),
                        ))
                        .with_animation(
                            ("clone-sweep", id),
                            Animation::new(Duration::from_millis(1500)).repeat(),
                            |el, t| el.left(px(-70. + (BAR_W + 70.) * t)),
                        ),
                )
            });

        let status: SharedString = match &job.state {
            CloneState::Running => format!(
                "{} · {} %",
                phase_label(job.progress.phase),
                (overall * 100.).round() as u32
            )
            .into(),
            CloneState::Done => tr("Cloné", "Cloned").into(),
            CloneState::Failed(_) => tr("Le clonage a échoué", "Cloning failed").into(),
        };

        let actions: AnyElement = match &job.state {
            CloneState::Running => Button::new("clone-cancel")
                .ghost()
                .small()
                .label(tr("Annuler le clonage", "Cancel clone"))
                .on_click(cx.listener(|this, _, _, cx| this.cancel_clone(cx)))
                .into_any_element(),
            CloneState::Failed(err) => v_flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .max_w(px(460.))
                        .max_h(px(64.))
                        .overflow_hidden()
                        .whitespace_normal()
                        .text_xs()
                        .text_center()
                        .text_color(theme::red())
                        .child(err.clone()),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("clone-back")
                                .ghost()
                                .small()
                                .icon(Icon::new(Lucide::ArrowLeft))
                                .label(tr("Retour", "Back"))
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_clone(cx))),
                        )
                        .child(
                            Button::new("clone-retry")
                                .outline()
                                .small()
                                .icon(Icon::new(Lucide::RotateCcw))
                                .label(tr("Réessayer", "Retry"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if let Some(job) = this.clone.take() {
                                        this.run_clone(job.target, job.dest, job.offer, window, cx);
                                    }
                                })),
                        ),
                )
                .into_any_element(),
            CloneState::Done => match &job.offer {
                Some(dir) => v_flex()
                    .items_center()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme::alpha(accent, 0.3))
                    .bg(theme::alpha(accent, 0.06))
                    .child(div().text_sm().child(tr(
                        "Cloner toujours ici la prochaine fois ?",
                        "Always clone here next time?",
                    )))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted())
                            .child(dir.display().to_string()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .pt_1()
                            .child(
                                Button::new("clone-offer-no")
                                    .ghost()
                                    .small()
                                    .label(tr("Pas maintenant", "Not now"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_cloned(false, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("clone-offer-yes")
                                    .primary()
                                    .small()
                                    .icon(Icon::new(Lucide::FolderCheck))
                                    .label(tr("Mémoriser ce dossier", "Remember this folder"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_cloned(true, window, cx)
                                    })),
                            ),
                    )
                    .with_animation(
                        ("clone-offer", id),
                        Animation::new(Duration::from_millis(300)),
                        |el, t| {
                            let k = ease_out(t);
                            el.opacity(k).mt(px(8. * (1. - k)))
                        },
                    )
                    .into_any_element(),
                None => div().h(px(28.)).into_any_element(),
            },
        };

        v_flex()
            .id("clone-view")
            .test_support()
            .h(px(LIST_H))
            .items_center()
            .justify_center()
            .gap_3()
            .child(emblem)
            .child(
                v_flex()
                    .items_center()
                    .gap_0p5()
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(job.target.name.clone()),
                    )
                    .child(
                        div()
                            .max_w(px(460.))
                            .truncate()
                            .text_xs()
                            .text_color(theme::muted())
                            .child(job.dest.display().to_string()),
                    ),
            )
            .child(bar)
            .child(
                div()
                    .text_xs()
                    .text_color(if failed { theme::red() } else { theme::muted() })
                    .child(status),
            )
            .child(actions)
            .with_animation(
                ("clone-view", id),
                Animation::new(Duration::from_millis(280)),
                |el, t| {
                    let k = ease_out(t);
                    el.opacity(k).mt(px(12. * (1. - k)))
                },
            )
            .into_any_element()
    }
}

fn phase_label(phase: ClonePhase) -> &'static str {
    match phase {
        ClonePhase::Connecting => tr("Connexion", "Connecting"),
        ClonePhase::Counting => tr("Préparation des objets", "Counting objects"),
        ClonePhase::Compressing => tr("Compression", "Compressing objects"),
        ClonePhase::Receiving => tr("Réception des objets", "Receiving objects"),
        ClonePhase::Resolving => tr("Résolution des deltas", "Resolving deltas"),
        ClonePhase::Checkout => tr("Extraction des fichiers", "Checking out files"),
    }
}

/// Placeholder row while the remote repositories load, breathing.
fn skeleton_row(i: usize) -> impl IntoElement {
    h_flex()
        .h(px(30.))
        .px_2()
        .gap_2()
        .child(
            div()
                .size(px(14.))
                .rounded_sm()
                .bg(theme::alpha(gpui_kit::white(), 0.06)),
        )
        .child(
            div()
                .h(px(9.))
                .w(px([150., 110., 180.][i % 3]))
                .rounded_full()
                .bg(theme::alpha(gpui_kit::white(), 0.06)),
        )
        .child(
            div()
                .h(px(8.))
                .w(px([200., 240., 140.][i % 3]))
                .rounded_full()
                .bg(theme::alpha(gpui_kit::white(), 0.035)),
        )
        .with_animation(
            ("open-remote-skeleton", i),
            Animation::new(Duration::from_millis(1400)).repeat(),
            move |el, t| {
                let wave = ((t + i as f32 * 0.12) * std::f32::consts::TAU).sin();
                el.opacity(0.55 + 0.45 * wave)
            },
        )
}

/// Pill shown on the right of a remote row when hovered.
fn hover_pill(group: SharedString, local: bool) -> impl IntoElement {
    h_flex()
        .flex_none()
        .gap_1()
        .px_2()
        .h(px(20.))
        .rounded_full()
        .text_xs()
        .text_color(theme::accent())
        .bg(theme::alpha(theme::accent(), 0.12))
        .opacity(0.)
        .group_hover(group, |s| s.opacity(1.))
        .child(
            Icon::new(if local {
                Lucide::FolderOpen
            } else {
                Lucide::Download
            })
            .xsmall(),
        )
        .child(if local {
            tr("Ouvrir", "Open")
        } else {
            tr("Cloner", "Clone")
        })
}

fn remote_row(index: usize, repo: &RemoteRepo, cx: &mut Context<OpenRepoForm>) -> AnyElement {
    let group = SharedString::from(format!("open-remote-{index}"));
    let local = local_clone(repo.name()).is_some();
    let (owner, name) = repo
        .full_name
        .rsplit_once('/')
        .map_or((String::new(), repo.full_name.clone()), |(o, n)| {
            (format!("{o}/"), n.to_string())
        });
    let target = repo.clone();
    let delay = (index.min(12) * 18) as f32;
    h_flex()
        .id(group.clone())
        .test_support()
        .group(group.clone())
        .h(px(30.))
        .px_2()
        .gap_2()
        .rounded_md()
        .cursor_pointer()
        .hover(|s| s.bg(theme::raised()))
        .child(
            Icon::new(if local {
                Lucide::FolderGit2
            } else {
                Lucide::CloudDownload
            })
            .xsmall()
            .text_color(if local {
                theme::muted()
            } else {
                theme::alpha(theme::accent(), 0.75)
            }),
        )
        .child(
            h_flex()
                .flex_none()
                .max_w(px(300.))
                .min_w_0()
                .text_sm()
                .child(
                    div()
                        .flex_shrink(1.)
                        .min_w_0()
                        .truncate()
                        .text_color(theme::muted())
                        .child(owner),
                )
                .child(div().flex_none().child(name)),
        )
        .when(repo.private, |s| {
            s.child(
                Icon::new(Lucide::Lock)
                    .xsmall()
                    .flex_none()
                    .text_color(theme::muted()),
            )
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(theme::muted())
                .child(repo.description.clone()),
        )
        .child(hover_pill(group, local))
        .on_click(cx.listener(move |this, _, window, cx| this.pick_remote(&target, window, cx)))
        .with_animation(
            ("open-remote-in", index),
            Animation::new(Duration::from_millis(220 + delay as u64)),
            move |el, t| {
                let total = 220. + delay;
                let k = ease_out(((t * total) - delay) / 220.);
                el.opacity(k).ml(px(8. * (1. - k)))
            },
        )
        .into_any_element()
}

fn url_row(target: &CloneTarget, cx: &mut Context<OpenRepoForm>) -> AnyElement {
    let group = SharedString::from("open-url");
    let t = target.clone();
    h_flex()
        .id("open-url")
        .test_support()
        .group(group.clone())
        .h(px(34.))
        .px_2()
        .gap_2()
        .rounded_md()
        .cursor_pointer()
        .bg(theme::alpha(theme::accent(), 0.08))
        .border_1()
        .border_color(theme::alpha(theme::accent(), 0.25))
        .hover(|s| s.bg(theme::alpha(theme::accent(), 0.14)))
        .child(Icon::new(Lucide::Link).xsmall().text_color(theme::accent()))
        .child(div().flex_none().text_sm().child(target.name.clone()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(theme::muted())
                .child(target.url.clone()),
        )
        .child(
            h_flex()
                .flex_none()
                .gap_1()
                .text_xs()
                .text_color(theme::accent())
                .child(Icon::new(Lucide::CornerDownLeft).xsmall())
                .child(tr("Cloner", "Clone")),
        )
        .on_click(cx.listener(move |this, _, window, cx| this.start_clone(t.clone(), window, cx)))
        .with_animation(
            "open-url-in",
            Animation::new(Duration::from_millis(220)),
            |el, t| {
                let k = ease_out(t);
                el.opacity(k).mt(px(4. * (1. - k)))
            },
        )
        .into_any_element()
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
            Some(icon) => div()
                .w(px(16.))
                .child(crate::repo_icon::view(&icon, 15.))
                .into_any_element(),
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
        let cloning = self.clone.is_some();
        v_flex()
            .gap_2()
            .child(
                div().when(cloning, |s| s.opacity(0.4)).child(
                    Input::new(&self.input)
                        .id("open-repo-search")
                        .disabled(cloning)
                        .prefix(
                            Icon::new(Lucide::Search)
                                .xsmall()
                                .text_color(theme::muted()),
                        ),
                ),
            )
            .child(if cloning {
                self.render_clone(cx)
            } else {
                self.render_list(cx)
            })
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
        InputState::new(window, cx).placeholder(tr(
            "Rechercher un dépôt, ou coller une URL à cloner",
            "Search a repository, or paste a URL to clone",
        ))
    });
    let form = cx.new(|cx: &mut Context<OpenRepoForm>| {
        let sub = cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify());
        OpenRepoForm {
            input: input.clone(),
            recent: settings.recent.clone(),
            found: Vec::new(),
            remote: crate::pr::cached_remote_repos(),
            clone: None,
            next_job: 0,
            pick: pick.clone(),
            _input: sub,
        }
    });
    // Scanning touches the disk, listing remote repositories the network:
    // both in the background.
    let folders = settings.scan_folders();
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
    // Tests stay off the network.
    if cfg!(test) {
        form.update(cx, |f, _| f.remote = Some(Vec::new()));
    } else {
        let remote = cx.background_spawn(async move { crate::pr::remote_repos() });
        let weak = form.downgrade();
        cx.spawn(async move |cx| {
            let list = remote.await;
            let _ = weak.update(cx, |f, cx| {
                f.remote = Some(list);
                cx.notify();
            });
        })
        .detach();
    }

    let browse: Browse = Rc::new(browse);
    let focus_once = std::cell::Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if focus_once.replace(false) {
            input.focus_handle(cx).focus(window, cx);
        }
        let browse = browse.clone();
        let cloning = form.read(cx).cloning();
        dialog
            .bg(crate::theme::sheet())
            .title(tr("Ouvrir un dépôt", "Open Repository"))
            .w(px(640.))
            .child(form.clone())
            // Enter: the dialog's confirm action.
            .on_ok({
                let form = form.clone();
                move |_, window, cx| {
                    form.update(cx, |f, cx| f.confirm(window, cx));
                    false
                }
            })
            .footer(
                DialogFooter::new()
                    .when(!cloning, |f| {
                        f.child(
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
                    })
                    .child(div().flex_1())
                    .child(DialogClose::new().trigger(|b| {
                        b.ghost().label(if cloning {
                            tr("Fermer", "Close")
                        } else {
                            tr("Annuler", "Cancel")
                        })
                    })),
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

    #[test]
    fn clones_never_overwrite_a_folder() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(free_dest(root.path(), "app"), root.path().join("app"));
        std::fs::create_dir_all(root.path().join("app")).unwrap();
        std::fs::create_dir_all(root.path().join("app-2")).unwrap();
        assert_eq!(free_dest(root.path(), "app"), root.path().join("app-3"));
    }
}
