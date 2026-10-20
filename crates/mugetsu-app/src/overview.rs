//! Overview of the workspace (icon left of the tabs): a card per open
//! repository, listing what needs doing there first (conflicts, branches to
//! pull or push, reviews waiting, failing CI…), then its pulse. Read from
//! what each tab already holds: no git call, no forge request.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, Context, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px, relative,
};

use crate::i18n::tr;
use crate::theme;
use crate::workspace::Workspace;

/// What a repository tab knows, for its card.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RepoCard {
    pub root: PathBuf,
    pub name: String,
    pub branch: String,
    pub dirty: usize,
    pub conflicts: usize,
    /// Merge, rebase… left in progress.
    pub operation: Option<&'static str>,
    /// Current branch against its upstream.
    pub ahead: u32,
    pub behind: u32,
    /// Local branches whose upstream is gone (merged and deleted remotely).
    pub gone: usize,
    pub stashes: usize,
    pub dead_worktrees: usize,
    /// A background fetch found remote changes not shown yet.
    pub updates: bool,
    /// Open PRs waiting for my review.
    pub reviews: usize,
    /// My open PRs: failing CI, changes requested, approved and green.
    pub failing: usize,
    pub changes_requested: usize,
    pub ready: usize,
    pub my_prs: usize,
    /// My open PRs, most pressing first.
    pub mine: Vec<PrLine>,
    /// Open PRs waiting for my review.
    pub to_review: Vec<PrLine>,
    /// Commits per day over the last month (the sidebar's pulse).
    pub pulse: Arc<Vec<u32>>,
}

/// A pull request as the overview lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrLine {
    pub number: u64,
    pub title: String,
    pub mark: PrMark,
}

/// Where a PR stands, most pressing first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PrMark {
    Failing,
    ChangesRequested,
    Ready,
    Pending,
    Open,
    Draft,
}

impl PrMark {
    fn look(self) -> (Lucide, Hsla, &'static str) {
        match self {
            PrMark::Failing => (Lucide::CircleX, theme::red(), tr("CI rouge", "CI failing")),
            PrMark::ChangesRequested => (
                Lucide::MessageSquareWarning,
                theme::amber(),
                tr("changements demandés", "changes requested"),
            ),
            PrMark::Ready => (
                Lucide::GitMerge,
                theme::lime(),
                tr("prête à merger", "ready to merge"),
            ),
            PrMark::Pending => (
                Lucide::CircleDashed,
                theme::amber(),
                tr("CI en cours", "CI running"),
            ),
            PrMark::Open => (Lucide::GitPullRequest, ink2(), tr("ouverte", "open")),
            PrMark::Draft => (
                Lucide::GitPullRequestDraft,
                faint(),
                tr("brouillon", "draft"),
            ),
        }
    }
}

/// Where a signal leads in the repository tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    History,
    Changes,
    Prs,
}

