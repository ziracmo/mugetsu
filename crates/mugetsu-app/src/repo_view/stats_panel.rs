//! Sidebar "Stats" tab: who writes, commits, reviews and merges on the
//! default branch, when, and which files move. Commits only (no forge API).
//! Metadata shows at once; line counts stream in while bars keep filling.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gix::ObjectId;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, AppContext as _, Context, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, linear_color_stop, linear_gradient,
    prelude::FluentBuilder as _, px, relative,
};
use mugetsu_git::{FileHeat, Person, RepoStats};

use super::RepoView;
use crate::i18n::tr;
use crate::theme;

/// Rows shown per leaderboard.
const TOP: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Period {
    Week,
    #[default]
    Month,
    Quarter,
    Year,
    All,
}

impl Period {
    const ALL: [Period; 5] = [
        Period::Week,
        Period::Month,
        Period::Quarter,
        Period::Year,
        Period::All,
    ];

    pub fn days(self) -> Option<i64> {
        match self {
            Period::Week => Some(7),
            Period::Month => Some(30),
            Period::Quarter => Some(90),
            Period::Year => Some(365),
            Period::All => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Period::Week => tr("7j", "7d"),
            Period::Month => tr("30j", "30d"),
            Period::Quarter => tr("90j", "90d"),
            Period::Year => tr("1 an", "1y"),
            Period::All => tr("Tout", "All"),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Period::Week => "stats-period-7",
            Period::Month => "stats-period-30",
            Period::Quarter => "stats-period-90",
            Period::Year => "stats-period-365",
            Period::All => "stats-period-all",
        }
    }
}

/// Stats of one tab, and what they were computed for.
#[derive(Default)]
pub struct StatsState {
    pub period: Period,
    pub stats: Option<Arc<RepoStats>>,
    /// Branch and its tip the stats belong to (recomputed when either moves).
    branch: Option<String>,
    tip: Option<ObjectId>,
    computed_for: Option<Period>,
    running: bool,
    error: Option<String>,
    /// Stops the running computation (period changed, tab closed).
    cancel: Arc<AtomicBool>,
    /// Bumped per computation: keys the fill animations.
    pub seq: usize,
}

