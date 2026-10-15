use crate::Repo;

/// How a path changed on one side (index or worktree).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Unmodified,
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflicted,
}

impl Change {
    fn from_code(c: u8) -> Self {
        match c {
            b'M' => Self::Modified,
            b'A' => Self::Added,
            b'D' => Self::Deleted,
            b'R' => Self::Renamed,
            b'C' => Self::Copied,
            b'T' => Self::TypeChanged,
            _ => Self::Unmodified,
        }
    }

    pub fn letter(self) -> &'static str {
        match self {
            Self::Unmodified => " ",
            Self::Modified => "M",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Copied => "C",
            Self::TypeChanged => "T",
            Self::Untracked => "?",
            Self::Conflicted => "!",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStatus {
    pub path: String,
    pub orig_path: Option<String>,
    pub index: Change,
    pub worktree: Change,
}

impl FileStatus {
    pub fn is_staged(&self) -> bool {
        !matches!(
            self.index,
            Change::Unmodified | Change::Untracked | Change::Conflicted
        )
    }

    pub fn is_unstaged(&self) -> bool {
        self.worktree != Change::Unmodified
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub branch: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<FileStatus>,
}

impl Status {
    pub fn staged(&self) -> impl Iterator<Item = &FileStatus> {
        self.files.iter().filter(|f| f.is_staged())
    }

    pub fn unstaged(&self) -> impl Iterator<Item = &FileStatus> {
        self.files.iter().filter(|f| f.is_unstaged())
    }

    /// Parses `git status --porcelain=v2 --branch -z` output.
    pub fn parse(raw: &str) -> Self {
        let mut status = Status::default();
        let mut records = raw.split('\0');
        while let Some(rec) = records.next() {
            if rec.is_empty() {
                continue;
            }
            let bytes = rec.as_bytes();
            match bytes[0] {
                b'#' => {
                    if let Some(head) = rec.strip_prefix("# branch.head ") {
                        status.branch = (head != "(detached)").then(|| head.to_string());
                    } else if let Some(ab) = rec.strip_prefix("# branch.ab ") {
                        let mut parts = ab.split(' ');
                        status.ahead = parts
                            .next()
                            .and_then(|a| a.trim_start_matches('+').parse().ok())
                            .unwrap_or(0);
                        status.behind = parts
                            .next()
                            .and_then(|b| b.trim_start_matches('-').parse().ok())
                            .unwrap_or(0);
                    }
                }
                b'1' => {
                    let fields: Vec<&str> = rec.splitn(9, ' ').collect();
                    if fields.len() == 9 {
                        status.files.push(entry(fields[1], fields[8], None));
                    }
                }
                b'2' => {
                    let fields: Vec<&str> = rec.splitn(10, ' ').collect();
                    let orig = records.next().map(str::to_string);
                    if fields.len() == 10 {
                        status.files.push(entry(fields[1], fields[9], orig));
                    }
                }
                b'u' => {
                    let fields: Vec<&str> = rec.splitn(11, ' ').collect();
                    if fields.len() == 11 {
                        status.files.push(FileStatus {
                            path: fields[10].to_string(),
                            orig_path: None,
                            index: Change::Conflicted,
                            worktree: Change::Conflicted,
                        });
                    }
                }
                b'?' => status.files.push(FileStatus {
                    path: rec[2..].to_string(),
                    orig_path: None,
                    index: Change::Untracked,
                    worktree: Change::Untracked,
                }),
                _ => {}
            }
        }
        status
    }
}

fn entry(xy: &str, path: &str, orig_path: Option<String>) -> FileStatus {
    let xy = xy.as_bytes();
    FileStatus {
        path: path.to_string(),
        orig_path,
        index: Change::from_code(xy[0]),
        worktree: Change::from_code(xy[1]),
    }
}

impl Repo {
    pub fn status(&self) -> anyhow::Result<Status> {
        let raw = self.git().run([
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ])?;
        Ok(Status::parse(&raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v2() {
        let raw = "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -3\0\
1 .M N... 100644 100644 100644 aaa bbb src/a b.rs\0\
1 A. N... 000000 100644 100644 000 ccc new.rs\0\
2 R. N... 100644 100644 100644 ddd ddd R100 renamed.rs\0old.rs\0\
u UU N... 100644 100644 100644 100644 e1 e2 e3 conflict.rs\0\
? untracked.txt\0";
        let s = Status::parse(raw);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!((s.ahead, s.behind), (2, 3));
        assert_eq!(s.files.len(), 5);
        assert_eq!(s.files[0].path, "src/a b.rs");
        assert!(s.files[0].is_unstaged() && !s.files[0].is_staged());
        assert!(s.files[1].is_staged());
        assert_eq!(s.files[2].orig_path.as_deref(), Some("old.rs"));
        assert_eq!(s.files[2].index, Change::Renamed);
        assert_eq!(s.files[3].index, Change::Conflicted);
        assert_eq!(s.files[4].worktree, Change::Untracked);
        assert_eq!(s.staged().count(), 2);
        assert_eq!(s.unstaged().count(), 3);
    }

    #[test]
    fn detached_head_has_no_branch() {
        let s = Status::parse("# branch.head (detached)\0");
        assert_eq!(s.branch, None);
    }
}
