//! Error toasts: git's stderr turned into a title, a sentence and, when the
//! failure has a known way out, a button.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{App, IntoElement as _, ParentElement as _, SharedString, Styled as _, Window, div};

/// A toast button: its label and what it does.
pub type Action = (SharedString, Rc<dyn Fn(&mut Window, &mut App)>);

/// What a git error says once its plumbing is gone.
#[derive(Debug, PartialEq, Eq)]
pub struct GitError {
    /// The error itself, `error:` / `fatal:` prefixes dropped.
    pub message: String,
    /// git's `hint:` lines, minus the ones about silencing hints.
    pub hints: Vec<String>,
}

impl GitError {
    pub fn parse(raw: &str) -> Self {
        let mut message: Vec<&str> = Vec::new();
        let mut hints: Vec<String> = Vec::new();
        for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if let Some(hint) = line.strip_prefix("hint:") {
                let hint = hint.trim();
                if hint.is_empty() || hint.starts_with("Disable this message") {
                    continue;
                }
                // Hints wrap over several `hint:` lines: glue them back.
                match hints.last_mut() {
                    Some(last) if !last.ends_with('.') && !starts_upper(hint) => {
                        last.push(' ');
                        last.push_str(hint);
                    }
                    _ => hints.push(hint.to_string()),
                }
                continue;
            }
            let line = ["error:", "fatal:", "warning:"]
                .iter()
                .find_map(|p| line.strip_prefix(p))
                .unwrap_or(line)
                .trim();
            message.push(line);
        }
        let mut message = message.join(" ");
        if let Some(first) = message.get(..1) {
            message.replace_range(..1, &first.to_uppercase());
        }
        Self { message, hints }
    }
}

fn starts_upper(s: &str) -> bool {
    s.chars().next().is_some_and(char::is_uppercase)
}

/// `git branch -d` refused because the branch isn't merged.
pub fn is_unmerged_branch(e: &anyhow::Error) -> bool {
    format!("{e:#}").contains("is not fully merged")
}

/// An error toast titled `title` showing `e`; `action` adds a button below the
/// text (and keeps the toast up until it is used or closed).
pub fn error(
    title: impl Into<SharedString>,
    e: &anyhow::Error,
    action: Option<Action>,
) -> Notification {
    let parsed = GitError::parse(&format!("{e:#}"));
    // A button replaces git's "run this command" advice.
    let hints = if action.is_some() {
        Vec::new()
    } else {
        parsed.hints
    };
    let keep = action.is_some();
    Notification::error(parsed.message)
        .title(title)
        .autohide(!keep)
        .content(move |_, _, cx| {
            let muted = cx.theme().muted_foreground;
            v_flex()
                .children(
                    hints
                        .iter()
                        .map(|h| div().pt_1().text_xs().text_color(muted).child(h.clone())),
                )
                .children(action.clone().map(|(label, run)| {
                    h_flex().pt_2().child(
                        Button::new("toast-action")
                            .danger()
                            .small()
                            .label(label)
                            .on_click(cx.listener(move |note, _, window, cx| {
                                note.dismiss(window, cx);
                                run(window, cx);
                            })),
                    )
                }))
                .into_any_element()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmerged_branch_error_reads_as_a_sentence() {
        let raw = "error: the branch 'backup/monitor' is not fully merged\n\
                   hint: If you are sure you want to delete it, run 'git branch -D backup/monitor'\n\
                   hint: Disable this message with \"git config set advice.forceDeleteBranch false\"";
        let e = GitError::parse(raw);
        assert_eq!(e.message, "The branch 'backup/monitor' is not fully merged");
        assert_eq!(
            e.hints,
            ["If you are sure you want to delete it, run 'git branch -D backup/monitor'"]
        );
        assert!(is_unmerged_branch(&anyhow::anyhow!("{raw}")));
    }

    #[test]
    fn wrapped_hints_are_glued_back() {
        let raw = "fatal: Need to specify how to reconcile divergent branches.\n\
                   hint: You have divergent branches and need to specify how to\n\
                   hint: reconcile them.";
        let e = GitError::parse(raw);
        assert_eq!(
            e.message,
            "Need to specify how to reconcile divergent branches."
        );
        assert_eq!(
            e.hints,
            ["You have divergent branches and need to specify how to reconcile them."]
        );
    }
}