impl Drop for StatsState {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Liquid fill: rises, overshoots a little, then settles with a wobble.
fn liquid(t: f32) -> f32 {
    if t >= 1. {
        return 1.;
    }
    let wobble = 0.4 + 0.6 * (t * std::f32::consts::PI * 3.).cos();
    1. - (1. - t).powi(3) * wobble
}

/// Accent fading to transparent, left to right (or bottom to top).
fn fill(angle: f32, strength: f32) -> gpui_kit::Background {
    linear_gradient(
        angle,
        linear_color_stop(theme::alpha(theme::accent(), strength), 0.),
        linear_color_stop(theme::alpha(theme::accent(), 0.), 1.),
    )
}

fn compact(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

fn matches(hay: &[&str], query: &str) -> bool {
    let hay = hay.join(" ").to_lowercase();
    query
        .split_whitespace()
        .all(|w| hay.contains(&w.to_lowercase()))
}

impl RepoView {
    /// Starts (or restarts) the computation when the period, the default
    /// branch's tip or the repository changed. Cheap when nothing did.
    pub(super) fn ensure_stats(&mut self, cx: &mut Context<Self>) {
        if self.sidebar_tab != super::SidebarTab::Stats
            || self.loading && self.data.history.is_empty()
        {
            return;
        }
        let s = &self.stats;
        let tip = s.branch.as_deref().and_then(|b| self.branch_tip(b));
        let stale = s.computed_for != Some(s.period) || (!s.running && tip != s.tip);
        if !stale {
            return;
        }
        self.compute_stats(cx);
    }

    fn branch_tip(&self, name: &str) -> Option<ObjectId> {
        let refs = &self.data.refs;
        refs.remote_branches
            .iter()
            .chain(&refs.branches)
            .find(|b| b.name == name)
            .map(|b| b.target)
            .or_else(|| (name == "HEAD").then_some(refs.head).flatten())
    }

    fn compute_stats(&mut self, cx: &mut Context<Self>) {
        self.stats.cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        let period = self.stats.period;
        self.stats.cancel = cancel.clone();
        self.stats.computed_for = Some(period);
        self.stats.running = true;
        self.stats.error = None;
        self.stats.seq += 1;
        let seq = self.stats.seq;
        let repo = self.repo.clone();
        let now = chrono::Utc::now().timestamp();
        let since = period.days().map(|d| now - d * 86_400);
        let latest: Arc<Mutex<Option<RepoStats>>> = Arc::default();
        let finished = Arc::new(AtomicBool::new(false));
        let work = cx.background_spawn({
            let latest = latest.clone();
            let finished = finished.clone();
            let cancel = cancel.clone();
            async move {
                let branch = repo.default_branch();
                let tip = repo
                    .git()
                    .run([
                        "rev-parse",
                        "--verify",
                        "-q",
                        &format!("{branch}^{{commit}}"),
                    ])
                    .ok()
                    .and_then(|s| ObjectId::from_hex(s.trim().as_bytes()).ok());
                let result = repo.stats(&branch, since, now, &cancel, |s| {
                    *latest.lock().unwrap() = Some(s);
                });
                finished.store(true, Ordering::Release);
                (branch, tip, result)
            }
        });
        cx.spawn(async move |this, cx| {
            let mut first = true;
            loop {
                if !first {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                }
                first = false;
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let partial = latest.lock().unwrap().take();
                if let Some(partial) = partial {
                    let alive = this.update(cx, |this, cx| {
                        if this.stats.seq == seq {
                            this.stats.stats = Some(Arc::new(partial));
                            cx.notify();
                        }
                    });
                    if alive.is_err() {
                        return;
                    }
                }
                if finished.load(Ordering::Acquire) {
                    break;
                }
            }
            let (branch, tip, result) = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.stats.seq != seq {
                    return;
                }
                this.stats.running = false;
                this.stats.branch = Some(branch);
                this.stats.tip = tip;
                match result {
                    Ok(stats) => this.stats.stats = Some(Arc::new(stats)),
                    Err(e) => this.stats.error = Some(format!("{e:#}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn set_stats_period(&mut self, period: Period, cx: &mut Context<Self>) {
        if self.stats.period == period {
            return;
        }
        self.stats.period = period;
        // Other period: drop the old numbers rather than show them as new.
        self.stats.stats = None;
        self.compute_stats(cx);
    }

    #[cfg(test)]
    pub fn stats_snapshot(&self) -> Option<(Arc<RepoStats>, bool)> {
        self.stats.stats.clone().map(|s| (s, self.stats.running))
    }

    /// Content of the Stats tab.
    pub(super) fn stats_sections(&self, query: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let searching = !query.is_empty();
        let mut out = vec![self.render_stats_header(cx)];
        if let Some(err) = &self.stats.error {
            out.push(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(theme::red())
                    .child(err.clone())
                    .into_any_element(),
            );
            return out;
        }
        let Some(stats) = self.stats.stats.clone() else {
            out.push(skeleton());
            return out;
        };
        if stats.commits == 0 {
            out.push(
                div()
                    .p_4()
                    .text_sm()
                    .text_color(theme::muted())
                    .child(match self.stats.period.days() {
                        Some(days) => trf!(
                            "Aucun commit sur {} ces {days} derniers jours.",
                            "No commits on {} in the last {days} days.",
                            stats.branch
                        ),
                        None => trf!("Aucun commit sur {}.", "No commits on {}.", stats.branch),
                    })
                    .into_any_element(),
            );
            return out;
        }
        let seq = self.stats.seq;
        if !searching {
            out.push(tiles(&stats, seq));
            out.push(activity(&stats, seq));
        }
        let people = |list: &[Person], key: &'static str, lines: bool, cx: &mut Context<Self>| {
            let max = list.first().map_or(1, |p| p.commits).max(1);
            list.iter()
                .filter(|p| matches(&[&p.name, &p.email], query))
                .take(if searching { usize::MAX } else { TOP })
                .enumerate()
                .map(|(i, p)| person_row(key, i, p, max, lines, seq, cx))
                .collect::<Vec<_>>()
        };
        let lines = stats.lines_scanned > 0;
        let groups: [(&'static str, &'static str, Lucide, &[Person], bool); 5] = [
            (
                "stats-authors",
                tr("Auteurs", "Authors"),
                Lucide::Users,
                &stats.authors,
                lines,
            ),
            (
                "stats-reviewers",
                tr("Reviewers", "Reviewers"),
                Lucide::Eye,
                &stats.reviewers,
                false,
            ),
            (
                "stats-mergers",
                tr("Merges de PR", "PR merges"),
                Lucide::GitMerge,
                &stats.mergers,
                false,
            ),
            (
                "stats-committers",
                tr("Committers", "Committers"),
                Lucide::GitCommitHorizontal,
                &stats.committers,
                false,
            ),
            (
                "stats-coauthors",
                tr("Co-auteurs", "Co-authors"),
                Lucide::HeartHandshake,
                &stats.coauthors,
                false,
            ),
        ];
        for (key, title, icon, list, lines) in groups {
            if list.is_empty() {
                continue;
            }
            let rows = people(list, key, lines, cx);
            out.extend(self.section(key, title, icon, list.len(), rows, searching, None, cx));
        }
        if !searching {
            out.extend(self.section(
                "stats-when",
                tr("Quand", "When"),
                Lucide::CalendarClock,
                stats.commits as usize,
                vec![punchcard(&stats, seq)],
                false,
                None,
                cx,
            ));
        }
        let files: Vec<&FileHeat> = stats
            .files
            .iter()
            .filter(|f| matches(&[&f.path], query))
            .take(if searching { 50 } else { TOP })
            .collect();
        if !files.is_empty() {
            let max = stats.files.first().map_or(1, |f| f.commits).max(1);
            let rows = files
                .iter()
                .enumerate()
                .map(|(i, f)| file_row(i, f, max, seq))
                .collect();
            out.extend(self.section(
                "stats-files",
                tr("Fichiers chauds", "Hot files"),
                Lucide::Flame,
                stats.files.len(),
                rows,
                searching,
                None,
                cx,
            ));
        }
        out
    }

    /// Branch, progress and the period picker.
    fn render_stats_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let stats = self.stats.stats.as_ref();
        let branch = stats
            .map(|s| s.branch.clone())
            .or_else(|| self.stats.branch.clone());
        let chip = |period: Period| {
            let active = self.stats.period == period;
            div()
                .id(period.id())
                .test_support()
                .flex_1()
                .h(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .text_xs()
                .cursor_pointer()
                .border_1()
                .when(active, |s| {
                    s.bg(theme::alpha(theme::accent(), 0.25))
                        .border_color(theme::alpha(theme::accent(), 0.7))
                        .text_color(gpui_kit::white())
                })
                .when(!active, |s| {
                    s.border_color(theme::hairline())
                        .text_color(theme::muted())
                        .hover(|s| s.text_color(gpui_kit::white()))
                })
                .child(period.label())
                .on_click(cx.listener(move |this, _, _, cx| this.set_stats_period(period, cx)))
        };
        let progress = stats
            .filter(|s| !s.lines_done && s.lines_total() > 0)
            .map(|s| (s.lines_scanned as f32 / s.lines_total() as f32).min(1.));
        v_flex()
            .px_2()
            .pb_1()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .text_xs()
                    .text_color(theme::muted())
                    .child(Icon::new(Lucide::GitBranch).xsmall())
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_color(gpui_kit::white())
                            .child(branch.unwrap_or_else(|| "…".into())),
                    )
                    .when(self.stats.running, |s| {
                        s.child(div().child(match progress {
                            Some(p) => {
                                trf!("lignes {}%", "lines {}%", (p * 100.).round() as u32)
                            }
                            None => tr("calcul…", "computing…").to_string(),
                        }))
                    }),
            )
            .child(h_flex().gap_1().children(Period::ALL.map(chip)))
            .when_some(progress, |s, p| {
                s.child(
                    div()
                        .relative()
                        .h(px(2.))
                        .w_full()
                        .rounded_full()
                        .overflow_hidden()
                        .bg(theme::raised())
                        .child(
                            div()
                                .h_full()
                                .w(relative(p))
                                .rounded_full()
                                .bg(fill(270., 0.9)),
                        ),
                )
            })
            .into_any_element()
    }
}

/// Placeholder while the first numbers load.
fn skeleton() -> AnyElement {
    v_flex()
        .px_2()
        .gap_1p5()
        .children((0..6usize).map(|i| {
            div()
                .h(px(18.))
                .w(relative(0.9 - i as f32 * 0.1))
                .rounded_md()
                .bg(theme::raised())
                .with_animation(
                    ("stats-skeleton", i),
                    Animation::new(Duration::from_millis(1100)).repeat(),
                    move |el, t| {
                        let wave = ((t + i as f32 * 0.12) * std::f32::consts::TAU).sin();
                        el.opacity(0.55 + 0.35 * wave)
                    },
                )
        }))
        .into_any_element()
}

/// Commits, people, lines added / removed, counting up.
fn tiles(stats: &RepoStats, seq: usize) -> AnyElement {
    let people = stats.authors.len() as u64;
    let lines_known = stats.lines_scanned > 0;
    let tile =
        |id: usize, label: &'static str, value: Option<u64>, prefix: &'static str, color: Hsla| {
            v_flex()
                .flex_1()
                .min_w_0()
                .px_2()
                .py_1()
                .rounded_lg()
                .border_1()
                .border_color(theme::hairline())
                .bg(fill(180., 0.12))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::muted())
                        .truncate()
                        .child(label),
                )
                .child(
                    div()
                        .text_base()
                        .font_weight(gpui_kit::FontWeight::BOLD)
                        .text_color(color)
                        .truncate()
                        .with_animation(
                            ("stats-tile", seq * 8 + id),
                            Animation::new(Duration::from_millis(900))
                                .with_easing(gpui_kit::ease_out_quint()),
                            move |el, t| {
                                el.child(match value {
                                    Some(v) => {
                                        format!("{prefix}{}", compact((v as f64 * t as f64) as u64))
                                    }
                                    None => "…".into(),
                                })
                            },
                        ),
                )
        };
    v_flex()
        .px_2()
        .pb_1()
        .gap_1()
        .child(
            h_flex()
                .gap_1()
                .child(tile(
                    0,
                    tr("Commits", "Commits"),
                    Some(stats.commits as u64),
                    "",
                    gpui_kit::white(),
                ))
                .child(tile(
                    1,
                    tr("Auteurs", "Authors"),
                    Some(people),
                    "",
                    gpui_kit::white(),
                )),
        )
        .child(
            h_flex()
                .gap_1()
                .child(tile(
                    2,
                    tr("Ajouts", "Added"),
                    lines_known.then_some(stats.additions),
                    "+",
                    theme::lime(),
                ))
                .child(tile(
                    3,
                    tr("Suppressions", "Removed"),
                    lines_known.then_some(stats.deletions),
                    "−",
                    theme::red(),
                )),
        )
        .into_any_element()
}

/// Commits over the period: bars rising one after the other.
fn activity(stats: &RepoStats, seq: usize) -> AnyElement {
    let max = stats.activity.iter().copied().max().unwrap_or(0).max(1);
    let n = stats.activity.len().max(1);
    let unit = if stats.bucket_days == 1 {
        tr("jour", "day").to_string()
    } else {
        trf!("{} jours", "{} days", stats.bucket_days)
    };
    let bars = stats.activity.iter().enumerate().map(|(i, &v)| {
        let h = v as f32 / max as f32;
        let ago = (n - 1 - i) as u32 * stats.bucket_days;
        let tip = match ago {
            0 => trf!("{v} commit(s) — aujourd'hui", "{v} commit(s) — today"),
            _ => trf!(
                "{v} commit(s) — il y a {ago} j",
                "{v} commit(s) — {ago}d ago"
            ),
        };
        div()
            .id(("stats-activity", i))
            .flex_1()
            .h_full()
            .flex()
            .items_end()
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .child(
                div()
                    .w_full()
                    .rounded_t_sm()
                    .bg(fill(0., 0.95))
                    .when(v > 0, |s| s.min_h(px(2.)))
                    .with_animation(
                        ("stats-activity-bar", seq * 1024 + i),
                        Animation::new(Duration::from_millis(1000)),
                        move |el, t| {
                            // Staggered left to right, then a liquid settle.
                            let start = i as f32 / n as f32 * 0.4;
                            let t = ((t - start) / 0.6).clamp(0., 1.);
                            el.h(relative(h * liquid(t)))
                        },
                    ),
            )
    });
    v_flex()
        .px_2()
        .pb_2()
        .gap_0p5()
        .child(
            h_flex()
                .text_xs()
                .text_color(theme::muted())
                .child(div().flex_1().child(tr("Activité", "Activity")))
                .child(trf!("max {max} / {unit}", "max {max} / {unit}")),
        )
        .child(
            h_flex()
                .h(px(56.))
                .items_end()
                .gap(px(1.))
                .border_b_1()
                .border_color(theme::hairline())
                .children(bars),
        )
        .into_any_element()
}

/// A leaderboard row: the bar fills behind the name like a liquid.
fn person_row(
    key: &'static str,
    i: usize,
    p: &Person,
    max: u32,
    lines: bool,
    seq: usize,
    cx: &mut Context<RepoView>,
) -> AnyElement {
    let frac = p.commits as f32 / max as f32;
    let tip = format!("{} <{}>", p.name, p.email);
    let extra = (lines && (p.additions > 0 || p.deletions > 0)).then(|| {
        h_flex()
            .flex_none()
            .gap_1()
            .text_xs()
            .child(
                div()
                    .text_color(theme::lime())
                    .child(format!("+{}", compact(p.additions))),
            )
            .child(
                div()
                    .text_color(theme::red())
                    .child(format!("−{}", compact(p.deletions))),
            )
            .into_any_element()
    });
    bar_row(
        SharedString::from(format!("{key}-{i}")),
        frac,
        i,
        seq,
        crate::avatars::avatar(&p.email, &p.name, 16., cx),
        div()
            .flex_1()
            .min_w_0()
            .truncate()
            .when(i == 0, |s| s.font_weight(gpui_kit::FontWeight::SEMIBOLD))
            .child(p.name.clone())
            .into_any_element(),
        p.commits.to_string(),
        extra,
        tip,
    )
}

fn file_row(i: usize, f: &FileHeat, max: u32, seq: usize) -> AnyElement {
    let (dir, name) = match f.path.rsplit_once('/') {
        Some((d, n)) => (Some(d.to_string()), n.to_string()),
        None => (None, f.path.clone()),
    };
    let tip = trf!(
        "{}\n{} commit(s), +{} −{}",
        "{}\n{} commit(s), +{} −{}",
        f.path,
        f.commits,
        f.additions,
        f.deletions
    );
    bar_row(
        SharedString::from(format!("stats-file-{i}")),
        f.commits as f32 / max as f32,
        i,
        seq,
        Icon::new(Lucide::File)
            .xsmall()
            .text_color(theme::muted())
            .into_any_element(),
        h_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .flex_none()
                    .max_w(relative(0.7))
                    .truncate()
                    .child(name),
            )
            .children(dir.map(|d| {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme::muted())
                    .child(d)
            }))
            .into_any_element(),
        f.commits.to_string(),
        None,
        tip,
    )
}

