//! Weekly recap, opened from the overview: animated slides over the window on
//! what the week added up to across the workspace's repositories, for me or
//! the whole team, each number against the same point of the week before.
//! The last slide exports a summary image (saved and copied).

pub mod png;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike as _, Local, TimeZone as _, Timelike as _};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, AppContext as _, ClipboardItem, Context,
    EventEmitter, FocusHandle, Hsla, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Task,
    Window, div, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px, relative,
};
use mugetsu_git::{Activity, Me, Repo, TimeSpan, Week};

use crate::i18n::tr;
use crate::theme;

gpui_kit::actions!(recap, [NextSlide, PrevSlide, CloseRecap, ToggleTeam]);

pub fn init(cx: &mut gpui_kit::App) {
    let ctx = Some("Recap");
    cx.bind_keys([
        KeyBinding::new("right", NextSlide, ctx),
        KeyBinding::new("space", NextSlide, ctx),
        KeyBinding::new("enter", NextSlide, ctx),
        KeyBinding::new("left", PrevSlide, ctx),
        KeyBinding::new("escape", CloseRecap, ctx),
        KeyBinding::new("tab", ToggleTeam, ctx),
    ]);
}

const DAY: i64 = 86_400;

/// A merged pull request, for the counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergedPr {
    /// Unix seconds.
    pub at: i64,
    pub mine: bool,
}

/// One repository of the workspace, as the recap reads it.
#[derive(Clone)]
pub struct Source {
    pub name: String,
    pub repo: Repo,
    pub merged: Vec<MergedPr>,
    /// Forge login of the user there.
    pub login: Option<String>,
}

