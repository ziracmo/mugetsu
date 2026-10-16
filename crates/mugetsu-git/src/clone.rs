//! `git clone` with live progress, cancellable.

use std::io::Read as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context as _, bail};

use crate::Git;

/// Step of a clone, in the order git goes through them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClonePhase {
    Connecting,
    Counting,
    Compressing,
    Receiving,
    Resolving,
    Checkout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloneProgress {
    pub phase: ClonePhase,
    /// Within the phase, 0 to 100.
    pub percent: u8,
}

impl CloneProgress {
    pub const START: Self = Self {
        phase: ClonePhase::Connecting,
        percent: 0,
    };

    /// Whole clone, 0 to 1: receiving objects weighs the most.
    pub fn overall(&self) -> f32 {
        let (from, to) = match self.phase {
            ClonePhase::Connecting => (0., 0.02),
            ClonePhase::Counting => (0.02, 0.05),
            ClonePhase::Compressing => (0.05, 0.1),
            ClonePhase::Receiving => (0.1, 0.8),
            ClonePhase::Resolving => (0.8, 0.95),
            ClonePhase::Checkout => (0.95, 1.),
        };
        from + (to - from) * f32::from(self.percent.min(100)) / 100.
    }
}

/// One line of `git clone --progress` output, e.g.
/// `Receiving objects:  45% (450/1000), 1.20 MiB | 2.00 MiB/s`.
pub fn parse_progress(line: &str) -> Option<CloneProgress> {
    let line = line.trim();
    let line = line.strip_prefix("remote:").unwrap_or(line).trim();
    let (label, rest) = line.split_once(':')?;
    let phase = match label.trim() {
        "Enumerating objects" | "Counting objects" => ClonePhase::Counting,
        "Compressing objects" => ClonePhase::Compressing,
        "Receiving objects" => ClonePhase::Receiving,
        "Resolving deltas" => ClonePhase::Resolving,
        "Updating files" | "Checking out files" => ClonePhase::Checkout,
        _ => return None,
    };
    let percent = rest
        .trim()
        .split_once('%')
        .and_then(|(n, _)| n.trim().parse::<u8>().ok())
        .unwrap_or(0);
    Some(CloneProgress { phase, percent })
}

/// Looks like something `git clone` takes: `https://…`, `ssh://…`, `git@host:path`.
pub fn is_clone_url(text: &str) -> bool {
    let t = text.trim();
    if t.contains(char::is_whitespace) || t.is_empty() {
        return false;
    }
    if let Some((scheme, rest)) = t.split_once("://") {
        return match scheme {
            "file" => rest.starts_with('/') && rest.len() > 1,
            "https" | "http" | "ssh" | "git" | "git+ssh" => {
                rest.contains('/') && !rest.starts_with('/')
            }
            _ => false,
        };
    }
    // scp-like: user@host:path
    t.split_once(':').is_some_and(|(authority, path)| {
        authority.contains('@') && !authority.contains('/') && !path.is_empty()
    })
}

/// Folder name git would pick for `url`: its last path segment, without `.git`.
pub fn clone_name(url: &str) -> Option<String> {
    let t = url.trim().trim_end_matches('/');
    let t = t.strip_suffix(".git").unwrap_or(t);
    let last = t.rsplit(['/', ':']).next()?;
    (!last.is_empty() && !last.contains('@')).then(|| last.to_string())
}

