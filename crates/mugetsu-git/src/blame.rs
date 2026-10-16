//! One file's story: who last touched each line (`git blame`) and every
//! commit that changed it, across renames (`git log --follow`).

use std::collections::HashMap;

use gix::ObjectId;

use crate::Repo;

/// One line of a blamed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    /// Commit that last changed the line; all zeros for uncommitted lines.
    pub id: ObjectId,
    pub author: String,
    pub email: String,
    /// Author time, seconds since epoch.
    pub time: i64,
    pub summary: String,
    /// Line number in the blamed file, from 1.
    pub line: usize,
    pub text: String,
}

impl BlameLine {
    pub fn is_uncommitted(&self) -> bool {
        self.id.is_null()
    }
}

/// One commit that changed a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileCommit {
    pub id: ObjectId,
    pub author: String,
    pub email: String,
    /// Author time, seconds since epoch.
    pub time: i64,
    pub summary: String,
    /// The file's path in this commit.
    pub path: String,
    /// Its path before, when this commit renamed it.
    pub old_path: Option<String>,
    /// Status letter of the change (`A`, `M`, `D`, `R`…).
    pub status: char,
}

impl FileCommit {
    pub fn short_id(&self) -> String {
        self.id.to_hex_with_len(8).to_string()
    }
}

#[derive(Default)]
struct BlameInfo {
    author: String,
    email: String,
    time: i64,
    summary: String,
}

/// Parses `git blame --porcelain`.
pub fn parse_blame(out: &str) -> Vec<BlameLine> {
    let mut infos: HashMap<ObjectId, BlameInfo> = HashMap::new();
    let mut lines = Vec::new();
    let mut current: Option<(ObjectId, usize)> = None;
    for raw in out.lines() {
        if let Some(text) = raw.strip_prefix('\t') {
            if let Some((id, line)) = current.take() {
                let info = infos.get(&id);
                let field = |f: fn(&BlameInfo) -> &str| info.map(f).unwrap_or("").to_string();
                lines.push(BlameLine {
                    id,
                    author: field(|i| &i.author),
                    email: field(|i| &i.email),
                    time: info.map_or(0, |i| i.time),
                    summary: field(|i| &i.summary),
                    line,
                    text: text.to_string(),
                });
            }
            continue;
        }
        let mut parts = raw.split(' ');
        let head = parts.next().unwrap_or("");
        if head.len() == 40
            && let Ok(id) = ObjectId::from_hex(head.as_bytes())
        {
            let _orig = parts.next();
            let line = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            infos.entry(id).or_default();
            current = Some((id, line));
            continue;
        }
        let Some((id, _)) = current else { continue };
        let info = infos.entry(id).or_default();
        let rest = raw.split_once(' ').map_or("", |(_, r)| r);
        match head {
            "author" => info.author = rest.to_string(),
            "author-mail" => {
                info.email = rest
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .to_string()
            }
            "author-time" => info.time = rest.parse().unwrap_or(0),
            "summary" => info.summary = rest.to_string(),
            _ => {}
        }
    }
    lines
}

const FIELD: char = '\x1f';
const RECORD: char = '\x1e';

/// Parses `git log --name-status` with records `\x1e%H\x1f%an\x1f%ae\x1f%at\x1f%s`.
pub fn parse_file_log(out: &str) -> Vec<FileCommit> {
    out.split(RECORD)
        .filter_map(|record| {
            let mut lines = record.lines();
            let header = lines.next()?;
            let f: Vec<&str> = header.split(FIELD).collect();
            let [id, author, email, time, summary] = f[..] else {
                return None;
            };
            let id = ObjectId::from_hex(id.as_bytes()).ok()?;
            let change = lines.find(|l| !l.trim().is_empty()).unwrap_or("");
            let cols: Vec<&str> = change.split('\t').collect();
            let status = cols.first().and_then(|s| s.chars().next()).unwrap_or('M');
            let (path, old_path) = match cols[..] {
                [_, old, new] => (new.to_string(), Some(old.to_string())),
                [_, path] => (path.to_string(), None),
                _ => (String::new(), None),
            };
            Some(FileCommit {
                id,
                author: author.to_string(),
                email: email.to_string(),
                time: time.parse().unwrap_or(0),
                summary: summary.to_string(),
                path,
                old_path,
                status,
            })
        })
        .collect()
}

impl Repo {
    /// Who last changed each line of `path`: in the working copy when `rev`
    /// is `None` (uncommitted lines included), else as of commit `rev`.
    pub fn blame(&self, path: &str, rev: Option<&str>) -> anyhow::Result<Vec<BlameLine>> {
        let mut args = vec!["blame", "--porcelain"];
        args.extend(rev);
        args.extend(["--", path]);
        Ok(parse_blame(&self.git().run(args)?))
    }

    /// Commits that changed `path`, newest first, from `start` (HEAD when
    /// `None`), following it through renames back to the commit that created it.
    pub fn file_log(&self, path: &str, start: Option<&str>) -> anyhow::Result<Vec<FileCommit>> {
        let mut args = vec![
            "log",
            "--follow",
            "--find-renames",
            "--name-status",
            "--format=%x1e%H%x1f%an%x1f%ae%x1f%at%x1f%s",
        ];
        args.extend(start);
        args.extend(["--", path]);
        let out = self.git().run(args)?;
        Ok(parse_file_log(&out))
    }

    /// Patch of what `commit` did to the file (both names when it renamed it).
    pub fn file_patch(&self, commit: &FileCommit) -> anyhow::Result<String> {
        let rev = commit.id.to_string();
        let mut args = vec![
            "show",
            "--format=",
            "--no-ext-diff",
            "--patch",
            "--find-renames",
            "--first-parent",
            &rev,
            "--",
            &commit.path,
        ];
        args.extend(commit.old_path.as_deref());
        self.git().run(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_blame_with_repeated_commits() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let out = format!(
            "{a} 1 1 2\nauthor Ann\nauthor-mail <ann@x.io>\nauthor-time 100\nsummary first\nfilename f\n\tone\n\
             {a} 2 2\n\ttwo\n\
             {b} 3 3 1\nauthor Bob\nauthor-mail <bob@x.io>\nauthor-time 200\nsummary second\nfilename f\n\tthree\n"
        );
        let lines = parse_blame(&out);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].author, "Ann");
        assert_eq!(lines[1].text, "two");
        assert_eq!(lines[1].line, 2);
        assert_eq!(lines[2].email, "bob@x.io");
        assert_eq!(lines[2].summary, "second");
        assert_eq!(lines[2].time, 200);
    }

    #[test]
    fn parses_log_with_a_rename() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let out = format!(
            "\x1e{a}\x1fAnn\x1fann@x.io\x1f200\x1fmove\n\nR100\told.txt\tnew.txt\n\
             \x1e{b}\x1fBob\x1fbob@x.io\x1f100\x1fcreate\n\nA\told.txt\n"
        );
        let log = parse_file_log(&out);
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].status, 'R');
        assert_eq!(log[0].path, "new.txt");
        assert_eq!(log[0].old_path.as_deref(), Some("old.txt"));
        assert_eq!(log[1].status, 'A');
        assert_eq!(log[1].path, "old.txt");
        assert_eq!(log[1].old_path, None);
    }
}