/// Something to know or do in a repository, most pressing first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Signal {
    Conflicts(usize),
    Operation(&'static str),
    Failing(usize),
    ChangesRequested(usize),
    Reviews(usize),
    Behind(u32),
    Ready(usize),
    Ahead(u32),
    Dirty(usize),
    Updates,
    Gone(usize),
    DeadWorktrees(usize),
    Stashes(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Blocks work.
    Alert,
    /// Someone waits on you, or you on them.
    Act,
    /// Worth a look when there's time.
    Note,
}

impl Signal {
    pub fn level(self) -> Level {
        match self {
            Signal::Conflicts(_) | Signal::Operation(_) | Signal::Failing(_) => Level::Alert,
            Signal::ChangesRequested(_)
            | Signal::Reviews(_)
            | Signal::Behind(_)
            | Signal::Ready(_)
            | Signal::Ahead(_) => Level::Act,
            _ => Level::Note,
        }
    }

    pub fn target(self) -> Target {
        match self {
            Signal::Conflicts(_) | Signal::Operation(_) | Signal::Dirty(_) => Target::Changes,
            Signal::Failing(_)
            | Signal::ChangesRequested(_)
            | Signal::Reviews(_)
            | Signal::Ready(_) => Target::Prs,
            _ => Target::History,
        }
    }

    fn icon(self) -> Lucide {
        match self {
            Signal::Conflicts(_) => Lucide::GitMergeConflict,
            Signal::Operation(_) => Lucide::TriangleAlert,
            Signal::Failing(_) => Lucide::CircleX,
            Signal::ChangesRequested(_) => Lucide::MessageSquareWarning,
            Signal::Reviews(_) => Lucide::Eye,
            Signal::Behind(_) => Lucide::ArrowDownToLine,
            Signal::Ready(_) => Lucide::GitMerge,
            Signal::Ahead(_) => Lucide::ArrowUpFromLine,
            Signal::Dirty(_) => Lucide::Pencil,
            Signal::Updates => Lucide::CloudDownload,
            Signal::Gone(_) => Lucide::GitBranch,
            Signal::DeadWorktrees(_) => Lucide::FolderTree,
            Signal::Stashes(_) => Lucide::Archive,
        }
    }

    pub fn text(self) -> String {
        match self {
            Signal::Conflicts(n) => trf!("{n} fichier(s) en conflit", "{n} conflicted file(s)"),
            Signal::Operation(op) => trf!("{op} en cours", "{op} in progress"),
            Signal::Failing(n) => trf!(
                "CI rouge sur {n} de tes PR",
                "CI failing on {n} of your PRs"
            ),
            Signal::ChangesRequested(n) => trf!(
                "Changements demandés sur {n} PR",
                "Changes requested on {n} PR(s)"
            ),
            Signal::Reviews(n) => trf!("{n} PR attendent ta review", "{n} PR(s) await your review"),
            Signal::Behind(n) => trf!("{n} commit(s) à récupérer", "{n} commit(s) to pull"),
            Signal::Ready(n) => trf!("{n} PR prête(s) à merger", "{n} PR(s) ready to merge"),
            Signal::Ahead(n) => trf!("{n} commit(s) à pousser", "{n} commit(s) to push"),
            Signal::Dirty(n) => trf!("{n} fichier(s) non commité(s)", "{n} uncommitted file(s)"),
            Signal::Updates => tr("Nouveautés sur le remote", "New changes on the remote").into(),
            Signal::Gone(n) => trf!(
                "{n} branche(s) supprimée(s) sur le remote",
                "{n} branch(es) deleted on the remote"
            ),
            Signal::DeadWorktrees(n) => {
                trf!("{n} worktree(s) terminé(s)", "{n} finished worktree(s)")
            }
            Signal::Stashes(n) => trf!("{n} stash(s) en attente", "{n} stash(es) waiting"),
        }
    }
}

impl RepoCard {
    /// What to know here, most pressing first.
    pub fn signals(&self) -> Vec<Signal> {
        let mut out: Vec<Signal> = [
            (self.conflicts > 0).then_some(Signal::Conflicts(self.conflicts)),
            self.operation.map(Signal::Operation),
            (self.failing > 0).then_some(Signal::Failing(self.failing)),
            (self.changes_requested > 0)
                .then_some(Signal::ChangesRequested(self.changes_requested)),
            (self.reviews > 0).then_some(Signal::Reviews(self.reviews)),
            (self.behind > 0).then_some(Signal::Behind(self.behind)),
            (self.ready > 0).then_some(Signal::Ready(self.ready)),
            (self.ahead > 0).then_some(Signal::Ahead(self.ahead)),
            // Conflicted files are already counted above.
            (self.dirty > self.conflicts).then_some(Signal::Dirty(self.dirty - self.conflicts)),
            self.updates.then_some(Signal::Updates),
            (self.gone > 0).then_some(Signal::Gone(self.gone)),
            (self.dead_worktrees > 0).then_some(Signal::DeadWorktrees(self.dead_worktrees)),
            (self.stashes > 0).then_some(Signal::Stashes(self.stashes)),
        ]
        .into_iter()
        .flatten()
        .collect();
        out.sort();
        out
    }
}

fn level_color(level: Level) -> Hsla {
    match level {
        Level::Alert => theme::red(),
        Level::Act => theme::amber(),
        Level::Note => theme::muted(),
    }
}

fn level_title(level: Level) -> &'static str {
    match level {
        Level::Alert => tr("Bloquant", "Blocking"),
        Level::Act => tr("À traiter", "To act on"),
        Level::Note => tr("À noter", "Worth a look"),
    }
}

