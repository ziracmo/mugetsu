//! System notifications for pull requests: CI finished, review requested, PR
//! merged. Each repository tab diffs its successive PR listings; the first
//! listing (or one after a long gap) is only a baseline.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use mugetsu_forge::{CheckState, PrState, PullRequest};

use crate::i18n::tr;
use crate::settings::Notifications;

/// A listing older than this is a new baseline: what changed meanwhile is old news.
const MAX_GAP: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub title: String,
    pub body: String,
    pub url: String,
}

#[derive(Clone, Copy)]
struct Seen {
    state: PrState,
    checks: Option<CheckState>,
    asked_me: bool,
}

#[derive(Default)]
pub struct Watcher {
    seen: HashMap<u64, Seen>,
    at: Option<Instant>,
}

impl Watcher {
    /// Records `prs` and returns what deserves a notification since the last call.
    pub fn diff(
        &mut self,
        repo: &str,
        prs: &[PullRequest],
        me: Option<&str>,
        prefs: Notifications,
        now: Instant,
    ) -> Vec<Note> {
        let fresh = self
            .at
            .is_some_and(|at| now.saturating_duration_since(at) <= MAX_GAP);
        let asked_me =
            |pr: &PullRequest| me.is_some_and(|m| pr.requested_reviewers.iter().any(|r| r == m));
        let mut notes = Vec::new();
        for pr in prs {
            let checks = pr.status.as_ref().and_then(|s| s.checks);
            let before = self.seen.get(&pr.number).copied();
            let mine = prefs.others || me.is_some_and(|m| pr.author == m);
            let label = format!("#{} {}", pr.number, pr.title);
            let mut push = |title: String, body: String| {
                notes.push(Note {
                    title,
                    body,
                    url: pr.url.clone(),
                })
            };
            if fresh {
                let was_open =
                    before.is_some_and(|b| matches!(b.state, PrState::Open | PrState::Draft));
                if prefs.merged && mine && pr.state == PrState::Merged && was_open {
                    push(
                        format!("{repo} · {}", tr("PR mergée", "PR merged")),
                        label.clone(),
                    );
                }
                let was_pending = before.is_some_and(|b| b.checks == Some(CheckState::Pending));
                if prefs.ci && mine && was_pending && pr.state != PrState::Merged {
                    match checks {
                        Some(CheckState::Passing) => push(
                            format!("{repo} · {}", tr("CI réussie", "CI passed")),
                            label.clone(),
                        ),
                        Some(CheckState::Failing) => {
                            let failing = pr
                                .status
                                .as_ref()
                                .map(|s| s.failing.join(", "))
                                .unwrap_or_default();
                            let body = if failing.is_empty() {
                                label.clone()
                            } else {
                                format!("{label}\n{}{failing}", tr("Échec : ", "Failed: "))
                            };
                            push(format!("{repo} · {}", tr("CI en échec", "CI failed")), body)
                        }
                        _ => {}
                    }
                }
                let open = matches!(pr.state, PrState::Open | PrState::Draft);
                if prefs.review && open && asked_me(pr) && !before.is_some_and(|b| b.asked_me) {
                    push(
                        format!("{repo} · {}", tr("Review demandée", "Review requested")),
                        trf!("{} · par {}", "{} · by {}", label, pr.author),
                    );
                }
            }
            self.seen.insert(
                pr.number,
                Seen {
                    state: pr.state,
                    // GraphQL statuses can be missing on one listing: keep the last known.
                    checks: checks
                        .or_else(|| pr.status.is_none().then_some(before?.checks).flatten()),
                    asked_me: asked_me(pr),
                },
            );
        }
        self.at = Some(now);
        notes
    }
}

/// Shows `note` as a system notification; clicking it opens the PR.
pub fn show(note: &Note) {
    if cfg!(test) {
        return;
    }
    #[cfg(target_os = "macos")]
    mac::show(note);
    #[cfg(not(target_os = "macos"))]
    let _ = note;
}

