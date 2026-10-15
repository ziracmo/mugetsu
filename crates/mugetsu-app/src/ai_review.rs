//! "Review with AI" on a PR: the command from Settings > AI, with the PR's
//! details filled in, run in a new terminal window inside the repository.

use std::path::Path;

use mugetsu_forge::PullRequest;
use mugetsu_git::process::command;

/// Fills `{url}`, `{number}`, `{branch}`, `{base}` and `{title}` in `template`.
pub fn expand(template: &str, pr: &PullRequest) -> String {
    template
        .replace("{url}", &pr.url)
        .replace("{number}", &pr.number.to_string())
        .replace("{branch}", &pr.head)
        .replace("{base}", &pr.base)
        .replace("{title}", &pr.title.replace('"', "'"))
}

/// `'…'` for a POSIX shell.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The shell line typed in the terminal: cd into the repository, then the command.
pub fn shell_line(dir: &Path, command: &str) -> String {
    format!("cd {} && {}", sh_quote(&dir.to_string_lossy()), command)
}

/// Opens a terminal running the review; Err carries a user-facing reason.
pub fn launch(dir: &Path, pr: &PullRequest) -> anyhow::Result<()> {
    let template = crate::settings::get().ai_command;
    if template.trim().is_empty() {
        anyhow::bail!(crate::i18n::tr(
            "Aucune commande configurée (Réglages > IA).",
            "No command set (Settings > AI)."
        ));
    }
    let line = shell_line(dir, &expand(&template, pr));
    spawn_terminal(&line)
}

#[cfg(target_os = "macos")]
fn spawn_terminal(line: &str) -> anyhow::Result<()> {
    let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
    let script =
        format!("tell application \"Terminal\"\nactivate\ndo script \"{escaped}\"\nend tell");
    let status = command("osascript").args(["-e", &script]).status()?;
    anyhow::ensure!(status.success(), "osascript failed");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn spawn_terminal(line: &str) -> anyhow::Result<()> {
    // Keeps the window open once the tool exits.
    let keep = format!("{line}; exec $SHELL");
    command("x-terminal-emulator")
        .args(["-e", "sh", "-c", &keep])
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr() -> PullRequest {
        PullRequest {
            number: 42,
            title: "Fix \"it\"".into(),
            url: "https://github.com/o/r/pull/42".into(),
            state: mugetsu_forge::PrState::Open,
            status: None,
            head: "fix/it".into(),
            base: "main".into(),
            same_repo: true,
            author: "me".into(),
            requested_reviewers: vec![],
            updated_at: String::new(),
            merged_at: None,
        }
    }

    #[test]
    fn fills_the_pr_into_the_command() {
        let preset = crate::settings::AiTool::Claude.preset().unwrap();
        assert_eq!(
            expand(preset, &pr()),
            "claude \"/code-review https://github.com/o/r/pull/42\""
        );
        assert_eq!(
            expand("x {number} {branch}→{base} \"{title}\"", &pr()),
            "x 42 fix/it→main \"Fix 'it'\""
        );
    }

    #[test]
    fn cds_into_the_repository_first() {
        assert_eq!(
            shell_line(Path::new("/a/it's"), "claude"),
            r"cd '/a/it'\''s' && claude"
        );
    }
}
