//! Merge conflicts: which operation stopped on them, the three versions of a
//! conflicted file split into clean and conflicting chunks, and the commands
//! that resolve, continue or abort.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::Repo;

/// The operation that left the repository with conflicts (or mid-way).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl Operation {
    pub fn name(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::Rebase => "Rebase",
            Self::CherryPick => "Cherry-pick",
            Self::Revert => "Revert",
        }
    }

    /// Rebase and cherry-pick can drop the commit being applied.
    pub fn can_skip(self) -> bool {
        matches!(self, Self::Rebase | Self::CherryPick | Self::Revert)
    }
}

/// An operation in progress, with what each side of its conflicts is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationState {
    pub op: Operation,
    /// Stage 2: what HEAD holds (the branch, or the rebase target).
    pub ours: String,
    /// Stage 3: what is being brought in.
    pub theirs: String,
    /// Rebase progress: (current step, total).
    pub step: Option<(usize, usize)>,
}

/// One side picked for a conflicting chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Resolution {
    Ours,
    Theirs,
    OursThenTheirs,
    TheirsThenOurs,
    /// Drop both sides.
    Neither,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conflict {
    pub ours: Vec<String>,
    pub base: Vec<String>,
    pub theirs: Vec<String>,
}

impl Conflict {
    /// Lines the resolution keeps (each line keeps its newline).
    pub fn lines(&self, r: Resolution) -> Vec<&str> {
        let (a, b): (&[String], &[String]) = match r {
            Resolution::Ours => (&self.ours, &[]),
            Resolution::Theirs => (&self.theirs, &[]),
            Resolution::OursThenTheirs => (&self.ours, &self.theirs),
            Resolution::TheirsThenOurs => (&self.theirs, &self.ours),
            Resolution::Neither => (&[], &[]),
        };
        a.iter().chain(b).map(String::as_str).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chunk {
    /// Merged without conflict.
    Clean(Vec<String>),
    Conflict(Conflict),
}

/// A conflicted path: its three versions and their line merge.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConflictFile {
    pub path: String,
    pub base: Option<Vec<u8>>,
    /// `None` when deleted on that side.
    pub ours: Option<Vec<u8>>,
    pub theirs: Option<Vec<u8>>,
    pub chunks: Vec<Chunk>,
}

impl ConflictFile {
    /// Deleted on one side, or binary: only a whole-file choice makes sense.
    pub fn whole_file_only(&self) -> bool {
        let binary = |v: &Option<Vec<u8>>| v.as_deref().is_some_and(|b| b.contains(&0));
        self.ours.is_none()
            || self.theirs.is_none()
            || binary(&self.ours)
            || binary(&self.theirs)
            || binary(&self.base)
    }

    pub fn conflicts(&self) -> impl Iterator<Item = &Conflict> {
        self.chunks.iter().filter_map(|c| match c {
            Chunk::Conflict(c) => Some(c),
            Chunk::Clean(_) => None,
        })
    }

    pub fn conflict_count(&self) -> usize {
        self.conflicts().count()
    }

    /// The merged text, `choices` giving one resolution per conflict in
    /// order. Unresolved conflicts keep git's markers.
    pub fn render(&self, choices: &[Option<Resolution>]) -> String {
        self.render_with(|i, c| choices.get(i).copied().flatten().map(|r| c.lines(r)))
    }

    /// The merged text, `pick` giving the lines kept for the n-th conflict
    /// (any mix of both sides). `None` keeps git's markers.
    pub fn render_with<'a>(
        &'a self,
        mut pick: impl FnMut(usize, &'a Conflict) -> Option<Vec<&'a str>>,
    ) -> String {
        let mut out = String::new();
        let mut i = 0;
        for chunk in &self.chunks {
            match chunk {
                Chunk::Clean(lines) => lines.iter().for_each(|l| out.push_str(l)),
                Chunk::Conflict(c) => {
                    match pick(i, c) {
                        Some(lines) => lines.into_iter().for_each(|l| out.push_str(l)),
                        None => {
                            let block = |out: &mut String, marker: &str, lines: &[String]| {
                                out.push_str(marker);
                                lines.iter().for_each(|l| push_line(out, l));
                            };
                            block(&mut out, "<<<<<<< ours\n", &c.ours);
                            block(&mut out, "||||||| base\n", &c.base);
                            block(&mut out, "=======\n", &c.theirs);
                            out.push_str(">>>>>>> theirs\n");
                        }
                    }
                    i += 1;
                }
            }
        }
        out
    }
}

/// Appends a line, ending it so a following marker starts on its own line.
fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    if !line.ends_with('\n') {
        out.push('\n');
    }
}

