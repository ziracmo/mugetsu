//! "Stats" view (next to History and Changes): who writes, commits, reviews and merges on the
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
    InteractiveElement as _, IntoElement, ParentElement as _, Rgba, SharedString,
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

impl RepoView {
    /// Starts (or restarts) the computation when the period, the default
    /// branch's tip or the repository changed. Cheap when nothing did.
    pub(super) fn ensure_stats(&mut self, cx: &mut Context<Self>) {
        if self.mode != super::Mode::Stats || self.loading && self.data.history.is_empty() {
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

    /// The Stats view, in place of the history.
    pub(super) fn render_stats(&self, cx: &mut Context<Self>) -> AnyElement {
        let stats = self.stats.stats.clone();
        let body: Vec<AnyElement> = match (&self.stats.error, &stats) {
            (Some(err), _) => vec![empty_state(Lucide::TriangleAlert, err.clone())],
            (None, None) => vec![skeleton()],
            (None, Some(stats)) if stats.commits == 0 => {
                vec![empty_state(
                    Lucide::Inbox,
                    match self.stats.period.days() {
                        Some(days) => trf!(
                            "Aucun commit sur {} ces {days} derniers jours.",
                            "No commits on {} in the last {days} days.",
                            stats.branch
                        ),
                        None => trf!("Aucun commit sur {}.", "No commits on {}.", stats.branch),
                    },
                )]
            }
            (None, Some(stats)) => self.stats_dashboard(stats, cx),
        };
        v_flex()
            .id("stats-view")
            .size_full()
            .overflow_y_scroll()
            .child(self.render_stats_header(cx))
            .child(
                v_flex()
                    .w_full()
                    .px_5()
                    .pt_4()
                    .pb_6()
                    .gap_4()
                    .children(body),
            )
            .into_any_element()
    }

    fn stats_dashboard(&self, stats: &RepoStats, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let seq = self.stats.seq;
        let lines = stats.lines_scanned > 0;
        let people = |key: &'static str,
                      title: &'static str,
                      hint: &'static str,
                      list: &[Person],
                      lines: bool,
                      cx: &mut Context<Self>| {
            (!list.is_empty()).then(|| {
                let max = list.first().map_or(1, |p| p.commits).max(1);
                let rows = list
                    .iter()
                    .take(TOP)
                    .enumerate()
                    .map(|(i, p)| person_row(key, i, p, max, lines, seq, cx))
                    .collect();
                panel(key, title, hint, list.len(), rows)
            })
        };
        let files = (!stats.files.is_empty()).then(|| {
            let max = stats.files.first().map_or(1, |f| f.commits).max(1);
            let rows = stats
                .files
                .iter()
                .take(TOP)
                .enumerate()
                .map(|(i, f)| file_row(i, f, max, seq))
                .collect();
            panel(
                "stats-files",
                tr("Fichiers chauds", "Hot files"),
                tr("commits qui les touchent", "commits touching them"),
                stats.files.len(),
                rows,
            )
        });
        let left: Vec<AnyElement> = [
            people(
                "stats-authors",
                tr("Auteurs", "Authors"),
                tr(
                    "commits, lignes ajoutées et supprimées",
                    "commits, lines added and removed",
                ),
                &stats.authors,
                lines,
                cx,
            ),
            people(
                "stats-committers",
                tr("Committers", "Committers"),
                tr("qui applique les commits", "who applies the commits"),
                &stats.committers,
                false,
                cx,
            ),
            people(
                "stats-coauthors",
                tr("Co-auteurs", "Co-authors"),
                "Co-authored-by",
                &stats.coauthors,
                false,
                cx,
            ),
        ]
        .into_iter()
        .flatten()
        .collect();
        let right: Vec<AnyElement> = [
            files,
            people(
                "stats-reviewers",
                tr("Reviewers", "Reviewers"),
                "Reviewed-by · Approved-by",
                &stats.reviewers,
                false,
                cx,
            ),
            people(
                "stats-mergers",
                tr("Merges de PR", "PR merges"),
                tr("auteurs des commits de merge", "authors of merge commits"),
                &stats.mergers,
                false,
                cx,
            ),
        ]
        .into_iter()
        .flatten()
        .collect();
        vec![
            tiles(stats, seq),
            panel(
                "stats-activity",
                tr("Activité", "Activity"),
                if stats.bucket_days == 1 {
                    tr("commits par jour", "commits per day")
                } else if stats.bucket_days == 7 {
                    tr("commits par semaine", "commits per week")
                } else {
                    tr("commits par période", "commits per period")
                },
                0,
                vec![activity(stats, seq)],
            ),
            h_flex()
                .w_full()
                .items_start()
                .gap_4()
                .child(v_flex().flex_1().min_w_0().gap_4().children(left))
                .child(v_flex().flex_1().min_w_0().gap_4().children(right))
                .into_any_element(),
            panel(
                "stats-when",
                tr("Rythme", "Rhythm"),
                tr(
                    "jour × heure, dans le fuseau de chaque auteur",
                    "day × hour, in each author's time zone",
                ),
                0,
                vec![punchcard(stats, seq)],
            ),
        ]
    }

    /// Title, branch, the period picker and the line-count progress.
    fn render_stats_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let stats = self.stats.stats.as_ref();
        let branch = stats
            .map(|s| s.branch.clone())
            .or_else(|| self.stats.branch.clone())
            .unwrap_or_else(|| "…".into());
        let segment = |period: Period| {
            let active = self.stats.period == period;
            div()
                .id(period.id())
                .test_support()
                .h(px(24.))
                .px_3()
                .flex()
                .items_center()
                .rounded_md()
                .text_xs()
                .font_weight(gpui_kit::FontWeight::MEDIUM)
                .cursor_pointer()
                .when(active, |s| {
                    s.bg(theme::alpha(gpui_kit::white(), 0.14))
                        .border_1()
                        .border_color(theme::glass_edge())
                        .shadow_sm()
                        .text_color(ink())
                })
                .when(!active, |s| {
                    s.text_color(muted()).hover(|s| s.text_color(ink()))
                })
                .child(period.label())
                .on_click(cx.listener(move |this, _, _, cx| this.set_stats_period(period, cx)))
        };
        let progress = stats
            .filter(|s| !s.lines_done && s.lines_total() > 0)
            .map(|s| (s.lines_scanned as f32 / s.lines_total() as f32).min(1.));
        let subtitle = match stats {
            Some(s) if self.stats.running => match progress {
                Some(p) => trf!(
                    "{} commits · lignes {}%",
                    "{} commits · lines {}%",
                    s.commits,
                    (p * 100.).round() as u32
                ),
                None => trf!("{} commits", "{} commits", s.commits),
            },
            Some(s) => trf!("{} commits", "{} commits", s.commits),
            None => tr("Calcul…", "Computing…").to_string(),
        };
        v_flex()
            .w_full()
            .flex_none()
            .child(
                h_flex()
                    .px_5()
                    .pt_4()
                    .pb_3()
                    .gap_3()
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
                                    .child(tr("Statistiques", "Statistics")),
                            )
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .text_xs()
                                    .text_color(muted())
                                    .child(Icon::new(Lucide::GitBranch).xsmall())
                                    .child(div().text_color(ink2()).child(branch))
                                    .child("·")
                                    .child(subtitle),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .p(px(2.))
                            .gap_0p5()
                            .rounded_lg()
                            .bg(theme::alpha(gpui_kit::white(), 0.05))
                            .border_1()
                            .border_color(theme::hairline())
                            .children(Period::ALL.map(segment)),
                    ),
            )
            // Hairline under the header; it fills with the accent while line
            // counts stream in.
            .child(
                div()
                    .relative()
                    .h(px(1.))
                    .w_full()
                    .bg(theme::hairline())
                    .when_some(progress, |s, p| {
                        s.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .h_full()
                                .w(relative(p))
                                .bg(linear_gradient(
                                    90.,
                                    linear_color_stop(theme::alpha(theme::accent(), 0.), 0.),
                                    linear_color_stop(theme::accent(), 1.),
                                )),
                        )
                    }),
            )
            .into_any_element()
    }
}