fn ink() -> Hsla {
    theme::alpha(gpui_kit::white(), 0.93)
}
fn ink2() -> Hsla {
    theme::alpha(gpui_kit::white(), 0.66)
}
fn faint() -> Hsla {
    theme::alpha(gpui_kit::white(), 0.42)
}

/// Repositories per row (workspaces rarely hold more).
const PER_ROW: usize = 4;

/// The whole window below the title bar: header, a tile per repository,
/// then everything to do across them and the pull requests.
pub fn render(cards: &[RepoCard], workspace: &str, cx: &mut Context<Workspace>) -> AnyElement {
    let signals: Vec<Vec<Signal>> = cards.iter().map(RepoCard::signals).collect();
    let count = |level: Level| {
        signals
            .iter()
            .flatten()
            .filter(|s| s.level() == level)
            .count()
    };
    let (alerts, acts) = (count(Level::Alert), count(Level::Act));
    let calm = signals.iter().filter(|s| s.is_empty()).count();
    let chip = |color: Hsla, text: String| {
        h_flex()
            .gap_1p5()
            .px_2p5()
            .py_0p5()
            .rounded_full()
            .border_1()
            .border_color(theme::hairline())
            .bg(theme::alpha(gpui_kit::white(), 0.03))
            .text_xs()
            .text_color(ink2())
            .child(div().size(px(6.)).rounded_full().bg(color))
            .child(text)
    };
    let chips = h_flex()
        .id("overview-summary")
        .test_support()
        .gap_2()
        .when(cards.is_empty(), |s| {
            s.child(div().text_sm().text_color(faint()).child(tr(
                "Aucun dépôt ouvert dans ce workspace.",
                "No repository open in this workspace.",
            )))
        })
        .when(alerts > 0, |s| {
            s.child(chip(
                theme::red(),
                trf!("{alerts} bloquant(s)", "{alerts} blocking"),
            ))
        })
        .when(acts > 0, |s| {
            s.child(chip(
                theme::amber(),
                trf!("{acts} à traiter", "{acts} to act on"),
            ))
        })
        .when(calm > 0, |s| {
            s.child(chip(theme::lime(), trf!("{calm} au calme", "{calm} calm")))
        });
    let header = h_flex()
        .w_full()
        .px_8()
        .pt_6()
        .pb_5()
        .gap_4()
        .items_center()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .items_baseline()
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(ink())
                                .child(workspace.to_string()),
                        )
                        .child(div().text_sm().text_color(faint()).child(trf!(
                            "{} dépôt(s)",
                            "{} repositories",
                            cards.len()
                        ))),
                )
                .child(chips),
        )
        .child(
            Button::new("overview-recap")
                .primary()
                .icon(Icon::new(Lucide::Sparkles))
                .label(tr("Recap de la semaine", "Weekly recap"))
                .disabled(cards.is_empty())
                .on_click(cx.listener(|ws, _, window, cx| ws.open_recap(window, cx))),
        );

    // Tiles: equal columns filling the width, PER_ROW at most per row.
    let mut rows: Vec<AnyElement> = Vec::new();
    for (r, chunk) in cards.chunks(PER_ROW).enumerate() {
        let mut row = h_flex().w_full().gap_4().items_stretch();
        for (j, card) in chunk.iter().enumerate() {
            let i = r * PER_ROW + j;
            row = row.child(tile(i, card, &signals[i], cx));
        }
        // Keep columns aligned on a short last row.
        for _ in chunk.len()..PER_ROW.min(cards.len()) {
            row = row.child(div().flex_1());
        }
        rows.push(row.into_any_element());
    }

    let activity = activity_panel(cards);
    let todo = todo_panel(cards, &signals, cx);
    let prs = prs_panel(cards, cx);
    v_flex()
        .id("overview")
        .test_support()
        .size_full()
        .overflow_y_scroll()
        .child(header)
        .child(
            v_flex()
                .w_full()
                .px_8()
                .pb_8()
                .gap_4()
                .children(rows)
                .children(activity)
                .child(
                    h_flex()
                        .w_full()
                        .gap_4()
                        .items_stretch()
                        .child(v_flex().flex_1().min_w_0().child(todo))
                        .child(v_flex().flex_1().min_w_0().child(prs)),
                ),
        )
        .into_any_element()
}