/// Clones `url` into `dest` (which must not exist yet), reporting progress.
/// Setting `cancel` stops it and removes what was written.
pub fn clone(
    url: &str,
    dest: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(CloneProgress),
) -> anyhow::Result<()> {
    if dest.exists() {
        bail!("{} already exists", dest.display());
    }
    let parent = dest.parent().context("no parent folder")?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let mut child = Git::new(parent).spawn_stderr([
        "clone".as_ref(),
        "--progress".as_ref(),
        "--".as_ref(),
        url.as_ref(),
        dest.as_os_str(),
    ])?;
    let mut stderr = child.stderr.take().context("no stderr")?;
    // Read on a thread so cancelling never waits on a quiet git.
    let (tx, rx) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut pending = String::new();
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            pending.push_str(&String::from_utf8_lossy(&buf[..n]));
            // Progress lines end with \r, messages with \n.
            while let Some(i) = pending.find(['\r', '\n']) {
                let line: String = pending.drain(..=i).collect();
                if tx.send(line).is_err() {
                    return;
                }
            }
        }
        if !pending.trim().is_empty() {
            let _ = tx.send(pending);
        }
    });
    let mut messages: Vec<String> = Vec::new();
    let mut last = CloneProgress::START;
    progress(last);
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            let _ = std::fs::remove_dir_all(dest);
            bail!("cancelled");
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => match parse_progress(&line) {
                Some(p) => {
                    if p != last {
                        last = p;
                        progress(p);
                    }
                }
                None if !line.trim().is_empty() => messages.push(line.trim().to_string()),
                None => {}
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break child.wait()?,
        }
    };
    let _ = reader.join();
    if !status.success() {
        let errors: Vec<&String> = messages
            .iter()
            .filter(|m| m.starts_with("fatal:") || m.starts_with("error:"))
            .collect();
        let text = if errors.is_empty() {
            messages.last().cloned().unwrap_or_default()
        } else {
            errors
                .iter()
                .map(|m| m.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        };
        bail!("{}", text.trim_start_matches("fatal:").trim());
    }
    progress(CloneProgress {
        phase: ClonePhase::Checkout,
        percent: 100,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_lines() {
        let p = |l| parse_progress(l).map(|p| (p.phase, p.percent));
        assert_eq!(
            p("remote: Counting objects:  12% (1/8)"),
            Some((ClonePhase::Counting, 12))
        );
        assert_eq!(
            p("Receiving objects:  45% (450/1000), 1.20 MiB | 2.00 MiB/s\r"),
            Some((ClonePhase::Receiving, 45))
        );
        assert_eq!(
            p("Resolving deltas: 100% (3/3), done.\n"),
            Some((ClonePhase::Resolving, 100))
        );
        assert_eq!(
            p("remote: Enumerating objects: 5, done."),
            Some((ClonePhase::Counting, 0))
        );
        assert_eq!(p("Cloning into 'x'..."), None);
        assert_eq!(p("fatal: repository not found"), None);
        let half = CloneProgress {
            phase: ClonePhase::Receiving,
            percent: 50,
        };
        assert!((half.overall() - 0.45).abs() < 1e-6);
    }

    #[test]
    fn recognises_clone_urls() {
        for url in [
            "https://github.com/o/r.git",
            "git@github.com:o/r.git",
            "ssh://git@host:22/o/r",
            "file:///srv/repos/app",
        ] {
            assert!(is_clone_url(url), "{url}");
        }
        for text in [
            "mugetsu",
            "C:\\repos\\x",
            "https://",
            "two words",
            "host:path",
        ] {
            assert!(!is_clone_url(text), "{text}");
        }
        assert_eq!(
            clone_name("git@github.com:o/repo.git").as_deref(),
            Some("repo")
        );
        assert_eq!(
            clone_name("https://gitlab.com/g/sub/proj/").as_deref(),
            Some("proj")
        );
    }

    #[test]
    fn clones_a_local_repository_with_progress() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        let git = Git::new(&src);
        git.run(["init", "-q", "-b", "main"]).unwrap();
        std::fs::write(src.join("a.txt"), "a\n").unwrap();
        git.run(["add", "a.txt"]).unwrap();
        git.run([
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@e",
            "commit",
            "-q",
            "-m",
            "c1",
        ])
        .unwrap();
        let dest = dir.path().join("out/copy");
        let mut seen = Vec::new();
        let url = format!("file://{}", src.display());
        clone(&url, &dest, &AtomicBool::new(false), |p| seen.push(p)).unwrap();
        assert!(dest.join("a.txt").exists());
        assert_eq!(seen.first(), Some(&CloneProgress::START));
        assert_eq!(seen.last().map(|p| p.overall()), Some(1.));
        let err = clone(&url, &dest, &AtomicBool::new(false), |_| {}).unwrap_err();
        assert!(err.to_string().contains("already exists"));
        let missing = dir.path().join("nope");
        let err = clone(
            &format!("file://{}", missing.display()),
            &dir.path().join("x"),
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap_err();
        assert!(!err.to_string().is_empty());
        let cancelled = clone(&url, &dir.path().join("c"), &AtomicBool::new(true), |_| {});
        assert_eq!(cancelled.unwrap_err().to_string(), "cancelled");
        assert!(!dir.path().join("c").exists());
    }
}
