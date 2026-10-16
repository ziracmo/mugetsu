//! ⌘K command palette: every action of the app, the repositories (open tabs,
//! recent ones, those in the settings' folders), workspaces and branches,
//! behind one fuzzy search.
//!
//! The workspace owns it while it is shown: the palette reports a pick
//! ([`PaletteEvent::Run`]) or a dismissal, plays its exit and then asks to be
//! dropped ([`PaletteEvent::Closed`]).

use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Action, Animation, AnimationExt as _, AnyElement, App, AppContext as _, AsKeystroke as _,
    BoxShadow, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable as _, Global,
    HighlightStyle, InteractiveElement as _, IntoElement, KeyBinding, KeyContext,
    ParentElement as _, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledText, Subscription, Window, div, linear_color_stop, linear_gradient, point,
    prelude::FluentBuilder as _, px, relative,
};

use crate::i18n::tr;
use crate::theme;
use crate::workspace::Workspace;

gpui_kit::actions!(
    palette,
    [
        TogglePalette,
        SearchBranches,
        SelectPrev,
        SelectNext,
        Confirm,
        Dismiss
    ]
);

const CONTEXT: &str = "CommandPalette";

pub fn init(cx: &mut App) {
    // Registered after the input's own bindings: within the palette, arrows,
    // enter and escape drive the list rather than the text field.
    let field = Some("CommandPalette > Input");
    let list = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("secondary-k", TogglePalette, Some("Workspace")),
        KeyBinding::new("secondary-k", Dismiss, list),
        KeyBinding::new("secondary-p", SearchBranches, Some("Workspace")),
        KeyBinding::new("up", SelectPrev, field),
        KeyBinding::new("down", SelectNext, field),
        KeyBinding::new("ctrl-p", SelectPrev, field),
        KeyBinding::new("ctrl-n", SelectNext, field),
        KeyBinding::new("enter", Confirm, field),
        KeyBinding::new("escape", Dismiss, field),
        KeyBinding::new("up", SelectPrev, list),
        KeyBinding::new("down", SelectNext, list),
        KeyBinding::new("enter", Confirm, list),
        KeyBinding::new("escape", Dismiss, list),
    ]);
}

// ---- Entries ------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Action,
    Repo,
    Workspace,
    Branch,
}

impl Kind {
    fn title(self) -> &'static str {
        match self {
            Kind::Action => tr("Actions", "Actions"),
            Kind::Repo => tr("Dépôts", "Repositories"),
            Kind::Workspace => "Workspaces",
            Kind::Branch => tr("Branches", "Branches"),
        }
    }
    /// Small tag on a row when results are ranked across kinds.
    fn tag(self) -> &'static str {
        match self {
            Kind::Action => tr("Action", "Action"),
            Kind::Repo => tr("Dépôt", "Repository"),
            Kind::Workspace => "Workspace",
            Kind::Branch => tr("Branche", "Branch"),
        }
    }
}

pub enum EntryIcon {
    Lucide(Lucide),
    /// A repository's own icon (emoji or Lucide name, see `repo_icon`).
    Repo(String),
}

pub type Run = Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

pub struct Entry {
    /// Stable id: remembered in the recents, used by tests.
    pub id: SharedString,
    pub kind: Kind,
    pub title: SharedString,
    pub detail: Option<SharedString>,
    pub icon: EntryIcon,
    /// Shortcut, as a list of keycaps.
    pub keys: Vec<SharedString>,
    /// Extra words searched (synonyms, the other language).
    pub keywords: SharedString,
    pub run: Run,
}

impl Entry {
    pub fn new(
        id: impl Into<SharedString>,
        kind: Kind,
        title: impl Into<SharedString>,
        icon: EntryIcon,
        run: Run,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            detail: None,
            icon,
            keys: Vec::new(),
            keywords: SharedString::default(),
            run,
        }
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn keywords(mut self, words: impl Into<SharedString>) -> Self {
        self.keywords = words.into();
        self
    }

    /// Shows the shortcut bound to `action` in `context` ("Repo", "Workspace", or none).
    pub fn shortcut(mut self, action: &dyn Action, context: Option<&str>, window: &Window) -> Self {
        let context = context
            .and_then(|c| KeyContext::parse(c).ok())
            .unwrap_or_default();
        if let Some(binding) =
            window.highest_precedence_binding_for_action_in_context(action, context)
            && let [stroke] = binding.keystrokes()
        {
            self.keys = keycaps(&Kbd::format(stroke.as_keystroke()));
        }
        self
    }
}

/// `⇧⌘F` → `⇧ ⌘ F`; `Ctrl+Shift+F` → `Ctrl Shift F`.
fn keycaps(formatted: &str) -> Vec<SharedString> {
    if formatted.contains('+') && formatted.len() > 1 {
        return formatted
            .split('+')
            .filter(|s| !s.is_empty())
            .map(|s| SharedString::from(s.to_string()))
            .collect();
    }
    let mut caps: Vec<SharedString> = Vec::new();
    let mut rest = String::new();
    for c in formatted.chars() {
        if "⌃⌥⇧⌘".contains(c) {
            caps.push(c.to_string().into());
        } else {
            rest.push(c);
        }
    }
    if !rest.is_empty() {
        caps.push(rest.into());
    }
    caps
}