/// Longer than any marker a file plausibly contains.
const MARKER: usize = 17;

/// Splits `git merge-file -p --diff3` output (markers of [`MARKER`] chars)
/// into chunks.
pub fn parse_merge(text: &str) -> Vec<Chunk> {
    let open = "<".repeat(MARKER);
    let mid_base = "|".repeat(MARKER);
    let sep = "=".repeat(MARKER);
    let close = ">".repeat(MARKER);
    let is = |line: &str, m: &str| {
        line.strip_prefix(m)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\n', '\r']))
    };
    enum State {
        Clean,
        Ours,
        Base,
        Theirs,
    }
    let mut chunks = Vec::new();
    let mut clean = Vec::new();
    let mut conflict = Conflict::default();
    let mut state = State::Clean;
    for line in text.split_inclusive('\n') {
        match state {
            State::Clean if is(line, &open) => {
                if !clean.is_empty() {
                    chunks.push(Chunk::Clean(std::mem::take(&mut clean)));
                }
                state = State::Ours;
            }
            State::Clean => clean.push(line.to_string()),
            State::Ours if is(line, &mid_base) => state = State::Base,
            State::Ours | State::Base if is(line, &sep) => state = State::Theirs,
            State::Ours => conflict.ours.push(line.to_string()),
            State::Base => conflict.base.push(line.to_string()),
            State::Theirs if is(line, &close) => {
                chunks.push(Chunk::Conflict(std::mem::take(&mut conflict)));
                state = State::Clean;
            }
            State::Theirs => conflict.theirs.push(line.to_string()),
        }
    }
    if !clean.is_empty() {
        chunks.push(Chunk::Clean(clean));
    }
    chunks
}