// ---- Look ---------------------------------------------------------------

/// Text tokens: data colours never go on text.
fn ink() -> Hsla {
    theme::alpha(gpui_kit::white(), 0.93)
}
fn ink2() -> Hsla {
    theme::alpha(gpui_kit::white(), 0.66)
}
fn muted() -> Hsla {
    theme::alpha(gpui_kit::white(), 0.42)
}
/// Unfilled part of a meter, empty heatmap cells.
fn track() -> Hsla {
    theme::alpha(gpui_kit::white(), 0.055)
}

/// `a` → `b` in sRGB, `t` in 0..=1.
fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b): (Rgba, Rgba) = (a.into(), b.into());
    let l = |x: f32, y: f32| x + (y - x) * t;
    Rgba {
        r: l(a.r, b.r),
        g: l(a.g, b.g),
        b: l(a.b, b.b),
        a: l(a.a, b.a),
    }
    .into()
}

/// Sequential ramp of the accent over the card surface, five steps
/// (heatmap): quantised so neighbouring cells read as distinct levels.
fn ramp(level: usize) -> Hsla {
    let surface: Hsla = gpui_kit::rgb(0x1c1c1f).into();
    const STEPS: [f32; 5] = [0.34, 0.5, 0.66, 0.83, 1.];
    mix(surface, theme::accent(), STEPS[level.min(4)])
}