/// A glass panel with a title row, like the Stats view's.
fn panel(
    id: &'static str,
    icon: Lucide,
    title: &'static str,
    count: usize,
    body: impl IntoElement,
) -> AnyElement {
    v_flex()
        .id(id)
        .test_support()
        .w_full()
        .flex_1()
        .min_h(px(260.))
        .relative()
        .rounded_xl()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::alpha(gpui_kit::white(), 0.028))
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .top_0()
                .left_4()
                .right_4()
                .h(px(1.))
                .bg(theme::alpha(gpui_kit::white(), 0.09)),
        )
        .child(
            h_flex()
                .px_4()
                .pt_3()
                .pb_2()
                .gap_2()
                .child(Icon::new(icon).small().text_color(faint()))
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(ink())
                        .child(title),
                )
                .when(count > 0, |s| {
                    s.child(div().text_xs().text_color(faint()).child(count.to_string()))
                }),
        )
        .child(body)
        .into_any_element()
}

fn repo_badge(card: &RepoCard, size: f32) -> AnyElement {
    let emoji = crate::settings::get().repo_icons.get(&card.root).cloned();
    let initial = card
        .name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    div()
        .flex_none()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded_lg()
        .bg(theme::alpha(gpui_kit::white(), 0.06))
        .child(match emoji {
            Some(e) => crate::repo_icon::view(&e, size * 0.6),
            None => div()
                .text_sm()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(ink2())
                .child(initial)
                .into_any_element(),
        })
        .into_any_element()
}

/// One repository: name, branch and sync state, its month of commits, a
/// health pill. Clicking opens its tab.
fn tile(
    index: usize,
    card: &RepoCard,
    signals: &[Signal],
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let worst = signals.first().map(|s| s.level());
    let (pill_color, pill) = match worst {
        Some(Level::Alert) => (theme::red(), tr("Bloqué", "Blocked")),
        Some(Level::Act) => (theme::amber(), tr("À traiter", "To act on")),
        _ => (theme::lime(), tr("À jour", "Up to date")),
    };
    let stat = |icon: Lucide, value: String, color: Hsla| {
        h_flex()
            .gap_1()
            .text_xs()
            .text_color(color)
            .child(Icon::new(icon).xsmall())
            .child(value)
    };
    let commits: u32 = card.pulse.iter().sum();
    v_flex()
        .id(SharedString::from(format!("overview-card-{}", card.name)))
        .test_support()
        .flex_1()
        .min_w_0()
        .relative()
        .p_4()
        .gap_3()
        .rounded_xl()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::alpha(gpui_kit::white(), 0.028))
        .overflow_hidden()
        .cursor_pointer()
        .hover(|s| {
            s.bg(theme::alpha(gpui_kit::white(), 0.05))
                .border_color(theme::glass_edge())
        })
        // Health on the top rim.
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(2.))
                .bg(theme::alpha(pill_color, 0.7)),
        )
        .child(
            h_flex()
                .gap_2p5()
                .child(repo_badge(card, 32.))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(ink())
                                .child(card.name.clone()),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .text_xs()
                                .text_color(faint())
                                .child(Icon::new(Lucide::GitBranch).xsmall())
                                .child(div().min_w_0().truncate().child(card.branch.clone())),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .px_2()
                        .py_0p5()
                        .rounded_full()
                        .bg(theme::alpha(pill_color, 0.14))
                        .text_xs()
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(pill_color)
                        .child(pill),
                ),
        )
        .child(pulse(index, &card.pulse))
        .child(
            h_flex()
                .gap_3()
                .flex_wrap()
                .child(
                    div()
                        .text_xs()
                        .text_color(faint())
                        .child(trf!("{commits} commits / 30 j", "{commits} commits / 30 d")),
                )
                .child(div().flex_1())
                .when(card.behind > 0, |s| {
                    s.child(stat(
                        Lucide::ArrowDownToLine,
                        card.behind.to_string(),
                        theme::amber(),
                    ))
                })
                .when(card.ahead > 0, |s| {
                    s.child(stat(
                        Lucide::ArrowUpFromLine,
                        card.ahead.to_string(),
                        ink2(),
                    ))
                })
                .when(card.dirty > 0, |s| {
                    s.child(stat(Lucide::Pencil, card.dirty.to_string(), theme::amber()))
                })
                .when(card.my_prs > 0, |s| {
                    s.child(stat(
                        Lucide::GitPullRequest,
                        card.my_prs.to_string(),
                        ink2(),
                    ))
                }),
        )
        .on_click(cx.listener(move |ws, _, _, cx| ws.open_from_overview(index, None, cx)))
        .into_any_element()
}

