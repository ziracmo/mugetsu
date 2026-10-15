use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use anyhow::{Context, bail};

static GIT_BINARY: OnceLock<PathBuf> = OnceLock::new();

/// Overrides the `git` executable (macOS GUI apps don't inherit the shell PATH).
pub fn set_git_binary(path: PathBuf) {
    let _ = GIT_BINARY.set(path);
}

fn git_binary() -> &'static Path {
    GIT_BINARY.get_or_init(|| PathBuf::from("git"))
}

/// Thin wrapper around the `git` executable, bound to a working directory.
#[derive(Clone, Debug)]
pub struct Git {
    cwd: PathBuf,
}

impl Git {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }

    fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = Command::new(git_binary());
        cmd.current_dir(&self.cwd)
            // Never block a background job on an interactive prompt.
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("LC_ALL", "C")
            .args(["-c", "core.quotepath=false", "-c", "color.ui=false"])
            .args(args)
            .stdin(Stdio::null());
        cmd
    }

    /// Runs git and returns stdout as UTF-8 (lossy). Fails with stderr on non-zero exit.
    pub fn run<I, S>(&self, args: I) -> anyhow::Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let bytes = self.run_bytes(args)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub fn run_bytes<I, S>(&self, args: I) -> anyhow::Result<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = self.command(args);
        let out = cmd
            .output()
            .with_context(|| format!("failed to spawn {cmd:?}"))?;
        if !out.status.success() {
            bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(out.stdout)
    }

    /// Runs git and returns stdout whatever the exit code (e.g. `diff --no-index`).
    pub fn run_lenient<I, S>(&self, args: I) -> anyhow::Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = self.command(args);
        let out = cmd
            .output()
            .with_context(|| format!("failed to spawn {cmd:?}"))?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Runs git feeding `input` on stdin.
    pub fn run_with_stdin<I, S>(&self, args: I, input: &[u8]) -> anyhow::Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        use std::io::Write;
        let mut cmd = self.command(args);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn {cmd:?}"))?;
        child.stdin.take().expect("piped stdin").write_all(input)?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}
