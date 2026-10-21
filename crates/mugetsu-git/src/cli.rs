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
        let mut cmd = crate::process::command(git_binary());
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

    /// Like [`Self::run`], with extra environment variables (e.g. the editors
    /// of a scripted `rebase -i`).
    pub fn run_with_env<I, S>(&self, args: I, envs: &[(&str, &OsStr)]) -> anyhow::Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = self.command(args);
        cmd.envs(envs.iter().copied());
        let out = cmd
            .output()
            .with_context(|| format!("failed to spawn {cmd:?}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            bail!(
                "{}",
                if stderr.trim().is_empty() {
                    stdout
                } else {
                    stderr
                }
                .trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Runs git and returns whether it succeeded, stdout and stderr (e.g.
    /// `push --porcelain`, which reports refused refs on stdout).
    pub fn run_full<I, S>(&self, args: I) -> anyhow::Result<(bool, String, String)>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = self.command(args);
        let out = cmd
            .output()
            .with_context(|| format!("failed to spawn {cmd:?}"))?;
        Ok((
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ))
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

    /// Starts git with stdout piped, for output read as it streams.
    pub fn spawn<I, S>(&self, args: I) -> anyhow::Result<std::process::Child>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = self.command(args);
        cmd.stdout(Stdio::piped()).stderr(Stdio::null());
        cmd.spawn()
            .with_context(|| format!("failed to spawn {cmd:?}"))
    }

    /// Starts git with stderr piped (where it reports progress), stdout dropped.
    pub fn spawn_stderr<I, S>(&self, args: I) -> anyhow::Result<std::process::Child>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = self.command(args);
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());
        cmd.spawn()
            .with_context(|| format!("failed to spawn {cmd:?}"))
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

/// Author identity commits are made with: `user.name` and `user.email`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

/// Where an identity is read from or written to: `["--global"]`, or
/// `["--file", path]` in tests.
fn identity_in(scope: &[&str]) -> Identity {
    identity_with(&Git::new(std::env::temp_dir()), scope)
}

fn identity_with(git: &Git, scope: &[&str]) -> Identity {
    let get = |key: &str| {
        let mut args = vec!["config"];
        args.extend_from_slice(scope);
        args.extend(["--get", key]);
        git.run(args)
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    Identity {
        name: get("user.name"),
        email: get("user.email"),
    }
}

fn set_identity_in(scope: &[&str], id: &Identity) -> anyhow::Result<()> {
    let git = Git::new(std::env::temp_dir());
    for (key, value) in [("user.name", &id.name), ("user.email", &id.email)] {
        let mut args = vec!["config"];
        args.extend_from_slice(scope);
        let value = value.trim();
        if value.is_empty() {
            args.extend(["--unset", key]);
            // Unsetting a missing key fails harmlessly (exit 5).
            let _ = git.run(args);
        } else {
            args.extend([key, value]);
            git.run(args)?;
        }
    }
    Ok(())
}

/// The identity in `~/.gitconfig` (empty fields when unset).
pub fn global_identity() -> Identity {
    identity_in(&["--global"])
}

impl crate::Repo {
    /// Identity commits are made with here: the repository's config, else
    /// the global one.
    pub fn identity(&self) -> Identity {
        identity_with(self.git(), &[])
    }
}

/// Writes the identity to `~/.gitconfig`; an empty field is unset.
pub fn set_global_identity(id: &Identity) -> anyhow::Result<()> {
    set_identity_in(&["--global"], id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_round_trips_through_a_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config");
        let path = file.to_str().unwrap();
        let scope = ["--file", path];
        assert_eq!(identity_in(&scope), Identity::default());
        let id = Identity {
            name: "Ada Lovelace".into(),
            email: "ada@example.com".into(),
        };
        set_identity_in(&scope, &id).unwrap();
        assert_eq!(identity_in(&scope), id);
        set_identity_in(
            &scope,
            &Identity {
                name: "Ada".into(),
                email: String::new(),
            },
        )
        .unwrap();
        assert_eq!(identity_in(&scope).email, "");
    }
}
