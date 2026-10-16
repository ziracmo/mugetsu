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
    /// Commits per day over the last month (the sidebar's pulse).
    pub pulse: Arc<Vec<u32>>,
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

/// Signals shown per card before "+n".
const SHOWN: usize = 5;

fn level_color(level: Level) -> Hsla {
    match level {
        Level::Alert => theme::red(),
        Level::Act => theme::amber(),
        Level::Note => theme::muted(),
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

/// The whole window below the title bar: header, totals, the cards.
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
    let summary = if cards.is_empty() {
        tr(
            "Aucun dépôt ouvert dans ce workspace.",
            "No repository open in this workspace.",
        )
        .to_string()
    } else if alerts + acts == 0 {
        tr(
            "Rien ne bloque : tout est à jour.",
            "Nothing blocking: all up to date.",
        )
        .to_string()
    } else {
        [
            (alerts > 0).then(|| trf!("{alerts} urgence(s)", "{alerts} urgent")),
            (acts > 0).then(|| trf!("{acts} action(s)", "{acts} to act on")),
            (calm > 0).then(|| trf!("{calm} dépôt(s) au calme", "{calm} calm repo(s)")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
    };
    let header = h_flex()
        .w_full()
        .px_6()
        .pt_5()
        .pb_4()
        .gap_3()
        .items_end()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_xl()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(ink())
                        .child(workspace.to_string()),
                )
                .child(
                    div()
                        .id("overview-summary")
                        .test_support()
                        .text_sm()
                        .text_color(faint())
                        .child(summary),
                ),
        )
        .child(
            Button::new("overview-recap")
                .primary()
                .small()
                .icon(Icon::new(Lucide::Sparkles))
                .label(tr("Recap de la semaine", "Weekly recap"))
                .disabled(cards.is_empty())
                .on_click(cx.listener(|ws, _, window, cx| ws.open_recap(window, cx))),
        );
    let grid = h_flex()
        .w_full()
        .flex_wrap()
        .items_start()
        .gap_4()
        .children(
            cards
                .iter()
                .zip(signals)
                .enumerate()
                .map(|(i, (card, signals))| render_card(i, card, signals, cx)),
        );
    v_flex()
        .id("overview")
        .test_support()
        .size_full()
        .overflow_y_scroll()
        .child(header)
        .child(div().h(px(1.)).w_full().flex_none().bg(theme::hairline()))
        .child(div().w_full().px_6().pt_5().pb_8().child(grid))
        .into_any_element()
}

fn render_card(
    index: usize,
    card: &RepoCard,
    signals: Vec<Signal>,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let emoji = crate::settings::get().repo_icons.get(&card.root).cloned();
    let worst = signals.first().map(|s| s.level());
    let rim = worst.map_or(theme::hairline(), |l| theme::alpha(level_color(l), 0.45));
    let more = signals.len().saturating_sub(SHOWN);
    let rows: Vec<AnyElement> = if signals.is_empty() {
        vec![
            h_flex()
                .gap_2()
                .px_2()
                .py_1()
                .text_sm()
                .text_color(ink2())
                .child(
                    Icon::new(Lucide::CircleCheck)
                        .small()
                        .text_color(theme::lime()),
                )
                .child(tr("Rien à faire", "Nothing to do"))
                .into_any_element(),
        ]
    } else {
        signals
            .into_iter()
            .take(SHOWN)
            .enumerate()
            .map(|(j, s)| {
                let target = s.target();
                h_flex()
                    .id(SharedString::from(format!("overview-signal-{index}-{j}")))
                    .test_support()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_sm()
                    .text_color(ink2())
                    .cursor_pointer()
                    .hover(|st| st.bg(theme::raised()).text_color(ink()))
                    .child(
                        Icon::new(s.icon())
                            .small()
                            .text_color(level_color(s.level())),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(s.text()))
                    .child(Icon::new(Lucide::ChevronRight).xsmall().text_color(faint()))
                    .on_click(cx.listener(move |ws, _, _, cx| {
                        cx.stop_propagation();
                        ws.open_from_overview(index, Some(target), cx);
                    }))
                    .into_any_element()
            })
            .collect()
    };
    let initial = card
        .name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    v_flex()
        .id(SharedString::from(format!("overview-card-{}", card.name)))
        .test_support()
        .w(px(320.))
        .relative()
        .rounded_xl()
        .border_1()
        .border_color(rim)
        .bg(theme::alpha(gpui_kit::white(), 0.028))
        .overflow_hidden()
        .cursor_pointer()
        .hover(|s| s.bg(theme::alpha(gpui_kit::white(), 0.05)))
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
                .gap_2p5()
                .child(
                    div()
                        .flex_none()
                        .size(px(30.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_lg()
                        .bg(theme::alpha(gpui_kit::white(), 0.06))
                        .child(match &emoji {
                            Some(e) => crate::repo_icon::view(e, 18.),
                            None => div()
                                .text_sm()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(ink2())
                                .child(initial)
                                .into_any_element(),
                        }),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_sm()
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
                                .child(div().min_w_0().truncate().child(card.branch.clone()))
                                .when(card.my_prs > 0, |s| {
                                    s.child("·").child(trf!(
                                        "{} PR à toi",
                                        "{} PR(s) of yours",
                                        card.my_prs
                                    ))
                                }),
                        ),
                )
                .child(pulse(index, &card.pulse)),
        )
        .child(v_flex().px_2().pb_3().children(rows).when(more > 0, |s| {
            s.child(
                div()
                    .pt_0p5()
                    .text_xs()
                    .text_color(faint())
                    .child(trf!("+{more} autre(s)", "+{more} more")),
            )
        }))
        .on_click(cx.listener(move |ws, _, _, cx| ws.open_from_overview(index, None, cx)))
        .into_any_element()
}

/// Last two weeks of commits, as small rising bars.
fn pulse(index: usize, days: &[u32]) -> AnyElement {
    let days: Vec<u32> = days.iter().rev().take(14).rev().copied().collect();
    let max = days.iter().copied().max().unwrap_or(0).max(1) as f32;
    let n = days.len();
    h_flex()
        .flex_none()
        .h(px(24.))
        .w(px(56.))
        .items_end()
        .gap(px(1.5))
        .children(days.into_iter().enumerate().map(move |(i, v)| {
            let h = (v as f32 / max).max(0.08);
            div()
                .flex_1()
                .h(relative(h))
                .rounded_t(px(1.))
                .bg(if i + 1 == n {
                    theme::accent()
                } else {
                    theme::alpha(gpui_kit::white(), 0.2)
                })
                .with_animation(
                    ("overview-pulse", index * 32 + i),
                    Animation::new(Duration::from_millis(500 + i as u64 * 25))
                        .with_easing(gpui_kit::ease_out_quint()),
                    move |el, t| el.h(relative(h * t)),
                )
        }))
        .into_any_element()
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
}