/// This week so far, and the same stretch of the week before: Monday 00:00
/// local to `now`, both.
pub fn spans(now: DateTime<Local>) -> (TimeSpan, TimeSpan) {
    let monday =
        now.date_naive() - chrono::Duration::days(now.weekday().num_days_from_monday().into());
    let start = Local
        .from_local_datetime(&monday.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .map_or(now.timestamp() - 7 * DAY, |t| t.timestamp());
    let end = now.timestamp() + 1;
    (
        TimeSpan { start, end },
        TimeSpan {
            start: start - 7 * DAY,
            end: end - 7 * DAY,
        },
    )
}

/// One audience's numbers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Numbers {
    pub week: Week,
    pub prev: Week,
    pub prs: u32,
    pub prs_prev: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Computed {
    pub span: TimeSpan,
    pub mine: Numbers,
    pub team: Numbers,
    /// Repositories looked at.
    pub repos: usize,
    /// Days of this week so far (today included).
    pub days: usize,
}

impl Computed {
    pub fn numbers(&self, team: bool) -> &Numbers {
        if team { &self.team } else { &self.mine }
    }
}

/// Adds up `repos` (name, commits) and the merged PRs for both audiences.
pub fn compute(
    repos: &[(String, Vec<Activity>)],
    merged: &[MergedPr],
    me: &Me,
    (span, prev): (TimeSpan, TimeSpan),
    utc_offset: i64,
) -> Computed {
    let count = |s: TimeSpan, mine: bool| {
        merged
            .iter()
            .filter(|p| (s.start..s.end).contains(&p.at) && (!mine || p.mine))
            .count() as u32
    };
    let numbers = |me: Option<&Me>| Numbers {
        week: mugetsu_git::recap::week(repos, span, utc_offset, me),
        prev: mugetsu_git::recap::week(repos, prev, utc_offset, me),
        prs: count(span, me.is_some()),
        prs_prev: count(prev, me.is_some()),
    };
    let first = (span.start + utc_offset).div_euclid(DAY);
    let today = (span.end - 1 + utc_offset).div_euclid(DAY);
    Computed {
        span,
        mine: numbers(Some(me)),
        team: numbers(None),
        repos: repos.len(),
        days: ((today - first + 1).clamp(1, 7)) as usize,
    }
}

/// Reads every source (git, off the UI thread) and adds the week up.
fn load(sources: Vec<Source>, now: DateTime<Local>) -> anyhow::Result<Computed> {
    let (span, prev) = spans(now);
    let offset = now.offset().local_minus_utc() as i64;
    let global = mugetsu_git::global_identity();
    let mut ids: Vec<(String, String)> = vec![(global.name, global.email)];
    let mut repos = Vec::new();
    let mut merged = Vec::new();
    for s in &sources {
        let get = |key: &str| {
            s.repo
                .git()
                .run(["config", "--get", key])
                .map(|v| v.trim().to_string())
                .unwrap_or_default()
        };
        ids.push((get("user.name"), get("user.email")));
        if let Some(login) = &s.login {
            ids.push((login.clone(), String::new()));
        }
        repos.push((s.name.clone(), s.repo.activity(prev.start)?));
        merged.extend(s.merged.iter().copied());
    }
    ids.retain(|(n, e)| !n.is_empty() || !e.is_empty());
    let me = Me::new(ids.iter().map(|(n, e)| (n.as_str(), e.as_str())));
    Ok(compute(&repos, &merged, &me, (span, prev), offset))
}

/// `cur` against `prev`, in percent; `None` when there was nothing before.
pub fn delta_pct(cur: u64, prev: u64) -> Option<i64> {
    (prev > 0).then(|| ((cur as f64 - prev as f64) / prev as f64 * 100.).round() as i64)
}

/// "↑ 32 %", "nouveau", "= stable"… and its colour.
fn trend(cur: u64, prev: u64) -> (String, Hsla) {
    match delta_pct(cur, prev) {
        None if cur == 0 => (String::new(), muted()),
        None => (tr("nouveau", "new").into(), theme::lime()),
        Some(0) => (tr("= pareil", "= same").into(), muted()),
        Some(p) if p > 0 => (format!("↑ {p} %"), theme::lime()),
        Some(p) => (format!("↓ {} %", -p), theme::red()),
    }
}

pub fn day_names() -> [&'static str; 7] {
    if crate::i18n::is_fr() {
        [
            "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche",
        ]
    } else {
        [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ]
    }
}

fn day_letters() -> [&'static str; 7] {
    if crate::i18n::is_fr() {
        ["L", "M", "M", "J", "V", "S", "D"]
    } else {
        ["M", "T", "W", "T", "F", "S", "S"]
    }
}

/// "13 – 19 oct." style range of the week.
pub fn week_label(span: TimeSpan) -> String {
    let at = |t: i64| Local.timestamp_opt(t, 0).single();
    let (Some(a), Some(b)) = (at(span.start), at(span.start + 6 * DAY)) else {
        return String::new();
    };
    let months_fr = [
        "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
        "déc.",
    ];
    let months_en = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = |d: &DateTime<Local>| {
        let i = d.month0() as usize;
        if crate::i18n::is_fr() {
            months_fr[i]
        } else {
            months_en[i]
        }
    };
    if crate::i18n::is_fr() {
        format!("{} {} – {} {}", a.day(), month(&a), b.day(), month(&b))
    } else {
        format!("{} {} – {} {}", month(&a), a.day(), month(&b), b.day())
    }
}

/// ISO week key, so the Friday reminder shows once a week.
pub fn week_key(now: DateTime<Local>) -> String {
    let w = now.iso_week();
    format!("{}-W{:02}", w.year(), w.week())
}

/// The Friday reminder is due: Friday from 16:00 through the weekend, once per week.
pub fn reminder_due(now: DateTime<Local>, last_shown: &str) -> bool {
    let day = now.weekday().num_days_from_monday();
    let late = day > 4 || (day == 4 && now.hour() >= 16);
    late && week_key(now) != last_shown
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slide {
    Intro,
    Commits,
    Lines,
    Rhythm,
    Places,
    People,
    Summary,
}

fn slides(n: &Numbers, team: bool) -> Vec<Slide> {
    if n.week.commits == 0 {
        return vec![Slide::Intro, Slide::Summary];
    }
    let mut out = vec![
        Slide::Intro,
        Slide::Commits,
        Slide::Lines,
        Slide::Rhythm,
        Slide::Places,
    ];
    if (team && n.week.people.len() > 1) || n.prs > 0 {
        out.push(Slide::People);
    }
    out.push(Slide::Summary);
    out
}

pub enum RecapEvent {
    Closed,
}

enum State {
    Loading,
    Ready(Arc<Computed>),
    Failed(String),
}

pub struct Recap {
    workspace: String,
    state: State,
    team: bool,
    slide: usize,
    /// Bumped on every slide or audience change: replays the entrance.
    seq: usize,
    focus: FocusHandle,
    _load: Task<()>,
}

impl EventEmitter<RecapEvent> for Recap {}

impl Recap {
    pub fn new(workspace: String, sources: Vec<Source>, cx: &mut Context<Self>) -> Self {
        let now = Local::now();
        let load = cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { load(sources, now) }).await;
            let _ = this.update(cx, |this, cx| {
                this.state = match result {
                    Ok(c) => State::Ready(Arc::new(c)),
                    Err(e) => State::Failed(format!("{e:#}")),
                };
                this.seq += 1;
                cx.notify();
            });
        });
        Self {
            workspace,
            state: State::Loading,
            team: false,
            slide: 0,
            seq: 0,
            focus: cx.focus_handle(),
            _load: load,
        }
    }

    #[cfg(test)]
    pub fn with_data(workspace: String, data: Computed, cx: &mut Context<Self>) -> Self {
        Self {
            workspace,
            state: State::Ready(Arc::new(data)),
            team: false,
            slide: 0,
            seq: 0,
            focus: cx.focus_handle(),
            _load: Task::ready(()),
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn data(&self) -> Option<Arc<Computed>> {
        match &self.state {
            State::Ready(c) => Some(c.clone()),
            _ => None,
        }
    }

    fn count(&self) -> usize {
        self.data()
            .map_or(1, |d| slides(d.numbers(self.team), self.team).len())
    }

    #[cfg(test)]
    pub fn computed(&self) -> Option<Arc<Computed>> {
        self.data()
    }

    #[cfg(test)]
    pub fn position(&self) -> (usize, usize, bool) {
        (self.slide, self.count(), self.team)
    }

    pub fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let n = self.count();
        let next = if forward {
            (self.slide + 1).min(n - 1)
        } else {
            self.slide.saturating_sub(1)
        };
        if next != self.slide {
            self.slide = next;
            self.seq += 1;
            cx.notify();
        }
    }

    pub fn set_team(&mut self, team: bool, cx: &mut Context<Self>) {
        if self.team == team {
            return;
        }
        // Same slide when the other audience has it, else the closest one.
        let current = self.data().and_then(|d| {
            slides(d.numbers(self.team), self.team)
                .get(self.slide)
                .copied()
        });
        self.team = team;
        if let (Some(d), Some(cur)) = (self.data(), current) {
            let list = slides(d.numbers(team), team);
            self.slide = list
                .iter()
                .position(|s| *s == cur)
                .unwrap_or(self.slide.min(list.len() - 1));
        }
        self.seq += 1;
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(RecapEvent::Closed);
    }

    /// The summary image: saved to Downloads and copied to the clipboard.
    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(data) = self.data() else {
            return;
        };
        let bytes = match png::render(&data, self.team, &self.workspace) {
            Ok(b) => b,
            Err(e) => {
                window.push_notification(
                    crate::toast::error(tr("Export impossible", "Export failed"), &e, None),
                    cx,
                );
                return;
            }
        };
        cx.write_to_clipboard(ClipboardItem::new_image(&gpui_kit::Image::from_bytes(
            gpui_kit::ImageFormat::Png,
            bytes.clone(),
        )));
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let name = format!(
            "mugetsu-recap-{}{}.png",
            week_key(Local::now()),
            if self.team { "-team" } else { "" }
        );
        let path: PathBuf = dir.join(name);
        match std::fs::write(&path, &bytes) {
            Ok(()) => window.push_notification(
                Notification::success(trf!(
                    "Image copiée et enregistrée dans {}",
                    "Image copied and saved to {}",
                    path.display()
                ))
                .title(tr("Recap exporté", "Recap exported")),
                cx,
            ),
            Err(e) => window.push_notification(
                Notification::warning(trf!(
                    "Image copiée, mais pas enregistrée : {e}",
                    "Image copied, but not saved: {e}"
                )),
                cx,
            ),
        }
    }
}

