//! Back / forward through what a tab showed (mouse side buttons, ⌘[ / ⌘]).
//!
//! The tab's place is sampled at every render: a change the user made becomes
//! a step, while reloads only update the current place (see `install`).

use std::time::{Duration, Instant};

use gix::ObjectId;
use gpui_kit::{Context, SharedString};

use super::{Mode, RepoView, SelectedChange};

/// What a tab shows, as far as going back is concerned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub mode: Mode,
    pub commit: Option<ObjectId>,
    pub commit_file: Option<SharedString>,
    pub change: Option<SelectedChange>,
    pub stack: Option<String>,
    pub pr: Option<u64>,
}

impl Place {
    fn bare(mode: Mode) -> Self {
        Self {
            mode,
            commit: None,
            commit_file: None,
            change: None,
            stack: None,
            pr: None,
        }
    }

    /// `self` only adds a detail `before` lacked (HEAD picked on first load,
    /// the first change selected when the view opened): no new step.
    fn refines(&self, before: &Place) -> bool {
        self.mode == before.mode
            && match self.mode {
                Mode::History => {
                    before.commit.is_none()
                        || (before.commit == self.commit && before.commit_file.is_none())
                }
                Mode::Changes => before.change.is_none(),
                _ => false,
            }
    }
}

/// Steps held each way.
const DEPTH: usize = 100;
/// History selections closer than this (↑↓ held down) make one step.
const BURST: Duration = Duration::from_millis(600);

#[derive(Default)]
pub struct NavHistory {
    back: Vec<Place>,
    forward: Vec<Place>,
    current: Option<Place>,
    last_step: Option<Instant>,
}

impl NavHistory {
    /// The tab now shows `place`: a new step unless nothing really moved.
    pub fn record(&mut self, place: Place, now: Instant) {
        let Some(current) = self.current.take() else {
            self.current = Some(place);
            return;
        };
        if current == place {
            self.current = Some(current);
            return;
        }
        let burst = place.mode == Mode::History
            && current.mode == Mode::History
            && self
                .last_step
                .is_some_and(|t| now.duration_since(t) < BURST);
        let refines = place.refines(&current);
        if !refines && !burst {
            self.back.push(current);
            if self.back.len() > DEPTH {
                self.back.remove(0);
            }
        }
        self.forward.clear();
        self.current = Some(place);
        if !refines {
            self.last_step = Some(now);
        }
    }

    /// Takes `place` as current without making a step (reloads).
    pub fn settle(&mut self, place: Place) {
        self.current = Some(place);
    }

    pub fn can_go(&self, back: bool) -> bool {
        !if back { &self.back } else { &self.forward }.is_empty()
    }

    /// The place to show one step back (or forward), now current.
    pub fn go(&mut self, back: bool) -> Option<Place> {
        let (from, to) = if back {
            (&mut self.back, &mut self.forward)
        } else {
            (&mut self.forward, &mut self.back)
        };
        let place = from.pop()?;
        if let Some(current) = self.current.take() {
            to.push(current);
        }
        self.current = Some(place.clone());
        self.last_step = None;
        Some(place)
    }
}

impl RepoView {
    pub(super) fn place(&self) -> Place {
        let mut place = Place::bare(self.mode);
        match self.mode {
            Mode::History => {
                place.commit = self.selected;
                place.commit_file = self.commit_file.clone();
            }
            Mode::Changes => place.change = self.selected_change.clone(),
            Mode::Stack => place.stack = self.stack_focus.clone(),
            Mode::Pr => place.pr = self.pr_page.as_ref().map(|p| p.number),
            Mode::Stats => {}
        }
        place
    }

    /// Notes where the tab is (every render).
    pub(super) fn track_place(&mut self) {
        if self.loading && self.data.history.is_empty() {
            return;
        }
        let place = self.place();
        self.nav.record(place, Instant::now());
    }

    pub fn can_navigate(&self, back: bool) -> bool {
        self.nav.can_go(back)
    }

    /// One step back (or forward) in what this tab showed.
    pub fn navigate(&mut self, back: bool, cx: &mut Context<Self>) {
        let Some(place) = self.nav.go(back) else {
            return;
        };
        self.show_place(place, cx);
        // Whatever could not be restored (a commit gone since) is where we are.
        let now = self.place();
        self.nav.settle(now);
        cx.notify();
    }

    fn show_place(&mut self, place: Place, cx: &mut Context<Self>) {
        match place.mode {
            Mode::History => {
                self.set_mode(Mode::History, cx);
                if let Some(id) = place.commit {
                    self.select_commit(id, true, cx);
                    if place.commit_file.is_some() && self.selected == Some(id) {
                        self.commit_file = place.commit_file;
                        if let Some(path) = self.commit_file.clone() {
                            self.commit_diff
                                .update(cx, |d, cx| d.scroll_to_file(&path, cx));
                        }
                    }
                }
            }
            Mode::Changes => {
                let present = place.change.filter(|sel| {
                    self.data.status.files.iter().any(|f| {
                        f.path == sel.path
                            && if sel.staged {
                                f.is_staged()
                            } else {
                                f.is_unstaged()
                            }
                    })
                });
                match present {
                    Some(sel) => {
                        self.mode = Mode::Changes;
                        if self.selected_change.as_ref() != Some(&sel) {
                            self.load_change(sel, cx);
                        }
                    }
                    None => self.set_mode(Mode::Changes, cx),
                }
            }
            Mode::Stats => self.set_mode(Mode::Stats, cx),
            Mode::Stack => match place.stack {
                Some(head) => self.open_stack(head, cx),
                None => self.set_mode(Mode::Stack, cx),
            },
            Mode::Pr => match place.pr {
                Some(n) => self.open_pr(n, cx),
                None => self.set_mode(Mode::Pr, cx),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(mode: Mode, pr: Option<u64>) -> Place {
        Place {
            pr,
            ..Place::bare(mode)
        }
    }

    #[test]
    fn steps_back_and_forward() {
        let mut nav = NavHistory::default();
        let t = Instant::now();
        nav.record(at(Mode::History, None), t);
        nav.record(at(Mode::Stats, None), t);
        nav.record(at(Mode::Pr, Some(3)), t);
        assert_eq!(nav.go(true), Some(at(Mode::Stats, None)));
        assert_eq!(nav.go(true), Some(at(Mode::History, None)));
        assert_eq!(nav.go(true), None);
        assert_eq!(nav.go(false), Some(at(Mode::Stats, None)));
        // A new step drops what was ahead.
        nav.record(at(Mode::Changes, None), t);
        assert!(!nav.can_go(false));
        assert_eq!(nav.go(true), Some(at(Mode::Stats, None)));
    }

    #[test]
    fn refinements_and_bursts_make_no_step() {
        let mut nav = NavHistory::default();
        let t = Instant::now();
        let commit = |b: u8| Place {
            commit: Some(ObjectId::from_bytes_or_panic(&[b; 20])),
            ..Place::bare(Mode::History)
        };
        nav.record(Place::bare(Mode::History), t);
        nav.record(commit(1), t);
        assert!(!nav.can_go(true), "HEAD picked on load is not a step");
        nav.record(commit(2), t + Duration::from_millis(100));
        nav.record(commit(3), t + Duration::from_millis(300));
        nav.record(commit(4), t + Duration::from_millis(500));
        assert_eq!(nav.go(true), Some(commit(1)));
        assert!(!nav.can_go(true));
    }
}