#[cfg(target_os = "macos")]
// objc 0.2's macros test cfgs this crate doesn't declare.
#[allow(unexpected_cfgs)]
mod mac {
    use std::ffi::CString;
    use std::sync::OnceLock;

    use objc::declare::ClassDecl;
    use objc::runtime::{BOOL, Class, Object, Sel, YES};
    use objc::{class, msg_send, sel, sel_impl};

    use super::Note;

    /// `NSString` from Rust text (interior NULs dropped).
    unsafe fn ns_string(s: &str) -> *mut Object {
        let c = CString::new(s.replace('\0', "")).unwrap_or_default();
        unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
    }

    extern "C" fn should_present(_: &Object, _: Sel, _: *mut Object, _: *mut Object) -> BOOL {
        YES
    }

    extern "C" fn did_activate(_: &Object, _: Sel, _: *mut Object, note: *mut Object) {
        // SAFETY: AppKit hands a valid NSUserNotification; nil results are checked.
        unsafe {
            let info: *mut Object = msg_send![note, userInfo];
            if info.is_null() {
                return;
            }
            let url: *mut Object = msg_send![info, objectForKey: ns_string("url")];
            if url.is_null() {
                return;
            }
            let url: *mut Object = msg_send![class!(NSURL), URLWithString: url];
            if url.is_null() {
                return;
            }
            let workspace: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
            let _: BOOL = msg_send![workspace, openURL: url];
        }
    }

    /// Delegate showing banners while the app is frontmost and opening the PR on click.
    fn delegate() -> usize {
        static DELEGATE: OnceLock<usize> = OnceLock::new();
        *DELEGATE.get_or_init(|| {
            let Some(mut decl) = ClassDecl::new("MugetsuNotificationDelegate", class!(NSObject))
            else {
                return 0;
            };
            // SAFETY: the signatures match the NSUserNotificationCenterDelegate methods.
            unsafe {
                decl.add_method(
                    sel!(userNotificationCenter:shouldPresentNotification:),
                    should_present as extern "C" fn(&Object, Sel, *mut Object, *mut Object) -> BOOL,
                );
                decl.add_method(
                    sel!(userNotificationCenter:didActivateNotification:),
                    did_activate as extern "C" fn(&Object, Sel, *mut Object, *mut Object),
                );
            }
            let cls = decl.register();
            // SAFETY: plain alloc/init; the instance is kept for the app's lifetime.
            let obj: *mut Object = unsafe { msg_send![cls, new] };
            obj as usize
        })
    }

    pub fn show(note: &Note) {
        // NSUserNotificationCenter needs a bundle: `cargo run` falls back to AppleScript.
        let Some(center_class) = Class::get("NSUserNotificationCenter") else {
            return osascript(note);
        };
        // SAFETY: AppKit calls on the main thread with valid objects; nil checked.
        unsafe {
            let center: *mut Object = msg_send![center_class, defaultUserNotificationCenter];
            if center.is_null() {
                return osascript(note);
            }
            let delegate = delegate() as *mut Object;
            if !delegate.is_null() {
                let _: () = msg_send![center, setDelegate: delegate];
            }
            let n: *mut Object = msg_send![class!(NSUserNotification), new];
            let _: () = msg_send![n, setTitle: ns_string(&note.title)];
            let _: () = msg_send![n, setInformativeText: ns_string(&note.body)];
            let info: *mut Object = msg_send![class!(NSDictionary),
                dictionaryWithObject: ns_string(&note.url)
                forKey: ns_string("url")];
            let _: () = msg_send![n, setUserInfo: info];
            let _: () = msg_send![center, deliverNotification: n];
            let _: () = msg_send![n, release];
        }
    }