/// Everything to do across the repositories, grouped by urgency.
fn todo_panel(
    cards: &[RepoCard],
    signals: &[Vec<Signal>],
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let total: usize = signals.iter().map(Vec::len).sum();
    let mut body = v_flex().px_2().pb_2();
    if total == 0 {
        body = body.child(empty(
            Lucide::CircleCheck,
            theme::lime(),
            tr("Rien à faire. Bon code !", "Nothing to do. Happy coding!"),
        ));
    }
    for level in [Level::Alert, Level::Act, Level::Note] {
        let items: Vec<(usize, usize, Signal)> = signals
            .iter()
            .enumerate()
            .flat_map(|(i, list)| {
                list.iter()
                    .enumerate()
                    .filter(|(_, s)| s.level() == level)
                    .map(move |(j, s)| (i, j, *s))
            })
            .collect();
        if items.is_empty() {
            continue;
        }
        body = body.child(
            div()
                .px_2()
                .pt_2()
                .pb_1()
                .text_xs()
                .font_weight(gpui_kit::FontWeight::MEDIUM)
                .text_color(level_color(level))
                .child(level_title(level)),
        );
        for (i, j, s) in items {
            let target = s.target();
            body = body.child(
                h_flex()
                    .id(SharedString::from(format!("overview-signal-{i}-{j}")))
                    .test_support()
                    .gap_2p5()
                    .px_2()
                    .py_1p5()
                    .rounded_md()
                    .text_sm()
                    .text_color(ink2())
                    .cursor_pointer()
                    .hover(|st| st.bg(theme::raised()).text_color(ink()))
                    .child(Icon::new(s.icon()).small().text_color(level_color(level)))
                    .child(div().flex_1().min_w_0().truncate().child(s.text()))
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(160.))
                            .truncate()
                            .px_1p5()
                            .rounded_sm()
                            .bg(theme::alpha(gpui_kit::white(), 0.05))
                            .text_xs()
                            .text_color(faint())
                            .child(cards[i].name.clone()),
                    )
                    .child(Icon::new(Lucide::ChevronRight).xsmall().text_color(faint()))
                    .on_click(
                        cx.listener(move |ws, _, _, cx| ws.open_from_overview(i, Some(target), cx)),
                    ),
            );
        }
    }
    panel(
        "overview-todo",
        Lucide::ListOrdered,
        tr("À faire", "To do"),
        total,
        body,
    )
}