/// Shared leaderboard row: gradient fill (accent → transparent) behind the
/// content, rising with a liquid wobble, then one light sweep.
#[allow(clippy::too_many_arguments)]
fn bar_row(
    id: SharedString,
    frac: f32,
    rank: usize,
    seq: usize,
    leading: AnyElement,
    label: AnyElement,
    count: String,
    extra: Option<AnyElement>,
    tip: String,
) -> AnyElement {
    let delay = (rank as f32 * 0.07).min(0.4);
    h_flex()
        .id(id.clone())
        .test_support()
        .relative()
        .h(px(24.))
        .mx_1()
        .px_2()
        .gap_2()
        .min_w_0()
        .rounded_md()
        .overflow_hidden()
        .text_sm()
        .hover(|s| s.bg(theme::raised()))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .rounded_md()
                .overflow_hidden()
                .bg(fill(90., if rank == 0 { 0.6 } else { 0.42 }))
                .with_animation(
                    SharedString::from(format!("{id}-fill-{seq}")),
                    Animation::new(Duration::from_millis(1100)),
                    move |el, t| {
                        let t = ((t - delay) / (1. - delay)).clamp(0., 1.);
                        el.w(relative(frac * liquid(t)))
                    },
                )
                .child(super::shimmer::sweep(seq * 64 + rank)),
        )
        .child(leading)
        .child(label)
        .child(
            div()
                .flex_none()
                .text_xs()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(gpui_kit::white())
                .child(count),
        )
        .children(extra)
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .into_any_element()
}