// ---- Look ---------------------------------------------------------------

fn ink() -> Hsla {
    theme::alpha(gpui_kit::white(), 0.95)
}
fn ink2() -> Hsla {
    theme::alpha(gpui_kit::white(), 0.68)
}
fn muted() -> Hsla {
    theme::alpha(gpui_kit::white(), 0.42)
}
fn bronze() -> Hsla {
    gpui_kit::rgb(0xc8875a).into()
}

fn compact(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// A big number counting up to `value`.
fn counter(
    key: (&'static str, usize),
    value: u64,
    prefix: &'static str,
    size: f32,
    color: Hsla,
) -> AnyElement {
    div()
        .text_size(px(size))
        .line_height(px(size * 1.1))
        .font_weight(gpui_kit::FontWeight::BOLD)
        .text_color(color)
        .with_animation(
            key,
            Animation::new(Duration::from_millis(1100)).with_easing(gpui_kit::ease_out_quint()),
            move |el, t| {
                el.child(format!(
                    "{prefix}{}",
                    compact((value as f64 * t as f64).round() as u64)
                ))
            },
        )
        .into_any_element()
}

fn trend_chip(cur: u64, prev: u64) -> Option<AnyElement> {
    let (text, color) = trend(cur, prev);
    (!text.is_empty()).then(|| {
        h_flex()
            .gap_1p5()
            .items_baseline()
            .child(
                div()
                    .px_2()
                    .py_0p5()
                    .rounded_full()
                    .bg(theme::alpha(color, 0.14))
                    .text_sm()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(color)
                    .child(text),
            )
            .child(div().text_xs().text_color(muted()).child(trf!(
                "vs {} la semaine dernière",
                "vs {} last week",
                prev
            )))
            .into_any_element()
    })
}

fn kicker(text: impl Into<SharedString>) -> AnyElement {
    div()
        .text_sm()
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .text_color(theme::accent())
        .child(text.into())
        .into_any_element()
}

fn headline(text: impl Into<SharedString>) -> AnyElement {
    div()
        .text_size(px(30.))
        .line_height(px(38.))
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .text_color(ink())
        .child(text.into())
        .into_any_element()
}

/// Columns rising one after the other; `dim_from` greys the days to come.
fn columns(
    key: &'static str,
    seq: usize,
    values: &[u32],
    labels: &[&'static str],
    dim_from: usize,
    height: f32,
) -> AnyElement {
    let max = values.iter().copied().max().unwrap_or(0).max(1) as f32;
    let peak = values.iter().copied().max().unwrap_or(0);
    h_flex()
        .w_full()
        .items_end()
        .gap_2()
        .children(values.iter().enumerate().map(|(i, &v)| {
            let h = (v as f32 / max).max(0.03);
            let future = i >= dim_from;
            let top = v == peak && v > 0;
            v_flex()
                .flex_1()
                .items_center()
                .gap_1p5()
                .child(
                    div()
                        .text_xs()
                        .text_color(if future {
                            theme::alpha(gpui_kit::white(), 0.2)
                        } else {
                            ink2()
                        })
                        .child(if future || v == 0 {
                            String::new()
                        } else {
                            v.to_string()
                        }),
                )
                .child(
                    div().w_full().h(px(height)).flex().items_end().child(
                        div()
                            .w_full()
                            .rounded_t(px(4.))
                            .bg(if future {
                                theme::alpha(gpui_kit::white(), 0.05).into()
                            } else if top {
                                linear_gradient(
                                    0.,
                                    linear_color_stop(theme::alpha(theme::accent(), 0.5), 0.),
                                    linear_color_stop(theme::accent(), 1.),
                                )
                            } else {
                                theme::alpha(gpui_kit::white(), 0.22).into()
                            })
                            .h(px(0.))
                            .with_animation(
                                (key, seq * 64 + i),
                                Animation::new(Duration::from_millis(700 + i as u64 * 70))
                                    .with_easing(gpui_kit::ease_out_quint()),
                                move |el, t| {
                                    let delay = (i as f32 * 0.06).min(0.5);
                                    let t = ((t - delay) / (1. - delay)).clamp(0., 1.);
                                    el.h(px(height * h * t))
                                },
                            ),
                    ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted())
                        .child(labels.get(i).copied().unwrap_or("")),
                )
        }))
        .into_any_element()
}

/// Horizontal bars (repositories, files), longest first.
fn bars(key: &'static str, seq: usize, rows: &[(String, u32)]) -> AnyElement {
    let max = rows.first().map_or(1, |r| r.1).max(1) as f32;
    v_flex()
        .w_full()
        .gap_2()
        .children(rows.iter().enumerate().map(|(i, (label, n))| {
            let w = *n as f32 / max;
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .text_sm()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(ink2())
                                .child(label.clone()),
                        )
                        .child(
                            div()
                                .text_color(ink())
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .child(n.to_string()),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(6.))
                        .rounded_full()
                        .bg(theme::alpha(gpui_kit::white(), 0.06))
                        .child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(linear_gradient(
                                    90.,
                                    linear_color_stop(theme::alpha(theme::accent(), 0.3), 0.),
                                    linear_color_stop(theme::accent(), 1.),
                                ))
                                .w(relative(0.))
                                .with_animation(
                                    (key, seq * 16 + i),
                                    Animation::new(Duration::from_millis(800 + i as u64 * 90))
                                        .with_easing(gpui_kit::ease_out_quint()),
                                    move |el, t| el.w(relative(w * t)),
                                ),
                        ),
                )
        }))
        .into_any_element()
}