/// My open PRs with their state, then the ones waiting for my review.
fn prs_panel(cards: &[RepoCard], cx: &mut Context<Workspace>) -> AnyElement {
    let mine: Vec<(usize, &PrLine)> = cards
        .iter()
        .enumerate()
        .flat_map(|(i, c)| c.mine.iter().map(move |p| (i, p)))
        .collect();
    let reviews: Vec<(usize, &PrLine)> = cards
        .iter()
        .enumerate()
        .flat_map(|(i, c)| c.to_review.iter().map(move |p| (i, p)))
        .collect();
    let row = |key: &'static str, n: usize, i: usize, p: &PrLine, cx: &mut Context<Workspace>| {
        let (icon, color, label) = p.mark.look();
        h_flex()
            .id(SharedString::from(format!("overview-{key}-{n}")))
            .gap_2p5()
            .px_2()
            .py_1p5()
            .rounded_md()
            .text_sm()
            .cursor_pointer()
            .hover(|st| st.bg(theme::raised()))
            .child(Icon::new(icon).small().text_color(color))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().text_color(ink()).child(p.title.clone()))
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(faint())
                            .child(format!("{} #{} · {label}", cards[i].name, p.number)),
                    ),
            )
            .on_click(
                cx.listener(move |ws, _, _, cx| ws.open_from_overview(i, Some(Target::Prs), cx)),
            )
            .into_any_element()
    };
    let mut body = v_flex().px_2().pb_2();
    if mine.is_empty() && reviews.is_empty() {
        body = body.child(empty(
            Lucide::GitPullRequest,
            faint(),
            tr(
                "Aucune PR ouverte à toi ni en attente de ta review.",
                "No open PR of yours nor waiting for your review.",
            ),
        ));
    }
    let heading = |text: &'static str| {
        div()
            .px_2()
            .pt_2()
            .pb_1()
            .text_xs()
            .font_weight(gpui_kit::FontWeight::MEDIUM)
            .text_color(faint())
            .child(text)
    };
    if !mine.is_empty() {
        body = body.child(heading(tr("Tes PR", "Your PRs")));
        for (n, (i, p)) in mine.iter().enumerate() {
            body = body.child(row("mine", n, *i, p, cx));
        }
    }
    if !reviews.is_empty() {
        body = body.child(heading(tr(
            "Ta review est demandée",
            "Your review is requested",
        )));
        for (n, (i, p)) in reviews.iter().enumerate() {
            body = body.child(row("review", n, *i, p, cx));
        }
    }
    panel(
        "overview-prs",
        Lucide::GitPullRequest,
        tr("Pull requests", "Pull requests"),
        mine.len() + reviews.len(),
        body,
    )
}

fn empty(icon: Lucide, color: Hsla, text: &'static str) -> AnyElement {
    h_flex()
        .gap_2()
        .px_2()
        .py_3()
        .text_sm()
        .text_color(ink2())
        .child(Icon::new(icon).small().text_color(color))
        .child(text)
        .into_any_element()
}

/// The last month of commits, one column per day, rising in turn.
fn pulse(index: usize, days: &[u32]) -> AnyElement {
    let max = days.iter().copied().max().unwrap_or(0).max(1) as f32;
    let n = days.len();
    h_flex()
        .w_full()
        .h(px(64.))
        .items_end()
        .gap(px(2.))
        .children(days.iter().copied().enumerate().map(move |(i, v)| {
            let h = if v == 0 {
                0.04
            } else {
                (v as f32 / max).max(0.12)
            };
            div()
                .flex_1()
                .rounded_t(px(1.5))
                .bg(if i + 1 == n {
                    theme::accent()
                } else if v == 0 {
                    theme::alpha(gpui_kit::white(), 0.06)
                } else {
                    theme::alpha(theme::accent(), 0.45)
                })
                .h(relative(0.))
                .with_animation(
                    ("overview-pulse", index * 64 + i),
                    Animation::new(Duration::from_millis(500 + i as u64 * 12))
                        .with_easing(gpui_kit::ease_out_quint()),
                    move |el, t| el.h(relative(h * t)),
                )
        }))
        .into_any_element()
}