// ---- Matching -----------------------------------------------------------------

/// Fuzzy match of `query` (spaces ignored) as a subsequence of `text`,
/// ignoring case. The score favours word starts, runs of consecutive letters
/// and an early first letter; the matched char indices come along for
/// highlighting. `None` when some letter is missing.
pub fn fuzzy(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
    let q: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if q.is_empty() {
        return Some((0, Vec::new()));
    }
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let n = lower.len();
    if q.len() > n {
        return None;
    }
    let word_start = |j: usize| {
        j == 0 || {
            let prev = chars[j - 1];
            !prev.is_alphanumeric() || (prev.is_lowercase() && chars[j].is_uppercase())
        }
    };
    const NONE: i32 = i32::MIN / 2;
    // best[i][j]: best score with q[..=i] matched and q[i] on text[j].
    let mut best = vec![vec![NONE; n]; q.len()];
    let mut from = vec![vec![usize::MAX; n]; q.len()];
    for i in 0..q.len() {
        // Running max of best[i-1][..j] (with its index), for gapped matches.
        let mut run = (NONE, usize::MAX);
        for j in 0..n {
            if i > 0 && j > 0 && best[i - 1][j - 1] > run.0 {
                run = (best[i - 1][j - 1] - (j as i32 - 1), j - 1);
            }
            if lower[j] != q[i] {
                continue;
            }
            let mut gain = 1;
            if word_start(j) {
                gain += 8;
            }
            if i == 0 {
                best[0][j] = gain - (j as i32).min(10) + if j == 0 { 6 } else { 0 };
                continue;
            }
            // Next to the previous letter, or after a gap.
            let adjacent = if j > 0 { best[i - 1][j - 1] } else { NONE };
            let gapped = if run.1 != usize::MAX && run.1 + 1 < j {
                run.0 + (j as i32 - 1) - (j - run.1 - 1).min(6) as i32
            } else {
                NONE
            };
            if adjacent > NONE && adjacent + gain + 6 >= gapped {
                best[i][j] = adjacent + gain + 6;
                from[i][j] = j - 1;
            } else if gapped > NONE {
                best[i][j] = gapped + gain;
                from[i][j] = run.1;
            }
        }
    }
    let last = q.len() - 1;
    let (end, score) = (0..n)
        .map(|j| (j, best[last][j]))
        .filter(|(_, s)| *s > NONE)
        .max_by_key(|(j, s)| (*s, std::cmp::Reverse(*j)))?;
    let mut hits = vec![end];
    let mut j = end;
    for i in (1..q.len()).rev() {
        j = from[i][j];
        hits.push(j);
    }
    hits.reverse();
    // Shorter titles win ties ("Push" before "Push…to remote").
    Some((score - (n as i32) / 8, hits))
}

/// One visible result.
#[derive(Clone, Debug, PartialEq)]
struct Hit {
    entry: usize,
    /// Char indices of the title to highlight.
    matched: Vec<usize>,
}

/// One row of the list: a heading or a result.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    Heading(&'static str),
    Hit(usize),
}

const ROW_H: f32 = 38.;
const HEADING_H: f32 = 26.;
const LIST_MAX_H: f32 = 392.;
const RECENTS: usize = 4;

/// Entry ids picked lately, newest first (kept for the app's lifetime).
#[derive(Default)]
struct Recents(Vec<SharedString>);
impl Global for Recents {}

fn recents(cx: &App) -> Vec<SharedString> {
    cx.try_global::<Recents>()
        .map(|r| r.0.clone())
        .unwrap_or_default()
}

fn remember(id: SharedString, cx: &mut App) {
    let list = &mut cx.default_global::<Recents>().0;
    list.retain(|x| *x != id);
    list.insert(0, id);
    list.truncate(12);
}

// ---- Palette ------------------------------------------------------------------

pub enum PaletteEvent {
    /// An entry was picked: run it (the palette is already leaving).
    Run(Run),
    /// Escape, ⌘K or a click outside: give focus back.
    Dismissed,
    /// The exit animation is over: drop the palette.
    Closed,
}

pub struct Palette {
    input: Entity<InputState>,
    focus: FocusHandle,
    entries: Vec<Entry>,
    hits: Vec<Hit>,
    rows: Vec<Row>,
    /// Top of each row in the list, and the list's full height.
    tops: Vec<f32>,
    height: f32,
    /// Index into `hits`.
    selected: usize,
    /// Where the highlight slides from (row top), and its animation key.
    slide_from: f32,
    slide_seq: usize,
    scroll: ScrollHandle,
    /// Distinct per opening (keys the entrance animations).
    open_seq: usize,
    /// Picked row (row index, key): flash and sparks while leaving.
    fired: Option<(usize, usize)>,
    closing: bool,
    _input: Subscription,
}

impl EventEmitter<PaletteEvent> for Palette {}