/// A labelled number with its trend, for the summary.
fn stat(
    key: usize,
    seq: usize,
    label: &'static str,
    cur: u64,
    prev: u64,
    prefix: &'static str,
) -> AnyElement {
    let (text, color) = trend(cur, prev);
    v_flex()
        .flex_1()
        .min_w_0()
        .p_4()
        .gap_1()
        .rounded_xl()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::alpha(gpui_kit::white(), 0.04))
        .child(div().text_xs().text_color(muted()).child(label))
        .child(counter(
            ("recap-stat", seq * 8 + key),
            cur,
            prefix,
            30.,
            ink(),
        ))
        .child(
            div()
                .h(px(16.))
                .text_xs()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(color)
                .child(text),
        )
        .into_any_element()
}

impl Recap {
    fn render_slide(&self, slide: Slide, data: &Computed, cx: &mut Context<Self>) -> AnyElement {
        let n = data.numbers(self.team);
        let (w, p) = (&n.week, &n.prev);
        let seq = self.seq;
        let team = self.team;
        match slide {
            Slide::Intro => {
                let title = if team {
                    tr("La semaine de l'équipe", "The team's week")
                } else {
                    tr("Ta semaine", "Your week")
                };
                let line = if w.commits == 0 {
                    tr(
                        "Pas encore de commit cette semaine. Le meilleur moment pour commencer, c'est maintenant.",
                        "No commits yet this week. Best time to start is now.",
                    )
                    .to_string()
                } else if team {
                    trf!(
                        "{} commits par {} personne(s) sur {} dépôt(s).",
                        "{} commits by {} people across {} repositories.",
                        w.commits,
                        w.people.len(),
                        w.repos.len()
                    )
                } else {
                    trf!(
                        "{} commits sur {} dépôt(s), {} jour(s) actif(s).",
                        "{} commits across {} repositories, {} active day(s).",
                        w.commits,
                        w.repos.len(),
                        w.active_days()
                    )
                };
                v_flex()
                    .gap_3()
                    .items_center()
                    .text_center()
                    .child(
                        Icon::new(Lucide::Sparkles)
                            .large()
                            .text_color(theme::accent()),
                    )
                    .child(kicker(format!(
                        "{} · {}",
                        self.workspace,
                        week_label(data.span)
                    )))
                    .child(
                        div()
                            .text_size(px(48.))
                            .line_height(px(56.))
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .text_color(ink())
                            .child(title),
                    )
                    .child(
                        div()
                            .max_w(px(520.))
                            .text_lg()
                            .text_color(ink2())
                            .child(line),
                    )
                    .into_any_element()
            }
            Slide::Commits => v_flex()
                .gap_4()
                .child(kicker(tr("Commits", "Commits")))
                .child(
                    h_flex()
                        .items_end()
                        .gap_4()
                        .child(counter(
                            ("recap-commits", seq),
                            w.commits.into(),
                            "",
                            96.,
                            ink(),
                        ))
                        .child(
                            div()
                                .pb_4()
                                .children(trend_chip(w.commits.into(), p.commits.into())),
                        ),
                )
                .child(columns(
                    "recap-days",
                    seq,
                    &w.days,
                    &day_letters(),
                    data.days,
                    150.,
                ))
                .into_any_element(),
            Slide::Lines => v_flex()
                .gap_5()
                .child(kicker(tr("Lignes", "Lines")))
                .child(
                    h_flex()
                        .gap_10()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(counter(
                                    ("recap-add", seq),
                                    w.additions,
                                    "+",
                                    72.,
                                    theme::lime(),
                                ))
                                .children(trend_chip(w.additions, p.additions)),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .child(counter(
                                    ("recap-del", seq),
                                    w.deletions,
                                    "−",
                                    72.,
                                    theme::red(),
                                ))
                                .children(trend_chip(w.deletions, p.deletions)),
                        ),
                )
                .child(div().text_lg().text_color(ink2()).child(trf!(
                    "{} fichier(s) touché(s){}",
                    "{} file(s) touched{}",
                    w.files,
                    if w.deletions > w.additions {
                        tr(
                            " — plus de lignes retirées qu'ajoutées : beau ménage.",
                            " — more lines removed than added: nice cleanup.",
                        )
                    } else {
                        ""
                    }
                )))
                .into_any_element(),
            Slide::Rhythm => {
                let names = day_names();
                let busiest = w.busiest_day().map(|d| names[d]).unwrap_or("—");
                let hour = w
                    .peak_hour()
                    .map_or("—".to_string(), |h| format!("{h}h–{}h", h + 1));
                let hours: Vec<u32> = w.hours.to_vec();
                let labels: Vec<&'static str> = (0..24)
                    .map(|h| match h {
                        0 => "0h",
                        6 => "6h",
                        12 => "12h",
                        18 => "18h",
                        _ => "",
                    })
                    .collect();
                v_flex()
                    .gap_4()
                    .child(kicker(tr("Rythme", "Rhythm")))
                    .child(headline(trf!(
                        "Jour fort : {busiest}. Heure de pointe : {hour}.",
                        "Busiest day: {busiest}. Peak hour: {hour}.",
                    )))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_baseline()
                            .child(counter(
                                ("recap-active", seq),
                                w.active_days().into(),
                                "",
                                40.,
                                ink(),
                            ))
                            .child(div().text_color(ink2()).child(trf!(
                                "jour(s) actif(s) sur {} · {} la semaine dernière",
                                "active day(s) of {} · {} last week",
                                data.days,
                                p.active_days()
                            ))),
                    )
                    .child(columns("recap-hours", seq, &hours, &labels, 24, 90.))
                    .into_any_element()
            }
            Slide::Places => {
                let repos: Vec<(String, u32)> = w.repos.iter().take(6).cloned().collect();
                let files: Vec<(String, u32)> = w.hot_files.iter().take(4).cloned().collect();
                v_flex()
                    .gap_4()
                    .child(kicker(tr("Où ça a bougé", "Where it moved")))
                    .child(headline(match w.repos.first() {
                        Some((r, n)) => trf!(
                            "{r} mène avec {n} commit(s).",
                            "{r} leads with {n} commit(s)."
                        ),
                        None => String::new(),
                    }))
                    .child(bars("recap-repos", seq, &repos))
                    .when(!files.is_empty(), |s| {
                        s.child(
                            v_flex()
                                .pt_2()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(muted())
                                        .child(tr("Fichiers chauds", "Hot files")),
                                )
                                .children(files.into_iter().map(|(f, c)| {
                                    h_flex()
                                        .gap_2()
                                        .text_sm()
                                        .child(
                                            Icon::new(Lucide::Flame)
                                                .xsmall()
                                                .text_color(theme::amber()),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(ink2())
                                                .child(f),
                                        )
                                        .child(div().text_color(muted()).child(c.to_string()))
                                })),
                        )
                    })
                    .into_any_element()
            }
            Slide::People => {
                // The team: how many committed (and the PRs merged); me: my PRs merged.
                let big: (u64, u64, String) = if team {
                    let label = if n.prs > 0 {
                        trf!(
                            "contributeur(s) · {} PR mergée(s)",
                            "contributor(s) · {} PR(s) merged",
                            n.prs
                        )
                    } else {
                        tr("contributeur(s)", "contributor(s)").to_string()
                    };
                    (w.people.len() as u64, p.people.len() as u64, label)
                } else {
                    (
                        n.prs.into(),
                        n.prs_prev.into(),
                        tr("de tes PR mergées", "of your PRs merged").to_string(),
                    )
                };
                let podium = team.then(|| {
                    let top: Vec<_> = w.people.iter().take(3).cloned().collect();
                    // Second, first, third, like a podium.
                    let order = [1usize, 0, 2];
                    h_flex()
                        .w_full()
                        .items_end()
                        .justify_center()
                        .gap_3()
                        .children(order.into_iter().filter_map(|rank| {
                            let person = top.get(rank)?.clone();
                            let height = [150., 110., 80.][rank];
                            let medal = [
                                theme::amber(),
                                theme::alpha(gpui_kit::white(), 0.7),
                                bronze(),
                            ][rank];
                            Some(
                                v_flex()
                                    .w(px(150.))
                                    .items_center()
                                    .gap_1p5()
                                    .child(Icon::new(Lucide::Trophy).text_color(medal))
                                    .child(
                                        div()
                                            .max_w(px(150.))
                                            .truncate()
                                            .text_sm()
                                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                            .text_color(ink())
                                            .child(person.name.clone()),
                                    )
                                    .child(div().text_xs().text_color(muted()).child(trf!(
                                        "{} commits",
                                        "{} commits",
                                        person.commits
                                    )))
                                    .child(
                                        div()
                                            .w_full()
                                            .rounded_t(px(8.))
                                            .border_1()
                                            .border_color(theme::alpha(medal, 0.5))
                                            .bg(theme::alpha(medal, 0.14))
                                            .h(px(0.))
                                            .with_animation(
                                                ("recap-podium", seq * 4 + rank),
                                                Animation::new(Duration::from_millis(
                                                    900 + rank as u64 * 150,
                                                ))
                                                .with_easing(gpui_kit::ease_out_quint()),
                                                move |el, t| el.h(px(height * t)),
                                            ),
                                    ),
                            )
                        }))
                });
                v_flex()
                    .gap_5()
                    .child(kicker(if team {
                        tr("L'équipe", "The team")
                    } else {
                        tr("Pull requests", "Pull requests")
                    }))
                    .child(
                        h_flex()
                            .gap_4()
                            .items_end()
                            .child(counter(("recap-people", seq), big.0, "", 72., ink()))
                            .child(
                                v_flex()
                                    .pb_3()
                                    .gap_1()
                                    .child(div().text_lg().text_color(ink2()).child(big.2))
                                    .children(trend_chip(big.0, big.1)),
                            ),
                    )
                    .children(podium)
                    .into_any_element()
            }
            Slide::Summary => {
                let stats = h_flex()
                    .w_full()
                    .gap_3()
                    .child(stat(
                        0,
                        seq,
                        tr("Commits", "Commits"),
                        w.commits.into(),
                        p.commits.into(),
                        "",
                    ))
                    .child(stat(
                        1,
                        seq,
                        tr("Lignes ajoutées", "Lines added"),
                        w.additions,
                        p.additions,
                        "+",
                    ))
                    .child(stat(
                        2,
                        seq,
                        tr("Jours actifs", "Active days"),
                        w.active_days().into(),
                        p.active_days().into(),
                        "",
                    ))
                    .child(stat(
                        3,
                        seq,
                        tr("PR mergées", "PRs merged"),
                        n.prs.into(),
                        n.prs_prev.into(),
                        "",
                    ));
                v_flex()
                    .gap_5()
                    .child(kicker(format!(
                        "{} · {}",
                        self.workspace,
                        week_label(data.span)
                    )))
                    .child(headline(if team {
                        tr(
                            "La semaine de l'équipe, en bref",
                            "The team's week, in short",
                        )
                    } else {
                        tr("Ta semaine, en bref", "Your week, in short")
                    }))
                    .child(stats)
                    .child(columns(
                        "recap-sum-days",
                        seq,
                        &w.days,
                        &day_letters(),
                        data.days,
                        70.,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("recap-export")
                                    .primary()
                                    .icon(Icon::new(Lucide::ImageDown))
                                    .label(tr(
                                        "Copier et enregistrer l'image",
                                        "Copy and save image",
                                    ))
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.export(window, cx)),
                                    ),
                            )
                            .child(div().text_xs().text_color(muted()).child(tr(
                                "Une image résumé, à coller dans Slack.",
                                "A summary image, ready to paste in Slack.",
                            ))),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_top(&self, count: usize, cx: &mut Context<Self>) -> AnyElement {
        let segment =
            |team: bool, label: &'static str, id: &'static str, cx: &mut Context<Self>| {
                let active = self.team == team;
                div()
                    .id(id)
                    .test_support()
                    .h(px(26.))
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
                            .text_color(ink())
                    })
                    .when(!active, |s| {
                        s.text_color(muted()).hover(|s| s.text_color(ink()))
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_team(team, cx)))
            };
        let progress = h_flex().flex_1().gap_1().children((0..count).map(|i| {
            let done = i < self.slide;
            let current = i == self.slide;
            let fill = div().h_full().rounded_full().bg(ink());
            let fill = if current {
                fill.with_animation(
                    ("recap-progress", self.seq),
                    Animation::new(Duration::from_millis(500))
                        .with_easing(gpui_kit::ease_out_quint()),
                    |el, t| el.w(relative(t)),
                )
                .into_any_element()
            } else {
                fill.w(relative(if done { 1. } else { 0. }))
                    .into_any_element()
            };
            div()
                .flex_1()
                .h(px(3.))
                .rounded_full()
                .bg(theme::alpha(gpui_kit::white(), 0.12))
                .overflow_hidden()
                .child(fill)
        }));
        h_flex()
            .w_full()
            .gap_4()
            .child(progress)
            .child(
                h_flex()
                    .flex_none()
                    .p(px(2.))
                    .gap_0p5()
                    .rounded_lg()
                    .bg(theme::alpha(gpui_kit::white(), 0.05))
                    .border_1()
                    .border_color(theme::hairline())
                    .child(segment(false, tr("Moi", "Me"), "recap-me", cx))
                    .child(segment(true, tr("Équipe", "Team"), "recap-team", cx)),
            )
            .child(
                Button::new("recap-close")
                    .ghost()
                    .small()
                    .icon(Icon::new(Lucide::X))
                    .tooltip(tr("Fermer (Échap)", "Close (Esc)"))
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            )
            .into_any_element()
    }
}