/// Weekday × hour heatmap in the authors' own time zones.
fn punchcard(stats: &RepoStats, seq: usize) -> AnyElement {
    let max = stats
        .punchcard
        .iter()
        .flatten()
        .copied()
        .max()
        .unwrap_or(0)
        .max(1);
    let days = if crate::i18n::is_fr() {
        ["L", "M", "M", "J", "V", "S", "D"]
    } else {
        ["M", "T", "W", "T", "F", "S", "S"]
    };
    let rows = stats.punchcard.iter().enumerate().map(|(d, hours)| {
        h_flex()
            .gap(px(2.))
            .child(
                div()
                    .w(px(10.))
                    .flex_none()
                    .text_xs()
                    .text_color(theme::muted())
                    .child(days[d]),
            )
            .children(hours.iter().enumerate().map(move |(h, &v)| {
                let k = v as f32 / max as f32;
                let tip = trf!("{v} commit(s) — {}h", "{v} commit(s) — {}:00", h);
                div()
                    .id(("stats-punch", d * 24 + h))
                    .flex_1()
                    .h(px(10.))
                    .rounded_sm()
                    .bg(if v == 0 {
                        theme::alpha(gpui_kit::white(), 0.04)
                    } else {
                        theme::alpha(theme::accent(), 0.18 + 0.82 * k)
                    })
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .with_animation(
                        ("stats-punch-in", seq * 256 + d * 24 + h),
                        Animation::new(Duration::from_millis(900)),
                        move |el, t| {
                            // A wave sweeping across the hours.
                            let start = (h as f32 / 24. * 0.5) + d as f32 * 0.03;
                            let t = ((t - start) / 0.4).clamp(0., 1.);
                            el.opacity(t)
                        },
                    )
            }))
    });
    v_flex()
        .px_2()
        .pb_2()
        .gap(px(2.))
        .children(rows)
        .child(
            h_flex()
                .pl(px(12.))
                .text_xs()
                .text_color(theme::muted())
                .child(div().flex_1().child("0h"))
                .child(div().flex_1().child("6h"))
                .child(div().flex_1().child("12h"))
                .child(div().flex_1().child("18h")),
        )
        .into_any_element()
}