static OPENINGS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl Palette {
    /// `branches_only`: ⌘P, the palette narrowed to the branches.
    pub fn new(
        entries: Vec<Entry>,
        branches_only: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let entries: Vec<Entry> = if branches_only {
            entries
                .into_iter()
                .filter(|e| e.kind == Kind::Branch)
                .collect()
        } else {
            entries
        };
        let placeholder = if branches_only {
            tr("Rechercher une branche…", "Search branches…")
        } else {
            tr(
                "Rechercher une action, un dépôt, une branche…",
                "Search actions, repositories, branches…",
            )
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let sub = cx.subscribe(&input, |this, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.refilter(cx);
            }
        });
        let mut this = Self {
            input,
            focus: cx.focus_handle(),
            entries,
            hits: Vec::new(),
            rows: Vec::new(),
            tops: Vec::new(),
            height: 0.,
            selected: 0,
            slide_from: 0.,
            slide_seq: 0,
            scroll: ScrollHandle::new(),
            open_seq: OPENINGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            fired: None,
            closing: false,
            _input: sub,
        };
        this.refilter(cx);
        this
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut App) {
        self.input.focus_handle(cx).focus(window, cx);
    }

    #[cfg(test)]
    pub fn is_closing(&self) -> bool {
        self.closing
    }

    #[cfg(test)]
    pub fn set_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |i, cx| i.set_value(query.to_string(), window, cx));
        self.refilter(cx);
    }

    #[cfg(test)]
    pub fn result_ids(&self) -> Vec<SharedString> {
        self.hits
            .iter()
            .map(|h| self.entries[h.entry].id.clone())
            .collect()
    }

    #[cfg(test)]
    pub fn selected_id(&self) -> Option<SharedString> {
        self.hits
            .get(self.selected)
            .map(|h| self.entries[h.entry].id.clone())
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().trim().to_string();
        let recent = recents(cx);
        self.hits.clear();
        self.rows.clear();
        if query.is_empty() {
            // Grouped: recents first, then each kind in its order.
            let mut shown = HashSet::new();
            let picked: Vec<usize> = recent
                .iter()
                .filter_map(|id| self.entries.iter().position(|e| e.id == *id))
                .take(RECENTS)
                .collect();
            if !picked.is_empty() {
                self.rows.push(Row::Heading(tr("Récents", "Recent")));
                for entry in picked {
                    shown.insert(entry);
                    self.push_hit(entry, Vec::new());
                }
            }
            for kind in [Kind::Action, Kind::Repo, Kind::Workspace, Kind::Branch] {
                let of_kind: Vec<usize> = (0..self.entries.len())
                    .filter(|i| self.entries[*i].kind == kind && !shown.contains(i))
                    .collect();
                if of_kind.is_empty() {
                    continue;
                }
                self.rows.push(Row::Heading(kind.title()));
                for entry in of_kind {
                    self.push_hit(entry, Vec::new());
                }
            }
        } else {
            let mut ranked: Vec<(i32, usize, Vec<usize>)> = self
                .entries
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    let title = fuzzy(&query, &e.title);
                    let other = || {
                        let words =
                            format!("{} {}", e.keywords, e.detail.as_deref().unwrap_or_default());
                        fuzzy(&query, &words).map(|(s, _)| (s - 12, Vec::new()))
                    };
                    let (mut score, matched) = match (title, other()) {
                        (Some(t), Some(o)) if o.0 > t.0 => o,
                        (Some(t), _) => t,
                        (None, o) => o?,
                    };
                    if let Some(pos) = recent.iter().position(|id| *id == e.id) {
                        score += 6 - pos.min(5) as i32;
                    }
                    Some((score, i, matched))
                })
                .collect();
            ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            for (_, entry, matched) in ranked {
                self.push_hit(entry, matched);
            }
        }
        // Layout of the list, for the highlight and scrolling.
        self.tops.clear();
        let mut y = 4.;
        for row in &self.rows {
            self.tops.push(y);
            y += match row {
                Row::Heading(_) => HEADING_H,
                Row::Hit(_) => ROW_H,
            };
        }
        self.height = y + 4.;
        self.selected = 0;
        self.slide_from = self.hit_top(0);
        self.slide_seq += 1;
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn push_hit(&mut self, entry: usize, matched: Vec<usize>) {
        self.rows.push(Row::Hit(self.hits.len()));
        self.hits.push(Hit { entry, matched });
    }

    fn row_of_hit(&self, hit: usize) -> Option<usize> {
        self.rows.iter().position(|r| *r == Row::Hit(hit))
    }

    fn hit_top(&self, hit: usize) -> f32 {
        self.row_of_hit(hit)
            .and_then(|r| self.tops.get(r).copied())
            .unwrap_or(4.)
    }

    fn select(&mut self, hit: usize, cx: &mut Context<Self>) {
        if hit >= self.hits.len() || hit == self.selected {
            return;
        }
        self.slide_from = self.hit_top(self.selected);
        self.selected = hit;
        self.slide_seq += 1;
        // Keep the row in view, with its heading when it is the first one.
        let row = self.row_of_hit(hit).unwrap_or(0);
        let mut top = self.tops[row];
        if row > 0 && matches!(self.rows[row - 1], Row::Heading(_)) {
            top = self.tops[row - 1];
        }
        let bottom = self.tops[row] + ROW_H;
        let view = LIST_MAX_H.min(self.height);
        let offset = -f32::from(self.scroll.offset().y);
        if top < offset {
            self.scroll.set_offset(point(px(0.), px(-top)));
        } else if bottom > offset + view {
            self.scroll
                .set_offset(point(px(0.), px(-(bottom + 4. - view))));
        }
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.hits.len();
        if n == 0 || self.closing {
            return;
        }
        let next = (self.selected as isize + delta).rem_euclid(n as isize) as usize;
        self.select(next, cx);
    }

    fn confirm(&mut self, hit: usize, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        let Some(h) = self.hits.get(hit) else {
            return;
        };
        let entry = &self.entries[h.entry];
        remember(entry.id.clone(), cx);
        let run = entry.run.clone();
        self.fired = self.row_of_hit(hit).map(|r| (r, self.open_seq));
        self.leave(cx);
        cx.emit(PaletteEvent::Run(run));
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        self.leave(cx);
        cx.emit(PaletteEvent::Dismissed);
    }

    /// Plays the exit, then asks to be dropped.
    fn leave(&mut self, cx: &mut Context<Self>) {
        self.closing = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(EXIT_MS))
                .await;
            let _ = this.update(cx, |_, cx| cx.emit(PaletteEvent::Closed));
        })
        .detach();
    }
}