impl Render for Recap {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.count();
        let body: AnyElement = match &self.state {
            State::Loading => v_flex()
                .items_center()
                .gap_3()
                .text_color(ink2())
                .child(Icon::new(Lucide::LoaderCircle).large().with_animation(
                    "recap-spin",
                    Animation::new(Duration::from_millis(900)).repeat(),
                    |el, t| el.rotate(gpui_kit::percentage(t)),
                ))
                .child(tr("La semaine se compte…", "Counting the week…"))
                .into_any_element(),
            State::Failed(e) => v_flex()
                .items_center()
                .gap_3()
                .text_color(ink2())
                .child(
                    Icon::new(Lucide::TriangleAlert)
                        .large()
                        .text_color(theme::red()),
                )
                .child(e.clone())
                .into_any_element(),
            State::Ready(data) => {
                let data = data.clone();
                let list = slides(data.numbers(self.team), self.team);
                let slide = list[self.slide.min(list.len() - 1)];
                div()
                    .w_full()
                    .relative()
                    .child(self.render_slide(slide, &data, cx))
                    .with_animation(
                        ("recap-slide", self.seq),
                        Animation::new(Duration::from_millis(450))
                            .with_easing(gpui_kit::ease_out_quint()),
                        |el, t| el.opacity(t).top(px(24. * (1. - t))),
                    )
                    .into_any_element()
            }
        };
        let last = self.slide + 1 >= count;
        div()
            .id("recap")
            .test_support()
            .key_context("Recap")
            .track_focus(&self.focus)
            .absolute()
            .inset_0()
            .bg(gpui_kit::rgb(0x0a0a0c))
            .on_action(cx.listener(|this, _: &NextSlide, _, cx| this.step(true, cx)))
            .on_action(cx.listener(|this, _: &PrevSlide, _, cx| this.step(false, cx)))
            .on_action(cx.listener(|this, _: &CloseRecap, _, cx| this.close(cx)))
            .on_action(cx.listener(|this, _: &ToggleTeam, _, cx| {
                let team = !this.team;
                this.set_team(team, cx)
            }))
            // Accent glow from the top.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(theme::alpha(theme::accent(), 0.22), 0.),
                        linear_color_stop(theme::alpha(theme::accent(), 0.), 0.6),
                    ))
                    .with_animation(
                        "recap-glow",
                        Animation::new(Duration::from_millis(4000))
                            .repeat()
                            .with_easing(gpui_kit::pulsating_between(0.6, 1.)),
                        |el, t| el.opacity(t),
                    ),
            )
            .child(
                v_flex()
                    .size_full()
                    .relative()
                    .px_10()
                    .pt(px(crate::workspace::TITLE_HEIGHT + 12.))
                    .pb_8()
                    .child(self.render_top(count, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(div().w_full().max_w(px(780.)).child(body)),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .child(
                                Button::new("recap-prev")
                                    .ghost()
                                    .icon(Icon::new(Lucide::ChevronLeft))
                                    .label(tr("Précédent", "Back"))
                                    .disabled(self.slide == 0)
                                    .on_click(cx.listener(|this, _, _, cx| this.step(false, cx))),
                            )
                            .child(div().text_xs().text_color(muted()).child(tr(
                                "← → pour naviguer · Tab : Moi / Équipe · Échap pour fermer",
                                "← → to navigate · Tab: Me / Team · Esc to close",
                            )))
                            .child(if last {
                                Button::new("recap-done")
                                    .ghost()
                                    .icon(Icon::new(Lucide::Check))
                                    .label(tr("Terminer", "Done"))
                                    .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
                            } else {
                                Button::new("recap-next")
                                    .ghost()
                                    .icon(Icon::new(Lucide::ChevronRight))
                                    .label(tr("Suivant", "Next"))
                                    .on_click(cx.listener(|this, _, _, cx| this.step(true, cx)))
                            }),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weeks_start_on_monday_and_compare_to_the_same_point() {
        // Thursday 16 Oct 2025, 15:00 local.
        let now = Local.with_ymd_and_hms(2025, 10, 16, 15, 0, 0).unwrap();
        let (cur, prev) = spans(now);
        let monday = Local.with_ymd_and_hms(2025, 10, 13, 0, 0, 0).unwrap();
        assert_eq!(cur.start, monday.timestamp());
        assert_eq!(cur.end, now.timestamp() + 1);
        assert_eq!(prev.start, cur.start - 7 * DAY);
        assert_eq!(prev.end, cur.end - 7 * DAY);
        assert_eq!(week_key(now), "2025-W42");
    }

    #[test]
    fn trends_and_friday_reminder() {
        assert_eq!(delta_pct(15, 10), Some(50));
        assert_eq!(delta_pct(5, 10), Some(-50));
        assert_eq!(delta_pct(3, 0), None);
        let thursday = Local.with_ymd_and_hms(2025, 10, 16, 18, 0, 0).unwrap();
        let friday_noon = Local.with_ymd_and_hms(2025, 10, 17, 12, 0, 0).unwrap();
        let friday_late = Local.with_ymd_and_hms(2025, 10, 17, 16, 30, 0).unwrap();
        let sunday = Local.with_ymd_and_hms(2025, 10, 19, 10, 0, 0).unwrap();
        assert!(!reminder_due(thursday, ""));
        assert!(!reminder_due(friday_noon, ""));
        assert!(reminder_due(friday_late, ""));
        assert!(reminder_due(sunday, "2025-W41"));
        assert!(!reminder_due(sunday, "2025-W42"));
    }

    #[test]
    fn slides_skip_what_has_nothing_to_show() {
        let empty = Numbers::default();
        assert_eq!(slides(&empty, false), [Slide::Intro, Slide::Summary]);
        let mut n = Numbers::default();
        n.week.commits = 3;
        assert!(!slides(&n, false).contains(&Slide::People));
        n.prs = 1;
        assert!(slides(&n, false).contains(&Slide::People));
    }
}