/// Commits per day across the repositories: one column per day, oldest
/// first, holding each card's count in card order. Pulses are aligned on
/// today (one still counting is all zeros).
fn stack(cards: &[RepoCard]) -> Vec<Vec<u32>> {
    let n = cards.iter().map(|c| c.pulse.len()).max().unwrap_or(0);
    (0..n)
        .map(|day| {
            cards
                .iter()
                .map(|c| {
                    let skip = n - c.pulse.len();
                    day.checked_sub(skip).map_or(0, |i| c.pulse[i])
                })
                .collect()
        })
        .collect()
}

/// A clean top for the scale: the next multiple of a round step (29 → 30).
fn scale_top(max: u32) -> u32 {
    let max = max.max(1);
    let step = match max {
        0..=4 => 2,
        5..=10 => 5,
        _ => {
            let p = 10u32.pow(max.ilog10());
            if max <= 2 * p { p / 2 } else { p }
        }
    };
    max.div_ceil(step) * step
}

/// Every repository's last month at once: a column per day stacking each
/// repository's commits in its color, a legend, the detail on hover.
/// Nothing until a pulse is counted.
fn activity_panel(cards: &[RepoCard]) -> Option<AnyElement> {
    let columns = stack(cards);
    if columns.is_empty() {
        return None;
    }
    let totals: Vec<u32> = (0..cards.len())
        .map(|r| columns.iter().map(|c| c[r]).sum())
        .collect();
    let total: u32 = totals.iter().sum();
    let top = scale_top(columns.iter().map(|c| c.iter().sum()).max().unwrap_or(0));
    let n = columns.len();
    let names: Arc<Vec<String>> = Arc::new(cards.iter().map(|c| c.name.clone()).collect());
    let today = chrono::Local::now().date_naive();
    let date = move |ago: usize| {
        if ago == 0 {
            tr("aujourd'hui", "today").to_string()
        } else {
            (today - chrono::Duration::days(ago as i64))
                .format("%d/%m")
                .to_string()
        }
    };
    let bars = columns.into_iter().enumerate().map(|(i, day)| {
        let sum: u32 = day.iter().sum();
        let when = date(n - 1 - i);
        let names = names.clone();
        let detail = day.clone();
        let tip = move |_: &mut gpui_kit::Window, _: &mut gpui_kit::App| {
            v_flex()
                .gap_1()
                .min_w(px(150.))
                .child(
                    h_flex()
                        .gap_3()
                        .text_xs()
                        .child(
                            div()
                                .flex_1()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .child(when.clone()),
                        )
                        .child(div().text_color(faint()).child(match sum {
                            0 => tr("aucun commit", "no commits").to_string(),
                            1 => "1 commit".to_string(),
                            _ => format!("{sum} commits"),
                        })),
                )
                .children(
                    detail
                        .iter()
                        .enumerate()
                        .rev()
                        .filter(|(_, v)| **v > 0)
                        .map(|(r, v)| {
                            h_flex()
                                .gap_2()
                                .text_xs()
                                .child(
                                    div()
                                        .size(px(7.))
                                        .rounded_full()
                                        .bg(theme::lane_color(r as u16)),
                                )
                                .child(div().flex_1().truncate().child(names[r].clone()))
                                .child(v.to_string())
                        }),
                )
        };
        let segments = day
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, v)| **v > 0)
            .map(move |(r, &v)| {
                div()
                    .w_full()
                    .h(relative(v as f32 / sum.max(1) as f32))
                    .bg(theme::alpha(theme::lane_color(r as u16), 0.85))
                    .border_t_1()
                    .border_color(theme::alpha(gpui_kit::black(), 0.25))
            });
        let h = sum as f32 / top as f32;
        div()
            .id(("overview-activity", i))
            .flex_1()
            .h_full()
            .flex()
            .items_end()
            .rounded_t(px(4.))
            .hover(|s| s.bg(theme::alpha(gpui_kit::white(), 0.04)))
            .tooltip(move |window, cx| Tooltip::element(tip.clone()).build(window, cx))
            .child(if sum == 0 {
                div()
                    .w_full()
                    .h(px(2.))
                    .bg(theme::alpha(gpui_kit::white(), 0.06))
                    .into_any_element()
            } else {
                v_flex()
                    .w_full()
                    .min_h(px(3.))
                    .rounded_t(px(3.))
                    .overflow_hidden()
                    .children(segments)
                    .h(relative(0.))
                    .with_animation(
                        ("overview-activity-bar", i),
                        Animation::new(Duration::from_millis(500 + i as u64 * 12))
                            .with_easing(gpui_kit::ease_out_quint()),
                        move |el, t| el.h(relative(h * t)),
                    )
                    .into_any_element()
            })
    });
    let gridline = |frac: f32, label: String| {
        div()
            .absolute()
            .left_0()
            .right_0()
            .top(relative(1. - frac))
            .h(px(1.))
            .bg(theme::alpha(
                gpui_kit::white(),
                if frac == 0. { 0.12 } else { 0.05 },
            ))
            .child(
                div()
                    .absolute()
                    .right(relative(1.))
                    .top(px(-7.))
                    .pr_2()
                    .text_xs()
                    .text_color(faint())
                    .child(label),
            )
    };
    let legend = h_flex()
        .id("overview-activity-legend")
        .test_support()
        .flex_wrap()
        .gap_x_4()
        .gap_y_1()
        .children(cards.iter().zip(&totals).enumerate().map(|(r, (card, t))| {
            h_flex()
                .gap_1p5()
                .text_xs()
                .text_color(ink2())
                .child(
                    div()
                        .size(px(8.))
                        .rounded_full()
                        .bg(theme::lane_color(r as u16)),
                )
                .child(card.name.clone())
                .child(div().text_color(faint()).child(t.to_string()))
        }));
    let body = v_flex()
        .px_4()
        .pb_3()
        .gap_2()
        .child(
            div()
                .relative()
                .ml(px(28.))
                .h(px(150.))
                .child(gridline(1., top.to_string()))
                .child(gridline(0.5, (top / 2).to_string()))
                .child(gridline(0., "0".into()))
                .child(
                    h_flex()
                        .absolute()
                        .inset_0()
                        .items_end()
                        .gap(px(3.))
                        .children(bars),
                ),
        )
        .child(
            h_flex()
                .ml(px(28.))
                .text_xs()
                .text_color(faint())
                .child(div().flex_1().child(date(n - 1)))
                .child(date(0)),
        )
        .child(legend);
    Some(
        div()
            .w_full()
            .child(panel(
                "overview-activity",
                Lucide::ChartColumn,
                tr("Activité cumulée · 30 j", "Combined activity · 30 d"),
                total as usize,
                body,
            ))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signals_come_most_pressing_first() {
        let card = RepoCard {
            dirty: 3,
            conflicts: 1,
            ahead: 2,
            reviews: 1,
            stashes: 1,
            ..RepoCard::default()
        };
        assert_eq!(
            card.signals(),
            [
                Signal::Conflicts(1),
                Signal::Reviews(1),
                Signal::Ahead(2),
                Signal::Dirty(2),
                Signal::Stashes(1),
            ]
        );
        assert_eq!(card.signals()[0].level(), Level::Alert);
        assert_eq!(card.signals()[0].target(), Target::Changes);
        assert!(RepoCard::default().signals().is_empty());
    }

    #[test]
    fn activity_stacks_each_repo_on_its_day() {
        let card = |days: Vec<u32>| RepoCard {
            pulse: Arc::new(days),
            ..RepoCard::default()
        };
        // The second pulse is still counting: zeros, aligned on today.
        let cols = stack(&[card(vec![1, 0, 3]), card(vec![]), card(vec![2, 5])]);
        assert_eq!(cols, [vec![1, 0, 0], vec![0, 0, 2], vec![3, 0, 5]]);
        assert!(stack(&[card(vec![])]).is_empty());
        assert_eq!(scale_top(8), 10);
        assert_eq!(scale_top(29), 30);
        assert_eq!(scale_top(0), 2);
    }
}