/// Meter fill: faint at the start, full accent at the tip, like a comet.
fn meter_fill() -> gpui_kit::Background {
    linear_gradient(
        90.,
        linear_color_stop(theme::alpha(theme::accent(), 0.18), 0.),
        linear_color_stop(theme::accent(), 1.),
    )
}

/// Liquid fill: rises, overshoots a little, then settles with a wobble.
fn liquid(t: f32) -> f32 {
    if t >= 1. {
        return 1.;
    }
    let wobble = 0.4 + 0.6 * (t * std::f32::consts::PI * 3.).cos();
    1. - (1. - t).powi(3) * wobble
}

fn compact(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// A card holding one block of the dashboard: title, a muted hint, the rows.
fn panel(
    key: &'static str,
    title: &'static str,
    hint: &'static str,
    count: usize,
    rows: Vec<AnyElement>,
) -> AnyElement {
    v_flex()
        .id(key)
        .w_full()
        .relative()
        .rounded_xl()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::alpha(gpui_kit::white(), 0.028))
        .overflow_hidden()
        // Light catching the top rim.
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
                .items_baseline()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(ink())
                        .child(title),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(muted())
                        .child(hint),
                )
                .when(count > TOP, |s| {
                    s.child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(muted())
                            .child(trf!("{TOP} sur {count}", "{TOP} of {count}")),
                    )
                }),
        )
        .child(v_flex().px_2().pb_2().children(rows))
        .into_any_element()
}

fn empty_state(icon: Lucide, text: String) -> AnyElement {
    v_flex()
        .w_full()
        .py_16()
        .gap_3()
        .items_center()
        .text_sm()
        .text_color(muted())
        .child(Icon::new(icon).large().text_color(muted()))
        .child(text)
        .into_any_element()
}

/// Placeholder while the first numbers load: the dashboard's shape, breathing.
fn skeleton() -> AnyElement {
    let block = |i: usize, h: f32| {
        div()
            .flex_1()
            .h(px(h))
            .rounded_xl()
            .bg(theme::alpha(gpui_kit::white(), 0.035))
            .with_animation(
                ("stats-skeleton", i),
                Animation::new(Duration::from_millis(1400)).repeat(),
                move |el, t| {
                    let wave = ((t + i as f32 * 0.1) * std::f32::consts::TAU).sin();
                    el.opacity(0.6 + 0.4 * wave)
                },
            )
    };
    v_flex()
        .gap_4()
        .child(h_flex().gap_3().children((0..4).map(|i| block(i, 92.))))
        .child(h_flex().child(block(4, 200.)))
        .child(h_flex().gap_4().child(block(5, 280.)).child(block(6, 280.)))
        .into_any_element()
}