const EXIT_MS: u64 = 190;

/// easeOutBack: a little overshoot before settling.
fn back(t: f32) -> f32 {
    let (c1, t) = (1.70158, t.clamp(0., 1.) - 1.);
    1. + (c1 + 1.) * t.powi(3) + c1 * t.powi(2)
}

fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

impl Palette {
    fn render_icon(&self, entry: &Entry, active: bool) -> AnyElement {
        let tint = if active {
            theme::accent()
        } else {
            theme::muted()
        };
        let inner = match &entry.icon {
            EntryIcon::Lucide(icon) => Icon::new(*icon).small().text_color(tint).into_any_element(),
            EntryIcon::Repo(icon) => crate::repo_icon::view(icon, 15.),
        };
        div()
            .flex_none()
            .size(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .border_1()
            .border_color(if active {
                theme::alpha(theme::accent(), 0.45)
            } else {
                theme::hairline()
            })
            .bg(if active {
                theme::alpha(theme::accent(), 0.14)
            } else {
                theme::alpha(gpui_kit::white(), 0.03)
            })
            .child(inner)
            .into_any_element()
    }

    fn render_title(&self, entry: &Entry, matched: &[usize], active: bool) -> AnyElement {
        let text = entry.title.clone();
        if matched.is_empty() {
            return div().truncate().child(text).into_any_element();
        }
        let hit_style = HighlightStyle {
            color: Some(if active {
                gpui_kit::white()
            } else {
                theme::accent()
            }),
            font_weight: Some(gpui_kit::FontWeight::BOLD),
            underline: active.then(|| gpui_kit::UnderlineStyle {
                thickness: px(1.),
                color: Some(theme::accent()),
                wavy: false,
            }),
            ..Default::default()
        };
        let mut ranges = Vec::new();
        for (i, (offset, c)) in text.char_indices().enumerate() {
            if matched.contains(&i) {
                ranges.push((offset..offset + c.len_utf8(), hit_style));
            }
        }
        div()
            .truncate()
            .child(StyledText::new(text).with_highlights(ranges))
            .into_any_element()
    }

    fn render_row(
        &self,
        row: usize,
        hit_ix: usize,
        ranked: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hit = &self.hits[hit_ix];
        let entry = &self.entries[hit.entry];
        let active = hit_ix == self.selected;
        let fired = self.fired.filter(|(r, _)| *r == row).map(|(_, s)| s);
        let open_seq = self.open_seq;
        // Rows of the first screen arrive one after the other.
        let delay = (row.min(12) as f32) * 22.;
        h_flex()
            .id(ElementId::Name(format!("palette-{}", entry.id).into()))
            .test_support()
            .relative()
            .h(px(ROW_H))
            .px_2()
            .gap_2p5()
            .items_center()
            .cursor_pointer()
            .text_sm()
            .text_color(if active {
                gpui_kit::white()
            } else {
                cx.theme().foreground
            })
            .child(self.render_icon(entry, active))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .items_baseline()
                    .child(self.render_title(entry, &hit.matched, active))
                    .when_some(entry.detail.clone(), |s, d| {
                        s.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(theme::muted())
                                .child(d),
                        )
                    }),
            )
            .when(ranked, |s| {
                s.child(
                    div()
                        .flex_none()
                        .px_1p5()
                        .rounded_sm()
                        .text_xs()
                        .text_color(theme::muted())
                        .bg(theme::alpha(gpui_kit::white(), 0.04))
                        .child(entry.kind.tag()),
                )
            })
            .when(!entry.keys.is_empty(), |s| {
                s.child(
                    h_flex()
                        .flex_none()
                        .gap_0p5()
                        .children(entry.keys.iter().map(|k| keycap(k.clone(), active))),
                )
            })
            .when(active && entry.keys.is_empty(), |s| {
                s.child(
                    Icon::new(Lucide::CornerDownLeft)
                        .xsmall()
                        .flex_none()
                        .text_color(theme::muted()),
                )
            })
            .when_some(fired, |s, seq| {
                s.child(
                    div()
                        .absolute()
                        .top(px(ROW_H / 2.))
                        .left(px(21.))
                        .size_0()
                        .child(crate::repo_view::links::flourish(seq)),
                )
            })
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if !this.closing && this.selected != hit_ix {
                    this.select(hit_ix, cx);
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.confirm(hit_ix, cx)))
            .with_animation(
                ElementId::NamedInteger(entry.id.clone(), open_seq as u64),
                Animation::new(Duration::from_millis(220 + delay as u64)),
                move |el, t| {
                    let total = 220. + delay;
                    let k = ease_out(((t * total) - delay) / 220.);
                    el.opacity(k).ml(px(10. * (1. - k)))
                },
            )
            .into_any_element()
    }

    /// The accent pill behind the selected row, sliding from the previous one.
    fn render_highlight(&self) -> Option<AnyElement> {
        if self.hits.is_empty() {
            return None;
        }
        let (from, to) = (self.slide_from, self.hit_top(self.selected));
        let fired = self.fired.is_some();
        Some(
            div()
                .absolute()
                .left_1()
                .right_1()
                .h(px(ROW_H))
                .rounded_lg()
                .bg(theme::alpha(
                    theme::accent(),
                    if fired { 0.3 } else { 0.13 },
                ))
                .border_1()
                .border_color(theme::alpha(theme::accent(), 0.35))
                .shadow(vec![BoxShadow {
                    color: theme::alpha(theme::accent(), 0.25),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(14.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                // Accent bar on the left edge.
                .child(
                    div()
                        .absolute()
                        .left(px(-1.))
                        .top(px(9.))
                        .bottom(px(9.))
                        .w(px(3.))
                        .rounded_full()
                        .bg(theme::accent()),
                )
                .with_animation(
                    ("palette-slide", self.slide_seq),
                    Animation::new(Duration::from_millis(170)),
                    move |el, t| el.top(px(from + (to - from) * ease_out(t))),
                )
                .into_any_element(),
        )
    }
}

fn keycap(text: SharedString, active: bool) -> impl IntoElement {
    div()
        .min_w(px(20.))
        .h(px(20.))
        .px_1()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .border_1()
        .border_color(if active {
            theme::alpha(theme::accent(), 0.5)
        } else {
            theme::hairline()
        })
        .bg(theme::alpha(
            gpui_kit::white(),
            if active { 0.08 } else { 0.04 },
        ))
        .text_xs()
        .text_color(if active {
            gpui_kit::white()
        } else {
            theme::muted()
        })
        .child(text)
}

fn hint(keys: &'static str, label: &'static str) -> impl IntoElement {
    h_flex()
        .gap_1()
        .items_center()
        .child(keycap(keys.into(), false))
        .child(label)
}

impl Render for Palette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ranked = !self.input.read(cx).value().trim().is_empty();
        let rows: Vec<AnyElement> = self
            .rows
            .clone()
            .into_iter()
            .enumerate()
            .map(|(r, row)| match row {
                Row::Heading(title) => div()
                    .h(px(HEADING_H))
                    .px_2()
                    .pt(px(9.))
                    .text_xs()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(theme::muted())
                    .child(title.to_uppercase())
                    .into_any_element(),
                Row::Hit(h) => self.render_row(r, h, ranked, cx),
            })
            .collect();
        let empty = self.hits.is_empty();
        let count = self.hits.len();
        let closing = self.closing;
        let open_seq = self.open_seq;
        let list_h = self.height.min(LIST_MAX_H);

        let panel = v_flex()
            .id("command-palette")
            .test_support()
            .relative()
            .w(px(640.))
            .overflow_hidden()
            .rounded_2xl()
            .bg(theme::popover())
            .border_1()
            .border_color(theme::glass_edge())
            .shadow(vec![
                BoxShadow {
                    color: theme::alpha(gpui_kit::black(), 0.55),
                    offset: point(px(0.), px(24.)),
                    blur_radius: px(60.),
                    spread_radius: px(-8.),
                    inset: false,
                },
                BoxShadow {
                    color: theme::alpha(theme::accent(), 0.18),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(40.),
                    spread_radius: px(0.),
                    inset: false,
                },
            ])
            // Accent rim on top, crossed by a light sweep on opening.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(1.))
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(theme::alpha(theme::accent(), 0.), 0.),
                        linear_color_stop(theme::alpha(theme::accent(), 0.9), 0.5),
                    ))
                    .with_animation(
                        ("palette-rim", open_seq),
                        Animation::new(Duration::from_millis(900)),
                        |el, t| {
                            let k = ease_out(t);
                            el.left(relative(-1. + k)).right(relative(1. - k))
                        },
                    ),
            )
            .child(
                h_flex()
                    .px_3()
                    .py_2p5()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme::hairline())
                    .child(
                        Input::new(&self.input)
                            .id("palette-input")
                            .appearance(false)
                            .prefix(
                                Icon::new(Lucide::Command)
                                    .small()
                                    .text_color(theme::accent()),
                            ),
                    )
                    .child(keycap("esc".into(), false)),
            )
            .child(
                div()
                    .id("palette-list")
                    .relative()
                    .h(px(if empty { 120. } else { list_h }))
                    .px_1p5()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .when(empty, |s| {
                        s.child(
                            v_flex()
                                .size_full()
                                .items_center()
                                .justify_center()
                                .gap_2()
                                .text_sm()
                                .text_color(theme::muted())
                                .child(
                                    Icon::new(Lucide::Moon)
                                        .size(px(22.))
                                        .text_color(theme::alpha(theme::accent(), 0.7)),
                                )
                                .child(tr("Rien ne correspond", "Nothing matches"))
                                .with_animation(
                                    "palette-empty",
                                    Animation::new(Duration::from_millis(260)),
                                    |el, t| el.opacity(ease_out(t)),
                                ),
                        )
                    })
                    .when(!empty, |s| {
                        s.child(
                            div()
                                .relative()
                                .h(px(self.height))
                                .children(self.render_highlight())
                                .child(
                                    div()
                                        .absolute()
                                        .top(px(4.))
                                        .left_0()
                                        .right_0()
                                        .children(rows),
                                ),
                        )
                    }),
            )
            .child(
                h_flex()
                    .px_3()
                    .py_1p5()
                    .gap_3()
                    .border_t_1()
                    .border_color(theme::hairline())
                    .bg(theme::alpha(gpui_kit::white(), 0.02))
                    .text_xs()
                    .text_color(theme::muted())
                    .child(hint("↑↓", tr("Naviguer", "Navigate")))
                    .child(hint("↵", tr("Exécuter", "Run")))
                    .child(div().flex_1())
                    .child(trf!("{count} résultat(s)", "{count} result(s)"))
                    .child(
                        div()
                            .text_color(theme::alpha(theme::accent(), 0.8))
                            .child("月"),
                    ),
            );

        let panel = if closing {
            panel
                .with_animation(
                    ("palette-out", open_seq),
                    Animation::new(Duration::from_millis(EXIT_MS)),
                    |el, t| {
                        let k = ease_out(t);
                        el.opacity(1. - k).mt(px(-10. * k))
                    },
                )
                .into_any_element()
        } else {
            panel
                .with_animation(
                    ("palette-in", open_seq),
                    Animation::new(Duration::from_millis(340)),
                    |el, t| el.opacity(ease_out(t * 1.6)).mt(px(22. * (1. - back(t)))),
                )
                .into_any_element()
        };

        div()
            .id("palette-overlay")
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .pt(relative(0.14))
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.confirm(this.selected, cx)))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.dismiss(cx)))
            // Dimmed backdrop; a click on it closes.
            .child(
                div()
                    .id("palette-backdrop")
                    .absolute()
                    .inset_0()
                    .bg(gpui_kit::black())
                    .when(!closing, |s| {
                        s.occlude().on_mouse_down(
                            gpui_kit::MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.dismiss(cx)),
                        )
                    })
                    .with_animation(
                        ("palette-dim", open_seq * 2 + closing as usize),
                        Animation::new(Duration::from_millis(if closing { EXIT_MS } else { 220 })),
                        move |el, t| el.opacity(0.32 * if closing { 1. - t } else { t }),
                    ),
            )
            .child(div().occlude().child(panel))
    }
}

