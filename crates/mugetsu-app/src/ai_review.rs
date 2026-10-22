//! "Review with AI" on a PR: the command from Settings > AI, with the PR's
//! details filled in, run inside the repository in the terminal picked there
//! (or opened as a Claude Code session in the Claude desktop app).

use std::path::Path;
use std::sync::OnceLock;

use mugetsu_forge::PullRequest;
#[cfg(not(windows))]
use mugetsu_git::process::command;

use crate::settings::AiTerminal;

/// Fills `{url}`, `{number}`, `{branch}`, `{base}` and `{title}` in `template`.
/// The PR's fields are written by whoever opened it and the result runs in a
/// shell: they are cut down to [`shell_safe`] characters first.
pub fn expand(template: &str, pr: &PullRequest) -> String {
    template
        .replace("{url}", &shell_safe(&pr.url))
        .replace("{number}", &pr.number.to_string())
        .replace("{branch}", &shell_safe(&pr.head))
        .replace("{base}", &shell_safe(&pr.base))
        .replace("{title}", &shell_safe(&pr.title))
}

/// `s` without anything a shell (sh, PowerShell, cmd) could expand or act on,
/// quoted or not: `$`, backticks, `;`, `&`, `|`, `%`, quotes, newlines… become
/// spaces, and so does a `-` starting a word (no `--option` in an unquoted spot).
fn shell_safe(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let word_start = out.is_empty() || out.ends_with(' ');
        let keep = c.is_alphanumeric() || " _.,:/#@+=".contains(c) || (c == '-' && !word_start);
        out.push(if keep { c } else { ' ' });
    }
    out
}

/// `'…'` for a POSIX shell.
#[cfg_attr(windows, allow(dead_code))]
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The shell line typed in the terminal: cd into the repository, then the command.
#[cfg_attr(windows, allow(dead_code))]
pub fn shell_line(dir: &Path, command: &str) -> String {
    format!("cd {} && {}", sh_quote(&dir.to_string_lossy()), command)
}

/// Where the review can run on this machine: the system terminal, the other
/// installed ones, and the Claude desktop app when it is installed.
pub fn terminals() -> &'static [AiTerminal] {
    static FOUND: OnceLock<Vec<AiTerminal>> = OnceLock::new();
    FOUND.get_or_init(|| {
        let mut all = platform::terminals();
        if platform::has_claude_desktop() {
            all.push(AiTerminal::Desktop);
        }
        all
    })
}

/// Opens the review where Settings > AI says; Err carries a user-facing reason.
pub fn launch(dir: &Path, pr: &PullRequest, cx: &gpui_kit::App) -> anyhow::Result<()> {
    let settings = crate::settings::get();
    if settings.ai_command.trim().is_empty() {
        anyhow::bail!(crate::i18n::tr(
            "Aucune commande configurée (Réglages > IA).",
            "No command set (Settings > AI)."
        ));
    }
    let command = expand(&settings.ai_command, pr);
    if settings.ai_terminal == AiTerminal::Desktop {
        let Some(prompt) = claude_prompt(&command) else {
            anyhow::bail!(crate::i18n::tr(
                "Claude Desktop ouvre une session Claude Code : choisissez l'outil Claude Code (Réglages > IA).",
                "Claude Desktop opens a Claude Code session: pick the Claude Code tool (Settings > AI)."
            ));
        };
        cx.open_url(&desktop_url(dir, &prompt));
        return Ok(());
    }
    platform::spawn(settings.ai_terminal, dir, &command)
}

/// What a `claude …` command asks, to prefill a Claude desktop session with;
/// None for another tool.
pub fn claude_prompt(command: &str) -> Option<String> {
    let rest = command.trim().strip_prefix("claude")?;
    if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
        return None;
    }
    let rest = rest.trim();
    let unquoted = ['"', '\'']
        .iter()
        .find_map(|&q| rest.strip_prefix(q)?.strip_suffix(q));
    Some(unquoted.unwrap_or(rest).to_string())
}

/// Claude desktop's link to a new Claude Code session in `dir`, its composer
/// prefilled with `prompt` (the app asks to confirm the folder).
pub fn desktop_url(dir: &Path, prompt: &str) -> String {
    let mut url = String::from("claude://code/new?");
    if !prompt.is_empty() {
        url += &format!("q={}&", percent_encode(prompt));
    }
    url + &format!("folder={}", percent_encode(&dir.to_string_lossy()))
}