/// Commits, people, lines added / removed, counting up.
fn tiles(stats: &RepoStats, seq: usize) -> AnyElement {
    let lines_known = stats.lines_scanned > 0;
    let spark: Vec<u32> = stats
        .activity
        .iter()
        .rev()
        .take(16)
        .rev()
        .copied()
        .collect();
    let tile = |id: usize,
                label: &'static str,
                dot: Option<Hsla>,
                value: Option<u64>,
                prefix: &'static str,
                extra: Option<AnyElement>| {
        v_flex()
            .flex_1()
            .min_w_0()
            .relative()
            .px_4()
            .pt_3()
            .pb_3()
            .gap_1()
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
                    .gap_1p5()
                    .text_xs()
                    .text_color(ink2())
                    .children(dot.map(|c| div().size(px(6.)).rounded_full().bg(c)))
                    .child(label),
            )
            .child(
                h_flex()
                    .items_end()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(28.))
                            .line_height(px(34.))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(ink())
                            .with_animation(
                                ("stats-tile", seq * 8 + id),
                                Animation::new(Duration::from_millis(900))
                                    .with_easing(gpui_kit::ease_out_quint()),
                                move |el, t| {
                                    el.child(match value {
                                        Some(v) => format!(
                                            "{prefix}{}",
                                            compact((v as f64 * t as f64).round() as u64)
                                        ),
                                        None => "—".into(),
                                    })
                                },
                            ),
                    )
                    .children(extra),
            )
    };
    let max = spark.iter().copied().max().unwrap_or(0).max(1);
    let n = spark.len();
    let sparkline = h_flex()
        .flex_none()
        .h(px(28.))
        .w(px(72.))
        .items_end()
        .gap(px(2.))
        .children(spark.into_iter().enumerate().map(|(i, v)| {
            let last = i + 1 == n;
            div()
                .flex_1()
                .h(relative((v as f32 / max as f32).max(0.06)))
                .rounded_t(px(1.5))
                .bg(if last {
                    theme::accent()
                } else {
                    theme::alpha(gpui_kit::white(), 0.18)
                })
        }))
        .into_any_element();
    h_flex()
        .gap_3()
        .child(tile(
            0,
            tr("Commits", "Commits"),
            None,
            Some(stats.commits as u64),
            "",
            Some(sparkline),
        ))
        .child(tile(
            1,
            tr("Contributeurs", "Contributors"),
            None,
            Some(stats.authors.len() as u64),
            "",
            None,
        ))
        .child(tile(
            2,
            tr("Lignes ajoutées", "Lines added"),
            Some(theme::lime()),
            lines_known.then_some(stats.additions),
            "+",
            None,
        ))
        .child(tile(
            3,
            tr("Lignes supprimées", "Lines removed"),
            Some(theme::red()),
            lines_known.then_some(stats.deletions),
            "−",
            None,
        ))
        .into_any_element()
}