/// Builds every entry the palette offers for `ws`.
pub fn entries(ws: &Workspace, window: &Window, cx: &App) -> Vec<Entry> {
    use crate::app_menu as app;
    use crate::repo_view as repo;
    use crate::workspace as wsa;

    let a = |id: &'static str,
             title: &'static str,
             icon: Lucide,
             action: Box<dyn Action>,
             context: Option<&str>,
             keywords: &'static str| {
        let run: Run = {
            let action: Rc<dyn Action> = action.boxed_clone().into();
            Rc::new(move |ws: &mut Workspace, window, cx| {
                ws.dispatch(action.boxed_clone(), window, cx)
            })
        };
        Entry::new(id, Kind::Action, title, EntryIcon::Lucide(icon), run)
            .shortcut(action.as_ref(), context, window)
            .keywords(keywords)
    };
    let has_repo = ws.active().is_some();
    let github = ws
        .active()
        .is_some_and(|v| v.read(cx).repo_github().is_some());
    let mut out = Vec::new();
    if has_repo {
        let r = Some("Repo");
        out.extend([
            a(
                "fetch",
                "Fetch",
                Lucide::CloudDownload,
                Box::new(repo::Fetch),
                r,
                "récupérer remote",
            ),
            a(
                "pull",
                "Pull",
                Lucide::ArrowDownToLine,
                Box::new(repo::Pull),
                r,
                "tirer mettre à jour update",
            ),
            a(
                "push",
                "Push",
                Lucide::ArrowUpFromLine,
                Box::new(repo::Push),
                r,
                "pousser envoyer upload",
            ),
            a(
                "new-branch",
                tr("Nouvelle branche…", "New Branch…"),
                Lucide::GitBranchPlus,
                Box::new(repo::NewBranch),
                r,
                "create branch créer",
            ),
            a(
                "checkout",
                tr("Checkout d'une branche…", "Checkout Branch…"),
                Lucide::GitBranch,
                Box::new(repo::CheckoutBranch),
                r,
                "switch changer basculer",
            ),
            a(
                "new-tag",
                tr("Nouveau tag…", "New Tag…"),
                Lucide::Tag,
                Box::new(repo::NewTag),
                r,
                "create tag release étiquette",
            ),
            a(
                "stash",
                "Stash",
                Lucide::Archive,
                Box::new(repo::Stash),
                r,
                "remiser mettre de côté",
            ),
            a(
                "refresh",
                tr("Rafraîchir", "Refresh"),
                Lucide::RefreshCw,
                Box::new(repo::Refresh),
                r,
                "reload recharger",
            ),
            a(
                "show-changes",
                tr("Afficher les changements", "Show Changes"),
                Lucide::FilePen,
                Box::new(repo::ShowChanges),
                r,
                "working copy commit stage modifications",
            ),
            a(
                "show-history",
                tr("Afficher l'historique", "Show History"),
                Lucide::GitCommitHorizontal,
                Box::new(repo::ShowHistory),
                r,
                "log graph commits historique",
            ),
            a(
                "show-stats",
                tr("Afficher les stats", "Show Stats"),
                Lucide::ChartColumn,
                Box::new(repo::ShowStats),
                r,
                "statistics authors auteurs",
            ),
            a(
                "reveal-head",
                tr("Aller à HEAD", "Reveal HEAD"),
                Lucide::Crosshair,
                Box::new(repo::RevealHead),
                r,
                "head current courant",
            ),
            a(
                "search-commits",
                tr("Rechercher des commits", "Search Commits"),
                Lucide::TextSearch,
                Box::new(repo::SearchCommits),
                r,
                "find chercher sha author auteur",
            ),
            a(
                "toggle-compact",
                tr("Vue compacte", "Compact View"),
                Lucide::Rows3,
                Box::new(repo::ToggleCompact),
                r,
                "compact branches résumé",
            ),
            a(
                "toggle-details-side",
                tr(
                    "Détails du commit à droite / en bas",
                    "Commit Details Right / Bottom",
                ),
                Lucide::PanelRight,
                Box::new(repo::ToggleDetailsSide),
                r,
                "details détails panneau côté bas layout disposition",
            ),
            a(
                "toggle-sidebar",
                tr("Barre latérale", "Toggle Sidebar"),
                Lucide::PanelLeft,
                Box::new(repo::ToggleSidebar),
                r,
                "sidebar panneau",
            ),
        ]);
        if github {
            out.push(a(
                "copy-github-link",
                tr("Copier le lien GitHub", "Copy GitHub Link"),
                Lucide::Link,
                Box::new(repo::CopyGithubLink),
                r,
                "url permalink commit lignes lines share partager",
            ));
        }
    }
    let w = Some("Workspace");
    out.extend([
        a(
            "open-repo",
            tr("Ouvrir un dépôt…", "Open Repository…"),
            Lucide::FolderOpen,
            Box::new(wsa::OpenRepository),
            w,
            "open folder dossier add ajouter",
        ),
        a(
            "new-window",
            tr("Nouvelle fenêtre", "New Window"),
            Lucide::AppWindow,
            Box::new(app::NewWindow),
            None,
            "window fenêtre",
        ),
        a(
            "next-tab",
            tr("Onglet suivant", "Next Tab"),
            Lucide::ArrowRight,
            Box::new(wsa::NextTab),
            w,
            "tab onglet",
        ),
    ]);
    if has_repo {
        out.extend([
            a(
                "overview",
                tr("Vue d'ensemble", "Overview"),
                Lucide::LayoutGrid,
                Box::new(wsa::ToggleOverview),
                w,
                "overview dashboard tableau de bord repos dépôts accueil home",
            ),
            a(
                "recap",
                tr("Recap de la semaine", "Weekly Recap"),
                Lucide::Sparkles,
                Box::new(wsa::OpenRecap),
                w,
                "recap semaine week weekly bilan productivité productivity wrapped",
            ),
        ]);
        out.push(a(
            "close-tab",
            tr("Fermer l'onglet", "Close Tab"),
            Lucide::X,
            Box::new(wsa::CloseTab),
            w,
            "close fermer tab",
        ));
    }
    out.extend([
        a(
            "settings",
            tr("Réglages…", "Settings…"),
            Lucide::Settings,
            Box::new(app::OpenSettings),
            None,
            "preferences préférences settings options",
        ),
        a(
            "about",
            tr("À propos de Mugetsu", "About Mugetsu"),
            Lucide::Info,
            Box::new(app::About),
            None,
            "version about",
        ),
        a(
            "quit",
            tr("Quitter Mugetsu", "Quit Mugetsu"),
            Lucide::Power,
            Box::new(app::Quit),
            None,
            "quit exit quitter",
        ),
    ]);

    // Repositories: open tabs, then recent ones, then those found in folders.
    let icons = crate::settings::get().repo_icons;
    let mut seen = HashSet::new();
    for (i, root) in ws.tab_roots(cx).into_iter().enumerate() {
        let name = repo_name(&root);
        let icon = icons
            .get(&root)
            .cloned()
            .map_or(EntryIcon::Lucide(Lucide::FolderGit2), EntryIcon::Repo);
        out.push(
            Entry::new(
                format!("repo:{}", root.display()),
                Kind::Repo,
                name,
                icon,
                Rc::new(move |ws: &mut Workspace, _, cx| ws.activate(i, cx)),
            )
            .detail(tr("Onglet ouvert", "Open tab"))
            .keywords(root.display().to_string()),
        );
        seen.insert(root);
    }
    let settings = crate::settings::get();
    let found = crate::open_repo::find_repos(&settings.scan_folders());
    for root in settings.recent.iter().chain(found.iter()) {
        if !seen.insert(root.clone()) {
            continue;
        }
        let path = root.clone();
        let icon = icons
            .get(root)
            .cloned()
            .map_or(EntryIcon::Lucide(Lucide::FolderGit2), EntryIcon::Repo);
        out.push(
            Entry::new(
                format!("repo:{}", root.display()),
                Kind::Repo,
                repo_name(root),
                icon,
                Rc::new(move |ws: &mut Workspace, window, cx| {
                    ws.open_or_report(path.clone(), window, cx)
                }),
            )
            .detail(crate::app_menu::display_path(root)),
        );
    }

    // Workspaces.
    let (names, current) = ws.workspace_list();
    if names.len() > 1 {
        for (i, name) in names.into_iter().enumerate() {
            if i == current {
                continue;
            }
            out.push(
                Entry::new(
                    format!("workspace:{name}"),
                    Kind::Workspace,
                    name,
                    EntryIcon::Lucide(Lucide::LayoutDashboard),
                    Rc::new(move |ws: &mut Workspace, window, cx| {
                        ws.switch_workspace(i, window, cx)
                    }),
                )
                .detail(tr("Changer de workspace", "Switch workspace")),
            );
        }
    }

    // Local branches of the active repository.
    if let Some(view) = ws.active() {
        let v = view.read(cx);
        for (name, is_head) in v.local_branches() {
            if is_head {
                continue;
            }
            let view = view.downgrade();
            let target = name.clone();
            out.push(
                Entry::new(
                    format!("branch:{name}"),
                    Kind::Branch,
                    name,
                    EntryIcon::Lucide(Lucide::GitBranch),
                    Rc::new(move |_: &mut Workspace, window, cx| {
                        let target = target.clone();
                        let _ =
                            view.update(cx, |v, cx| v.request_checkout(target, false, window, cx));
                    }),
                )
                .detail("Checkout"),
            );
        }
    }
    out
}