    fn osascript(note: &Note) {
        let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            quote(&note.body),
            quote(&note.title)
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use mugetsu_forge::PrStatus;

    use super::*;

    fn pr(number: u64, author: &str, state: PrState, checks: Option<CheckState>) -> PullRequest {
        PullRequest {
            number,
            title: format!("PR {number}"),
            url: format!("https://github.com/o/r/pull/{number}"),
            state,
            status: checks.map(|c| PrStatus {
                checks: Some(c),
                failing: if c == CheckState::Failing {
                    vec!["lint".into()]
                } else {
                    vec![]
                },
                ..PrStatus::default()
            }),
            head: format!("b{number}"),
            base: "main".into(),
            same_repo: true,
            author: author.into(),
            requested_reviewers: vec![],
            updated_at: String::new(),
            merged_at: None,
        }
    }

    const ALL: Notifications = Notifications {
        ci: true,
        review: true,
        merged: true,
        others: false,
    };

    #[test]
    fn first_listing_is_a_baseline() {
        let mut w = Watcher::default();
        let t = Instant::now();
        let prs = [pr(1, "me", PrState::Merged, None)];
        assert!(w.diff("r", &prs, Some("me"), ALL, t).is_empty());
    }

    #[test]
    fn ci_merge_and_review_are_reported() {
        let mut w = Watcher::default();
        let t = Instant::now();
        let mut review = pr(3, "bob", PrState::Open, None);
        let before = [
            pr(1, "me", PrState::Open, Some(CheckState::Pending)),
            pr(2, "me", PrState::Open, None),
            review.clone(),
        ];
        w.diff("r", &before, Some("me"), ALL, t);
        review.requested_reviewers = vec!["me".into()];
        let after = [
            pr(1, "me", PrState::Open, Some(CheckState::Failing)),
            pr(2, "me", PrState::Merged, None),
            review,
        ];
        let notes = w.diff("r", &after, Some("me"), ALL, t + Duration::from_secs(60));
        let titles: Vec<_> = notes.iter().map(|n| n.title.as_str()).collect();
        assert_eq!(
            titles,
            ["r · CI en échec", "r · PR mergée", "r · Review demandée"]
        );
        assert!(notes[0].body.contains("lint"));
        assert_eq!(notes[1].url, "https://github.com/o/r/pull/2");
        // Nothing new: nothing reported again.
        assert!(
            w.diff("r", &after, Some("me"), ALL, t + Duration::from_secs(120))
                .is_empty()
        );
    }

    #[test]
    fn switched_off_and_others_prs_stay_quiet() {
        let t = Instant::now();
        let before = [pr(1, "bob", PrState::Open, Some(CheckState::Pending))];
        let after = [pr(1, "bob", PrState::Merged, Some(CheckState::Passing))];
        let mut w = Watcher::default();
        w.diff("r", &before, Some("me"), ALL, t);
        assert!(
            w.diff("r", &after, Some("me"), ALL, t + Duration::from_secs(1))
                .is_empty()
        );

        let mut w = Watcher::default();
        let others = Notifications {
            others: true,
            ..ALL
        };
        w.diff("r", &before, Some("me"), others, t);
        assert_eq!(
            w.diff("r", &after, Some("me"), others, t + Duration::from_secs(1))
                .len(),
            1
        );

        let mut w = Watcher::default();
        let off = Notifications {
            others: true,
            ..Notifications::default()
        };
        w.diff("r", &before, Some("me"), off, t);
        assert!(
            w.diff("r", &after, Some("me"), off, t + Duration::from_secs(1))
                .is_empty()
        );
    }

    #[test]
    fn a_long_gap_resets_the_baseline() {
        let mut w = Watcher::default();
        let t = Instant::now();
        w.diff("r", &[pr(1, "me", PrState::Open, None)], Some("me"), ALL, t);
        let later = t + MAX_GAP + Duration::from_secs(1);
        assert!(
            w.diff(
                "r",
                &[pr(1, "me", PrState::Merged, None)],
                Some("me"),
                ALL,
                later
            )
            .is_empty()
        );
    }
}