/// Commits over the period: one hue, recessive grid, columns rising in turn.
fn activity(stats: &RepoStats, seq: usize) -> AnyElement {
    let max = stats.activity.iter().copied().max().unwrap_or(0).max(1);
    // A clean top for the scale: the next multiple of a round step (29 → 30).
    let top = {
        let step = match max {
            0..=4 => 2,
            5..=10 => 5,
            _ => {
                let p = 10u32.pow(max.ilog10());
                if max <= 2 * p { p / 2 } else { p }
            }
        };
        max.div_ceil(step) * step
    };
    let n = stats.activity.len().max(1);
    let bucket = stats.bucket_days;
    let today = chrono::Local::now().date_naive();
    let date = |ago: u32| {
        let d = today - chrono::Duration::days(ago as i64);
        d.format("%d/%m").to_string()
    };
    let bars = stats.activity.iter().enumerate().map(|(i, &v)| {
        let h = v as f32 / top as f32;
        let ago = (n - 1 - i) as u32 * bucket;
        let when = match (ago, bucket) {
            (0, 1) => tr("aujourd'hui", "today").to_string(),
            (_, 1) => date(ago),
            _ => trf!("semaine du {}", "week of {}", date(ago + bucket - 1)),
        };
        let tip = trf!("{v} commit(s) — {when}", "{v} commit(s) — {when}");
        div()
            .id(("stats-activity", i))
            .flex_1()
            .max_w(px(28.))
            .h_full()
            .flex()
            .items_end()
            .rounded_t(px(4.))
            .hover(|s| s.bg(theme::alpha(gpui_kit::white(), 0.04)))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .child(if v == 0 {
                div()
                    .w_full()
                    .h(px(2.))
                    .rounded_t(px(1.))
                    .bg(track())
                    .into_any_element()
            } else {
                div()
                    .w_full()
                    .min_h(px(3.))
                    .rounded_t(px(4.))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(theme::accent(), 0.),
                        linear_color_stop(theme::alpha(theme::accent(), 0.55), 1.),
                    ))
                    .with_animation(
                        ("stats-activity-bar", seq * 1024 + i),
                        Animation::new(Duration::from_millis(1100)),
                        move |el, t| {
                            // Staggered left to right, then a liquid settle.
                            let start = i as f32 / n as f32 * 0.45;
                            let t = ((t - start) / 0.55).clamp(0., 1.);
                            el.h(relative(h * liquid(t)))
                        },
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
                    .text_color(muted())
                    .child(label),
            )
    };
    let first = (n as u32 - 1) * bucket;
    v_flex()
        .px_2()
        .pt_2()
        .pb_1()
        .gap_1p5()
        .child(
            div()
                .relative()
                .ml(px(28.))
                .mr_2()
                .h(px(150.))
                .child(gridline(1., top.to_string()))
                .child(gridline(0.5, (top / 2).to_string()))
                .child(gridline(0., "0".into()))
                .child(
                    h_flex()
                        .absolute()
                        .inset_0()
                        .items_end()
                        .justify_between()
                        .gap(px(3.))
                        .children(bars),
                ),
        )
        .child(
            h_flex()
                .ml(px(28.))
                .mr_2()
                .text_xs()
                .text_color(muted())
                .child(div().flex_1().child(date(first)))
                .child(tr("aujourd'hui", "today")),
        )
        .into_any_element()
}

/// A leaderboard row: rank, avatar, name over a thin meter, count.
fn person_row(
    key: &'static str,
    i: usize,
    p: &Person,
    max: u32,
    lines: bool,
    seq: usize,
    cx: &mut Context<RepoView>,
) -> AnyElement {
    let tip = format!("{} <{}>", p.name, p.email);
    let extra = (lines && (p.additions > 0 || p.deletions > 0)).then(|| {
        let dot = |c: Hsla| div().size(px(5.)).rounded_full().bg(c);
        h_flex()
            .flex_none()
            .w(px(104.))
            .justify_end()
            .gap_1()
            .text_xs()
            .text_color(muted())
            .child(dot(theme::lime()))
            .child(compact(p.additions))
            .child(div().w(px(2.)))
            .child(dot(theme::red()))
            .child(compact(p.deletions))
            .into_any_element()
    });
    leaderboard_row(
        SharedString::from(format!("{key}-{i}")),
        i,
        p.commits as f32 / max as f32,
        seq,
        crate::avatars::avatar(&p.email, &p.name, 22., cx),
        div()
            .truncate()
            .text_color(ink())
            .when(i == 0, |s| s.font_weight(gpui_kit::FontWeight::SEMIBOLD))
            .child(p.name.clone())
            .into_any_element(),
        p.commits,
        extra,
        tip,
    )
}

fn file_row(i: usize, f: &FileHeat, max: u32, seq: usize) -> AnyElement {
    let (dir, name) = match f.path.rsplit_once('/') {
        Some((d, n)) => (Some(format!("{d}/")), n.to_string()),
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
    leaderboard_row(
        SharedString::from(format!("stats-file-{i}")),
        i,
        f.commits as f32 / max as f32,
        seq,
        div()
            .size(px(22.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .bg(theme::alpha(gpui_kit::white(), 0.05))
            .child(Icon::new(Lucide::FileCode).xsmall().text_color(ink2()))
            .into_any_element(),
        h_flex()
            .min_w_0()
            .child(div().min_w_0().truncate().text_color(muted()).children(dir))
            .child(div().flex_none().text_color(ink()).child(name))
            .into_any_element(),
        f.commits,
        None,
        tip,
    )
}

/// Shared leaderboard row. The meter fills like a liquid: a quick rise with a
/// small overshoot and wobble, rows following one another.
#[allow(clippy::too_many_arguments)]
fn leaderboard_row(
    id: SharedString,
    rank: usize,
    frac: f32,
    seq: usize,
    leading: AnyElement,
    label: AnyElement,
    count: u32,
    extra: Option<AnyElement>,
    tip: String,
) -> AnyElement {
    let delay = (rank as f32 * 0.08).min(0.5);
    h_flex()
        .id(id.clone())
        .test_support()
        .h(px(42.))
        .px_2()
        .gap_3()
        .min_w_0()
        .rounded_lg()
        .text_sm()
        .hover(|s| s.bg(theme::alpha(gpui_kit::white(), 0.04)))
        .child(
            div()
                .flex_none()
                .w(px(14.))
                .text_xs()
                .text_color(muted())
                .child((rank + 1).to_string()),
        )
        .child(leading)
        .child(
            v_flex().flex_1().min_w_0().gap(px(5.)).child(label).child(
                div()
                    .relative()
                    .h(px(4.))
                    .w_full()
                    .rounded_full()
                    .bg(track())
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .h_full()
                            .rounded_full()
                            .bg(meter_fill())
                            .with_animation(
                                SharedString::from(format!("{id}-fill-{seq}")),
                                Animation::new(Duration::from_millis(1200)),
                                move |el, t| {
                                    let t = ((t - delay) / (1. - delay)).clamp(0., 1.);
                                    el.w(relative((frac * liquid(t)).max(0.015)))
                                },
                            ),
                    ),
            ),
        )
        .children(extra)
        .child(
            div()
                .flex_none()
                .min_w(px(32.))
                .flex()
                .justify_end()
                .text_sm()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(ink())
                .child(count.to_string()),
        )
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .into_any_element()
}

/// Weekday × hour heatmap in the authors' own time zones, five accent levels.
fn punchcard(stats: &RepoStats, seq: usize) -> AnyElement {
    let max = stats
        .punchcard
        .iter()
        .flatten()
        .copied()
        .max()
        .unwrap_or(0)
        .max(1);
    let level = |v: u32| ((v as f32 / max as f32 * 5.).ceil() as usize).clamp(1, 5) - 1;
    let days = if crate::i18n::is_fr() {
        ["Lun", "Mar", "Mer", "Jeu", "Ven", "Sam", "Dim"]
    } else {
        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
    };
    const LABEL: f32 = 34.;
    let rows = stats.punchcard.iter().enumerate().map(|(d, hours)| {
        h_flex()
            .gap(px(3.))
            .child(
                div()
                    .w(px(LABEL))
                    .flex_none()
                    .text_xs()
                    .text_color(muted())
                    .child(days[d]),
            )
            .children(hours.iter().enumerate().map(move |(h, &v)| {
                let tip = trf!(
                    "{} {}h — {v} commit(s)",
                    "{} {}:00 — {v} commit(s)",
                    days[d],
                    h
                );
                div()
                    .id(("stats-punch", d * 24 + h))
                    .flex_1()
                    .h(px(18.))
                    .rounded(px(3.))
                    .bg(if v == 0 { track() } else { ramp(level(v)) })
                    .hover(|s| {
                        s.border_1()
                            .border_color(theme::alpha(gpui_kit::white(), 0.5))
                    })
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .with_animation(
                        ("stats-punch-in", seq * 256 + d * 24 + h),
                        Animation::new(Duration::from_millis(900)),
                        move |el, t| {
                            // A wave sweeping across the hours.
                            let start = (h as f32 / 24. * 0.5) + d as f32 * 0.03;
                            el.opacity(((t - start) / 0.4).clamp(0., 1.))
                        },
                    )
            }))
    });
    let hours = h_flex()
        .gap(px(3.))
        .text_xs()
        .text_color(muted())
        .child(div().w(px(LABEL)).flex_none())
        .children((0..24).map(|h| {
            div()
                .flex_1()
                .when(h % 6 == 0, |s| s.child(format!("{h}h")))
        }));
    let legend = h_flex()
        .gap_1()
        .text_xs()
        .text_color(muted())
        .child(tr("Moins", "Less"))
        .child(div().size(px(11.)).rounded(px(2.)).bg(track()))
        .children((0..5).map(|l| div().size(px(11.)).rounded(px(2.)).bg(ramp(l))))
        .child(tr("Plus", "More"));
    v_flex()
        .px_2()
        .pb_2()
        .gap(px(3.))
        .children(rows)
        .child(div().pt_1().child(hours))
        .child(h_flex().pt_2().child(div().flex_1()).child(legend))
        .into_any_element()
}
