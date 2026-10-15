use crate::Repo;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    /// "\ No newline at end of file"
    NoNewline,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// Raw `@@ -a,b +c,d @@ ctx` line.
    pub header: String,
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffFile {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    /// Header lines (`diff --git`, mode lines, `---`, `+++`), kept to rebuild patches.
    pub header: Vec<String>,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

impl DiffFile {
    pub fn path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or("")
    }

    pub fn stats(&self) -> (usize, usize) {
        let mut add = 0;
        let mut del = 0;
        for l in self.hunks.iter().flat_map(|h| &h.lines) {
            match l.kind {
                LineKind::Added => add += 1,
                LineKind::Removed => del += 1,
                _ => {}
            }
        }
        (add, del)
    }

    /// Builds a patch containing only hunk `index`, suitable for `git apply`.
    pub fn hunk_patch(&self, index: usize) -> Option<String> {
        let hunk = self.hunks.get(index)?;
        let mut out = String::new();
        for h in &self.header {
            out.push_str(h);
            out.push('\n');
        }
        out.push_str(&hunk.header);
        out.push('\n');
        for line in &hunk.lines {
            let prefix = match line.kind {
                LineKind::Context => ' ',
                LineKind::Added => '+',
                LineKind::Removed => '-',
                LineKind::NoNewline => '\\',
            };
            out.push(prefix);
            out.push_str(&line.text);
            out.push('\n');
        }
        Some(out)
    }
}

/// A parsed multi-file unified diff.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub files: Vec<DiffFile>,
}

fn strip_path(p: &str) -> Option<String> {
    if p == "/dev/null" {
        return None;
    }
    Some(
        p.strip_prefix("a/")
            .or_else(|| p.strip_prefix("b/"))
            .unwrap_or(p)
            .to_string(),
    )
}

fn parse_range(s: &str) -> (u32, u32) {
    let mut it = s.splitn(2, ',');
    let start = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let len = it.next().map_or(1, |v| v.parse().unwrap_or(0));
    (start, len)
}

impl FileDiff {
    pub fn parse(raw: &str) -> Self {
        let mut files: Vec<DiffFile> = Vec::new();
        let mut old_no = 0;
        let mut new_no = 0;
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("diff --git ") {
                let mut file = DiffFile {
                    header: vec![line.to_string()],
                    ..Default::default()
                };
                // Best effort until ---/+++ give the exact paths (needed for binary files).
                if let Some((a, b)) = rest.split_once(" b/") {
                    file.old_path = strip_path(a);
                    file.new_path = Some(b.to_string());
                }
                files.push(file);
                continue;
            }
            let Some(file) = files.last_mut() else {
                continue;
            };
            if let Some(h) = line.strip_prefix("@@ ") {
                let mut parts = h.split(' ');
                let (old_start, old_len) =
                    parse_range(parts.next().unwrap_or("-0").trim_start_matches('-'));
                let (new_start, new_len) =
                    parse_range(parts.next().unwrap_or("+0").trim_start_matches('+'));
                old_no = old_start;
                new_no = new_start;
                file.hunks.push(Hunk {
                    header: line.to_string(),
                    old_start,
                    old_len,
                    new_start,
                    new_len,
                    lines: Vec::new(),
                });
                continue;
            }
            match file.hunks.last_mut() {
                None => {
                    if let Some(p) = line.strip_prefix("--- ") {
                        file.old_path = strip_path(p);
                    } else if let Some(p) = line.strip_prefix("+++ ") {
                        file.new_path = strip_path(p);
                    } else if line.starts_with("Binary files ") {
                        file.binary = true;
                    } else if line.starts_with("new file mode") {
                        file.old_path = None;
                    } else if line.starts_with("deleted file mode") {
                        file.new_path = None;
                    } else if let Some(p) = line.strip_prefix("rename from ") {
                        file.old_path = Some(p.to_string());
                    } else if let Some(p) = line.strip_prefix("rename to ") {
                        file.new_path = Some(p.to_string());
                    }
                    file.header.push(line.to_string());
                }
                Some(hunk) => {
                    let (kind, text) = match line.as_bytes().first() {
                        Some(b'+') => (LineKind::Added, &line[1..]),
                        Some(b'-') => (LineKind::Removed, &line[1..]),
                        Some(b'\\') => (LineKind::NoNewline, &line[1..]),
                        Some(b' ') => (LineKind::Context, &line[1..]),
                        None => (LineKind::Context, ""),
                        _ => continue,
                    };
                    let (o, n) = match kind {
                        LineKind::Context => {
                            old_no += 1;
                            new_no += 1;
                            (Some(old_no - 1), Some(new_no - 1))
                        }
                        LineKind::Added => {
                            new_no += 1;
                            (None, Some(new_no - 1))
                        }
                        LineKind::Removed => {
                            old_no += 1;
                            (Some(old_no - 1), None)
                        }
                        LineKind::NoNewline => (None, None),
                    };
                    hunk.lines.push(DiffLine {
                        kind,
                        old_no: o,
                        new_no: n,
                        text: text.to_string(),
                    });
                }
            }
        }
        Self { files }
    }
}