fn repo_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits(q: &str, t: &str) -> Option<Vec<usize>> {
        fuzzy(q, t).map(|(_, h)| h)
    }

    #[test]
    fn fuzzy_matches_subsequences_and_prefers_word_starts() {
        assert_eq!(hits("nb", "New Branch…"), Some(vec![0, 4]));
        assert_eq!(hits("NEW BR", "New Branch…"), Some(vec![0, 1, 2, 4, 5]));
        assert_eq!(hits("xyz", "Push"), None);
        assert_eq!(hits("", "Push"), Some(vec![]));
        // Word starts beat letters in the middle of a word.
        assert_eq!(hits("sc", "Search Commits"), Some(vec![0, 7]));
        let score = |q, t| fuzzy(q, t).unwrap().0;
        assert!(score("pu", "Push") > score("pu", "Compute"));
        assert!(score("push", "Push") > score("push", "Push to every remote"));
        assert!(score("fet", "Fetch") > score("fet", "Fix the tests"));
    }

    #[test]
    fn shortcuts_split_into_keycaps() {
        assert_eq!(keycaps("⇧⌘F"), ["⇧", "⌘", "F"]);
        assert_eq!(keycaps("Ctrl+Shift+F"), ["Ctrl", "Shift", "F"]);
        assert_eq!(keycaps("⌘0"), ["⌘", "0"]);
    }

    #[test]
    fn easing_ends_where_it_should() {
        assert!(back(0.).abs() < 1e-6 && (back(1.) - 1.).abs() < 1e-6);
        assert_eq!(ease_out(1.), 1.);
        assert_eq!(ease_out(-1.), 0.);
    }
}