/// Encodes everything but RFC 3986 unreserved characters.
fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(target_os = "macos")]
mod platform {
    use std::path::{Path, PathBuf};

    use super::{command, shell_line};
    use crate::settings::AiTerminal;

    /// Application bundle of each terminal offered besides Terminal.app.
    const APPS: [(AiTerminal, &str); 5] = [
        (AiTerminal::Iterm, "iTerm"),
        (AiTerminal::Ghostty, "Ghostty"),
        (AiTerminal::Wezterm, "WezTerm"),
        (AiTerminal::Kitty, "kitty"),
        (AiTerminal::Alacritty, "Alacritty"),
    ];

    fn installed(app: &str) -> bool {
        let bundle = format!("{app}.app");
        let user = dirs::home_dir().map(|h| h.join("Applications"));
        [Some(PathBuf::from("/Applications")), user]
            .into_iter()
            .flatten()
            .any(|dir| dir.join(&bundle).exists())
    }

    pub fn terminals() -> Vec<AiTerminal> {
        let others = APPS.iter().filter(|(_, app)| installed(app));
        std::iter::once(AiTerminal::System)
            .chain(others.map(|&(t, _)| t))
            .collect()
    }

    pub fn has_claude_desktop() -> bool {
        installed("Claude")
    }

    fn osascript(script: &str) -> anyhow::Result<()> {
        let status = command("osascript").args(["-e", script]).status()?;
        anyhow::ensure!(status.success(), "osascript failed");
        Ok(())
    }