/// A scratch file removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(bytes: &[u8]) -> anyhow::Result<Self> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "mugetsu-merge-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, bytes)?;
        Ok(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl Repo {
    fn git_path(&self, name: &str) -> anyhow::Result<PathBuf> {
        let p = self.git().run(["rev-parse", "--git-path", name])?;
        Ok(self.root().join(p.trim()))
    }

    /// The merge, rebase, cherry-pick or revert in progress, if any.
    pub fn operation(&self) -> anyhow::Result<Option<OperationState>> {
        let exists = |name: &str| self.git_path(name).map(|p| p.exists());
        let op = if exists("rebase-merge")? || exists("rebase-apply")? {
            Operation::Rebase
        } else if exists("MERGE_HEAD")? {
            Operation::Merge
        } else if exists("CHERRY_PICK_HEAD")? {
            Operation::CherryPick
        } else if exists("REVERT_HEAD")? {
            Operation::Revert
        } else {
            return Ok(None);
        };
        let describe = |rev: &str| {
            self.git()
                .run(["log", "-1", "--format=%h %s", rev])
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        };
        let name = |rev: &str| {
            self.git()
                .run(["name-rev", "--name-only", "--exclude=tags/*", rev])
                .map(|s| s.trim().to_string())
                .ok()
                .filter(|s| !s.is_empty() && s != "undefined")
                .unwrap_or_else(|| describe(rev))
        };
        let head = self
            .git()
            .run(["symbolic-ref", "--short", "-q", "HEAD"])
            .map(|s| s.trim().to_string())
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| describe("HEAD"));
        let read = |name: &str| {
            self.git_path(name)
                .ok()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|s| s.trim().to_string())
        };
        let state = match op {
            Operation::Merge => OperationState {
                op,
                ours: head,
                theirs: name("MERGE_HEAD"),
                step: None,
            },
            Operation::Rebase => {
                let dir = if exists("rebase-merge")? {
                    "rebase-merge"
                } else {
                    "rebase-apply"
                };
                let onto = read(&format!("{dir}/head-name"))
                    .map(|h| h.trim_start_matches("refs/heads/").to_string());
                let step = read(&format!("{dir}/msgnum"))
                    .or_else(|| read(&format!("{dir}/next")))
                    .and_then(|n| n.parse().ok())
                    .zip(
                        read(&format!("{dir}/end"))
                            .or_else(|| read(&format!("{dir}/last")))
                            .and_then(|n| n.parse().ok()),
                    );
                OperationState {
                    op,
                    ours: describe("HEAD"),
                    theirs: if exists("REBASE_HEAD")? {
                        describe("REBASE_HEAD")
                    } else {
                        onto.unwrap_or_default()
                    },
                    step,
                }
            }
            Operation::CherryPick => OperationState {
                op,
                ours: head,
                theirs: describe("CHERRY_PICK_HEAD"),
                step: None,
            },
            Operation::Revert => OperationState {
                op,
                ours: head,
                theirs: format!("revert {}", describe("REVERT_HEAD")),
                step: None,
            },
        };
        Ok(Some(state))
    }

    /// Paths with unmerged index entries.
    pub fn conflicted_paths(&self) -> anyhow::Result<Vec<String>> {
        let raw = self
            .git()
            .run(["diff", "--name-only", "--diff-filter=U", "-z"])?;
        let mut paths: Vec<String> = raw
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
        paths.dedup();
        Ok(paths)
    }

    /// The three versions of `path` (index stages 1–3) and their line merge.
    pub fn conflict_file(&self, path: &str) -> anyhow::Result<ConflictFile> {
        let raw = self.git().run(["ls-files", "-u", "-z", "--", path])?;
        let mut file = ConflictFile {
            path: path.to_string(),
            ..Default::default()
        };
        for rec in raw.split('\0').filter(|r| !r.is_empty()) {
            // "<mode> <oid> <stage>\t<path>"
            let Some((meta, _)) = rec.split_once('\t') else {
                continue;
            };
            let mut f = meta.split(' ');
            let (_, Some(oid), Some(stage)) = (f.next(), f.next(), f.next()) else {
                continue;
            };
            let blob = self.git().run_bytes(["cat-file", "blob", oid])?;
            match stage {
                "1" => file.base = Some(blob),
                "2" => file.ours = Some(blob),
                "3" => file.theirs = Some(blob),
                _ => {}
            }
        }
        if file.ours.is_none() && file.theirs.is_none() {
            anyhow::bail!("{path} is not conflicted");
        }
        if !file.whole_file_only() {
            let empty = Vec::new();
            let ours = Scratch::new(file.ours.as_ref().unwrap_or(&empty))?;
            let base = Scratch::new(file.base.as_ref().unwrap_or(&empty))?;
            let theirs = Scratch::new(file.theirs.as_ref().unwrap_or(&empty))?;
            // Exit code is the number of conflicts: read stdout regardless.
            let marker = format!("--marker-size={MARKER}");
            let args: [&std::ffi::OsStr; 7] = [
                "merge-file".as_ref(),
                "-p".as_ref(),
                "--diff3".as_ref(),
                marker.as_ref(),
                ours.0.as_os_str(),
                base.0.as_os_str(),
                theirs.0.as_os_str(),
            ];
            let merged = self.git().run_lenient(args)?;
            file.chunks = parse_merge(&merged);
        }
        Ok(file)
    }

    /// Writes the resolved content of `path` and marks it resolved.
    pub fn resolve_with(&self, path: &str, content: &str) -> anyhow::Result<()> {
        std::fs::write(self.root().join(path), content)?;
        self.git().run(["add", "--", path]).map(drop)
    }

    /// Keeps one side's whole file (deleting it when that side deleted it).
    pub fn resolve_side(&self, path: &str, ours: bool) -> anyhow::Result<()> {
        let stage = if ours { "2" } else { "3" };
        let present = self
            .git()
            .run(["ls-files", "-u", "-z", "--", path])?
            .split('\0')
            .any(|r| r.split(['\t']).next().and_then(|m| m.split(' ').nth(2)) == Some(stage));
        if present {
            self.git().run([
                "checkout",
                if ours { "--ours" } else { "--theirs" },
                "--",
                path,
            ])?;
            self.git().run(["add", "--", path]).map(drop)
        } else {
            self.git().run(["rm", "-q", "--", path]).map(drop)
        }
    }

    /// Finishes the operation once its conflicts are resolved. `Ok(false)`:
    /// it moved on and stopped on new conflicts (next rebase step).
    pub fn continue_operation(&self, op: Operation) -> anyhow::Result<bool> {
        let args: &[&str] = match op {
            Operation::Merge => &["commit", "--no-edit"],
            Operation::Rebase => &["rebase", "--continue"],
            Operation::CherryPick => &["cherry-pick", "--continue"],
            Operation::Revert => &["revert", "--continue"],
        };
        self.run_sequencer(args)
    }

    /// Drops the commit being applied and goes on.
    pub fn skip_operation(&self, op: Operation) -> anyhow::Result<bool> {
        let args: &[&str] = match op {
            Operation::Merge => anyhow::bail!("a merge can't be skipped"),
            Operation::Rebase => &["rebase", "--skip"],
            Operation::CherryPick => &["cherry-pick", "--skip"],
            Operation::Revert => &["revert", "--skip"],
        };
        self.run_sequencer(args)
    }

    /// Back to the state before the operation started. Without an operation
    /// (a stash pop that conflicted), resets conflicted paths to HEAD; the
    /// stash is still there.
    pub fn abort_operation(&self, op: Option<Operation>) -> anyhow::Result<()> {
        match op {
            Some(Operation::Merge) => self.git().run(["merge", "--abort"]),
            Some(Operation::Rebase) => self.git().run(["rebase", "--abort"]),
            Some(Operation::CherryPick) => self.git().run(["cherry-pick", "--abort"]),
            Some(Operation::Revert) => self.git().run(["revert", "--abort"]),
            None => self.git().run(["reset", "--merge"]),
        }
        .map(drop)
    }

    /// Runs a step that may stop on conflicts; no editor ever opens.
    fn run_sequencer(&self, args: &[&str]) -> anyhow::Result<bool> {
        let mut full = vec!["-c", "core.editor=true"];
        full.extend_from_slice(args);
        match self.git().run(&full) {
            Ok(_) => Ok(true),
            Err(_) if !self.conflicted_paths()?.is_empty() => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Merges `rev` into HEAD. `Ok(false)`: stopped on conflicts.
    pub fn merge(&self, rev: &str) -> anyhow::Result<bool> {
        self.run_sequencer(&["merge", "--no-edit", rev])
    }

    /// Rebases HEAD onto `rev`. `Ok(false)`: stopped on conflicts.
    pub fn rebase(&self, rev: &str) -> anyhow::Result<bool> {
        self.run_sequencer(&["rebase", rev])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.split_inclusive('\n').map(str::to_string).collect()
    }

    #[test]
    fn parses_diff3_output() {
        let m = "<".repeat(MARKER);
        let b = "|".repeat(MARKER);
        let s = "=".repeat(MARKER);
        let e = ">".repeat(MARKER);
        let text = format!(
            "a\nb\n{m} ours\nX\n{b} base\nB\n{s}\nY\nY2\n{e} theirs\nc\n{m} ours\n{b} base\nq\n{s}\nZ\n{e} theirs\n"
        );
        let chunks = parse_merge(&text);
        assert_eq!(
            chunks,
            vec![
                Chunk::Clean(lines("a\nb\n")),
                Chunk::Conflict(Conflict {
                    ours: lines("X\n"),
                    base: lines("B\n"),
                    theirs: lines("Y\nY2\n"),
                }),
                Chunk::Clean(lines("c\n")),
                Chunk::Conflict(Conflict {
                    ours: vec![],
                    base: lines("q\n"),
                    theirs: lines("Z\n"),
                }),
            ]
        );
    }

    #[test]
    fn renders_choices_and_keeps_markers_for_the_rest() {
        let file = ConflictFile {
            chunks: vec![
                Chunk::Clean(lines("a\n")),
                Chunk::Conflict(Conflict {
                    ours: lines("X\n"),
                    base: lines("B\n"),
                    theirs: lines("Y\n"),
                }),
                Chunk::Conflict(Conflict {
                    ours: lines("1\n"),
                    base: vec![],
                    theirs: lines("2\n"),
                }),
            ],
            ..Default::default()
        };
        assert_eq!(
            file.render(&[Some(Resolution::TheirsThenOurs), Some(Resolution::Ours)]),
            "a\nY\nX\n1\n"
        );
        assert_eq!(
            file.render(&[Some(Resolution::Neither), None]),
            "a\n<<<<<<< ours\n1\n||||||| base\n=======\n2\n>>>>>>> theirs\n"
        );
    }
}