const DIFF_ARGS: [&str; 3] = ["--no-ext-diff", "--patch", "--find-renames"];

impl Repo {
    /// Diff of unstaged (worktree vs index) or staged (index vs HEAD) changes.
    pub fn diff_worktree(&self, staged: bool, path: Option<&str>) -> anyhow::Result<FileDiff> {
        let mut args = vec!["diff"];
        args.extend(DIFF_ARGS);
        if staged {
            args.push("--cached");
        }
        if let Some(p) = path {
            args.extend(["--", p]);
        }
        Ok(FileDiff::parse(&self.git().run(args)?))
    }

    /// Diff for an untracked file, shown as a full addition.
    pub fn diff_untracked(&self, path: &str) -> anyhow::Result<FileDiff> {
        // `git diff --no-index` exits 1 when files differ, so read stdout regardless.
        let out = self.git().run_lenient([
            "diff",
            "--no-ext-diff",
            "--no-index",
            "--",
            "/dev/null",
            path,
        ])?;
        Ok(FileDiff::parse(&out))
    }

    /// Diff introduced by a commit (against its first parent).
    pub fn diff_commit(&self, rev: &str) -> anyhow::Result<FileDiff> {
        let mut args = vec!["show", "--format="];
        args.extend(DIFF_ARGS);
        args.extend(["--first-parent", rev]);
        Ok(FileDiff::parse(&self.git().run(args)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "diff --git a/src/lib.rs b/src/lib.rs
index 111..222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@ fn main()
 a
-b
+B
+c
 d
@@ -10 +11 @@
-x
\\ No newline at end of file
+y
diff --git a/new.txt b/new.txt
new file mode 100644
index 000..333
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+hello
diff --git a/img.png b/img.png
index 1..2 100644
Binary files a/img.png and b/img.png differ
";

    #[test]
    fn parses_files_hunks_and_line_numbers() {
        let d = FileDiff::parse(SAMPLE);
        assert_eq!(d.files.len(), 3);
        let f = &d.files[0];
        assert_eq!(f.path(), "src/lib.rs");
        assert_eq!(f.hunks.len(), 2);
        let h = &f.hunks[0];
        assert_eq!(
            (h.old_start, h.old_len, h.new_start, h.new_len),
            (1, 3, 1, 4)
        );
        assert_eq!(
            h.lines[1],
            DiffLine {
                kind: LineKind::Removed,
                old_no: Some(2),
                new_no: None,
                text: "b".into()
            }
        );
        assert_eq!(h.lines[3].new_no, Some(3));
        assert_eq!(
            h.lines[4],
            DiffLine {
                kind: LineKind::Context,
                old_no: Some(3),
                new_no: Some(4),
                text: "d".into()
            }
        );
        assert_eq!(f.hunks[1].old_len, 1);
        assert_eq!(f.hunks[1].lines[1].kind, LineKind::NoNewline);
        assert_eq!(f.stats(), (3, 2));

        assert_eq!(d.files[1].old_path, None);
        assert_eq!(d.files[1].new_path.as_deref(), Some("new.txt"));
        assert!(d.files[2].binary);
        assert_eq!(d.files[2].path(), "img.png");
    }

    #[test]
    fn hunk_patch_round_trips() {
        let d = FileDiff::parse(SAMPLE);
        let patch = d.files[0].hunk_patch(1).unwrap();
        assert!(patch.starts_with("diff --git a/src/lib.rs b/src/lib.rs\n"));
        assert!(
            patch.contains(
                "+++ b/src/lib.rs\n@@ -10 +11 @@\n-x\n\\ No newline at end of file\n+y\n"
            )
        );
        assert!(!patch.contains("+B"));
    }
}