    pub fn spawn(terminal: AiTerminal, dir: &Path, cmd: &str) -> anyhow::Result<()> {
        let line = shell_line(dir, cmd);
        let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
        match terminal {
            AiTerminal::Iterm => osascript(&format!(
                "tell application \"iTerm\"\nactivate\nset w to (create window with default profile)\ntell current session of w to write text \"{escaped}\"\nend tell"
            )),
            AiTerminal::Ghostty
            | AiTerminal::Wezterm
            | AiTerminal::Kitty
            | AiTerminal::Alacritty => {
                // A login, interactive shell finds the tool like in the user's
                // own terminal, and stays open once it exits.
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
                let script = format!("{line}; exec {} -l", super::sh_quote(&shell));
                let run = [shell.as_str(), "-lic", script.as_str()];
                let (app, args): (&str, Vec<&str>) = match terminal {
                    AiTerminal::Ghostty => ("Ghostty", [&["-e"][..], &run[..]].concat()),
                    AiTerminal::Wezterm => ("WezTerm", [&["start", "--"][..], &run[..]].concat()),
                    AiTerminal::Kitty => ("kitty", run.to_vec()),
                    _ => ("Alacritty", [&["-e"][..], &run[..]].concat()),
                };
                let status = command("open")
                    .args(["-na", app, "--args"])
                    .args(args)
                    .status()?;
                anyhow::ensure!(status.success(), "open -a {app} failed");
                Ok(())
            }
            _ => osascript(&format!(
                "tell application \"Terminal\"\nactivate\ndo script \"{escaped}\"\nend tell"
            )),
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::os::windows::process::CommandExt as _;
    use std::path::{Path, PathBuf};

    use mugetsu_git::process::command;

    use crate::settings::AiTerminal;

    /// Opens console programs in a window of their own (replaces the
    /// CREATE_NO_WINDOW `command` sets).
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

    /// Windows Terminal's `wt` alias.
    fn wt() -> Option<PathBuf> {
        let alias = dirs::data_local_dir()?.join(r"Microsoft\WindowsApps\wt.exe");
        alias.exists().then_some(alias)
    }

    pub fn terminals() -> Vec<AiTerminal> {
        let system = wt().map(|_| AiTerminal::System);
        system
            .into_iter()
            .chain([AiTerminal::Powershell, AiTerminal::Cmd])
            .collect()
    }

    pub fn has_claude_desktop() -> bool {
        [
            r"HKCU\Software\Classes\claude",
            r"HKLM\Software\Classes\claude",
        ]
        .iter()
        .any(|key| {
            command("reg")
                .args(["query", key])
                .output()
                .is_ok_and(|o| o.status.success())
        })
    }

    pub fn spawn(terminal: AiTerminal, dir: &Path, cmd: &str) -> anyhow::Result<()> {
        // PowerShell takes the command as base64 UTF-16, so no quote in it
        // has to survive Windows' command-line parsing.
        let encoded = super::base64(
            &cmd.encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<u8>>(),
        );
        let powershell = ["powershell", "-NoExit", "-EncodedCommand", encoded.as_str()];
        match (terminal, wt()) {
            (AiTerminal::Cmd, _) => command("cmd")
                .raw_arg(format!("/K {cmd}"))
                .current_dir(dir)
                .creation_flags(CREATE_NEW_CONSOLE)
                .spawn()?,
            (AiTerminal::Powershell, _) | (_, None) => command(powershell[0])
                .args(&powershell[1..])
                .current_dir(dir)
                .creation_flags(CREATE_NEW_CONSOLE)
                .spawn()?,
            (_, Some(wt)) => command(wt).arg("-d").arg(dir).args(powershell).spawn()?,
        };
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use std::path::Path;

    use super::{command, shell_line};
    use crate::settings::AiTerminal;

    pub fn terminals() -> Vec<AiTerminal> {
        vec![AiTerminal::System]
    }

    pub fn has_claude_desktop() -> bool {
        command("xdg-mime")
            .args(["query", "default", "x-scheme-handler/claude"])
            .output()
            .is_ok_and(|o| o.status.success() && !o.stdout.trim_ascii().is_empty())
    }

    pub fn spawn(_: AiTerminal, dir: &Path, cmd: &str) -> anyhow::Result<()> {
        // Keeps the window open once the tool exits.
        let keep = format!("{}; exec $SHELL", shell_line(dir, cmd));
        command("x-terminal-emulator")
            .args(["-e", "sh", "-c", &keep])
            .spawn()?;
        Ok(())
    }
}

/// Standard base64, with padding.
#[cfg_attr(not(windows), allow(dead_code))]
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | ((b as u32) << (16 - 8 * i)));
        for i in 0..4 {
            out.push(if i <= chunk.len() {
                ABC[((n >> (18 - 6 * i)) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
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
            "x 42 fix/it→main \"Fix  it \""
        );
    }

    #[test]
    fn pr_fields_cannot_inject_shell() {
        let mut evil = pr();
        evil.title = "$(rm -rf ~) `id`; echo \"x\" | sh & %PATH% 'q'\nnext".into();
        evil.head = "a;$(id)&b".into();
        let line = expand("tool \"{title}\" {branch}", &evil);
        assert!(!line.contains(['$', '`', ';', '|', '&', '%', '\'', '\n', '(']));
        assert_eq!(line.matches('"').count(), 2);
        evil.title = "fix-it --yolo -x".into();
        assert_eq!(expand("{title}", &evil), "fix-it   yolo  x");
    }

    #[test]
    fn claude_desktop_gets_the_prompt_and_folder() {
        assert_eq!(
            claude_prompt("claude \"/code-review https://x/pull/1\"").as_deref(),
            Some("/code-review https://x/pull/1")
        );
        assert_eq!(claude_prompt(" claude ").as_deref(), Some(""));
        assert_eq!(claude_prompt("codex \"Review\""), None);
        assert_eq!(claude_prompt("claudette x"), None);
        assert_eq!(
            desktop_url(Path::new("/a b/r"), "/code-review https://x/pull/1"),
            "claude://code/new?q=%2Fcode-review%20https%3A%2F%2Fx%2Fpull%2F1&folder=%2Fa%20b%2Fr"
        );
        assert_eq!(
            desktop_url(Path::new("/r"), ""),
            "claude://code/new?folder=%2Fr"
        );
    }

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // PowerShell -EncodedCommand of `dir`: UTF-16LE.
        let utf16: Vec<u8> = "dir".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(base64(&utf16), "ZABpAHIA");
    }

    #[test]
    fn cds_into_the_repository_first() {
        assert_eq!(
            shell_line(Path::new("/a/it's"), "claude"),
            r"cd '/a/it'\''s' && claude"
        );
    }
}
