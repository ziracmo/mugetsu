//! Repository statistics read from commits only (no forge API, so no rate
//! limit): who writes, who commits, who reviews (trailers), when, and which
//! files move. Metadata comes first in one cheap `git log`; line counts follow
//! from a streamed `--numstat` pass, reported as it goes.

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::Repo;

const DAY: i64 = 86_400;
/// At most this many bars in the activity chart.
const MAX_BUCKETS: u64 = 60;
/// Committer of GitHub's web merges / squashes: not a person.
const WEB_FLOW: &str = "noreply@github.com";

/// One merged identity (mailmap, same email, same name or same GitHub login).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub email: String,
    pub commits: u32,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileHeat {
    pub path: String,
    /// Commits touching the file.
    pub commits: u32,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoStats {
    /// Branch the stats were computed on (`origin/main`, …).
    pub branch: String,
    pub commits: u32,
    pub merges: u32,
    /// Sorted by commits, most active first (same for every list).
    pub authors: Vec<Person>,
    /// Who applied the commits, when it differs from the author for someone.
    pub committers: Vec<Person>,
    /// `Reviewed-by` / `Approved-by` / `Acked-by` / `Tested-by` trailers.
    pub reviewers: Vec<Person>,
    /// `Co-authored-by` trailers.
    pub coauthors: Vec<Person>,
    /// Authors of merge commits (who lands pull requests).
    pub mergers: Vec<Person>,
    /// Commits per bucket of `bucket_days`, oldest first, ending today.
    pub activity: Vec<u32>,
    pub bucket_days: u32,
    /// Commits by weekday (Monday first) and hour, in the author's time zone.
    pub punchcard: [[u32; 24]; 7],
    /// Most touched files first (filled by the line pass).
    pub files: Vec<FileHeat>,
    pub additions: u64,
    pub deletions: u64,
    /// Commits whose line counts are in, out of `commits` minus merges.
    pub lines_scanned: u32,
    pub lines_done: bool,
}

impl RepoStats {
    /// Commits the line pass looks at (merges carry no numstat).
    pub fn lines_total(&self) -> u32 {
        self.commits - self.merges
    }
}

impl Repo {
    /// The branch stats are computed on: the remote's default branch (kept
    /// fresh by the periodic fetch), else a local main / master, else HEAD.
    pub fn default_branch(&self) -> String {
        let git = self.git();
        if let Ok(head) = git.run(["symbolic-ref", "-q", "refs/remotes/origin/HEAD"])
            && let Some(name) = head.trim().strip_prefix("refs/remotes/")
        {
            return name.to_string();
        }
        ["origin/main", "origin/master", "main", "master"]
            .into_iter()
            .find(|r| {
                git.run(["rev-parse", "--verify", "-q", &format!("{r}^{{commit}}")])
                    .is_ok()
            })
            .unwrap_or("HEAD")
            .to_string()
    }

    /// Stats for the commits of `branch` newer than `since` (unix seconds;
    /// `None` for the whole history). `progress` gets a snapshot once metadata
    /// is read, then while line counts stream in. `cancel` stops early.
    pub fn stats(
        &self,
        branch: &str,
        since: Option<i64>,
        now: i64,
        cancel: &AtomicBool,
        mut progress: impl FnMut(RepoStats),
    ) -> anyhow::Result<RepoStats> {
        let since_arg = since.map(|s| format!("--since=@{s}"));
        let mut args: Vec<&str> = vec![
            "log",
            branch,
            "--no-show-signature",
            "--date=format:%u %H",
            META_FORMAT,
        ];
        args.extend(since_arg.as_deref());
        let raw = self.git().run(&args)?;
        let mut acc = Acc::new(branch, parse_meta(&raw), since, now);
        progress(acc.snapshot());

        let mut args: Vec<&str> = vec![
            "log",
            branch,
            "--no-show-signature",
            "--no-renames",
            "--numstat",
            "--format=%x1e%H",
        ];
        args.extend(since_arg.as_deref());
        let mut child = self.git().spawn(&args)?;
        let stdout = child.stdout.take().expect("piped stdout");
        let mut current: Option<usize> = None;
        let mut last = Instant::now();
        for line in BufReader::new(stdout).split(b'\n') {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(acc.snapshot());
            }
            let line = line?;
            let line = String::from_utf8_lossy(&line);
            if let Some(id) = line.strip_prefix('\x1e') {
                current = acc.index.get(id.trim()).copied();
                if current.is_some() {
                    acc.scanned += 1;
                }
            } else if let (Some(i), Some((add, del, path))) = (current, parse_numstat(&line)) {
                acc.add_lines(i, add, del, path);
            }
            if last.elapsed() > Duration::from_millis(120) {
                last = Instant::now();
                progress(acc.snapshot());
            }
        }
        let _ = child.wait();
        acc.done = true;
        Ok(acc.snapshot())
    }
}

impl Repo {
    /// Commits per local day over the last `days` days (oldest first, today
    /// last), across every local and remote branch: the sidebar's pulse.
    /// `utc_offset` is the local offset in seconds, so days break at midnight.
    pub fn daily_commits(
        &self,
        days: usize,
        now: i64,
        utc_offset: i64,
    ) -> anyhow::Result<Vec<u32>> {
        let since = format!("--since=@{}", now - days as i64 * DAY);
        let raw = self
            .git()
            .run(["log", "--branches", "--remotes", "--format=%at", &since])?;
        let times = raw.lines().filter_map(|l| l.trim().parse().ok());
        Ok(bucket_days(times, days, now, utc_offset))
    }
}

/// Counts `times` (unix seconds) per local calendar day, the last bucket being today.
fn bucket_days(
    times: impl Iterator<Item = i64>,
    days: usize,
    now: i64,
    utc_offset: i64,
) -> Vec<u32> {
    let today = (now + utc_offset).div_euclid(DAY);
    let mut out = vec![0; days];
    for t in times {
        let ago = today - (t + utc_offset).div_euclid(DAY);
        if (0..days as i64).contains(&ago) {
            out[days - 1 - ago as usize] += 1;
        }
    }
    out
}

/// `%x1e` starts a commit, `%x1f` separates fields, `%x1d` separates trailers.
const META_FORMAT: &str = "--format=%x1e%H%x1f%P%x1f%aN%x1f%aE%x1f%cN%x1f%cE%x1f%at%x1f%ad%x1f%(trailers:only,unfold,separator=%x1d)";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Meta {
    id: String,
    merge: bool,
    author: (String, String),
    committer: (String, String),
    time: i64,
    weekday: usize,
    hour: usize,
    reviewers: Vec<(String, String)>,
    coauthors: Vec<(String, String)>,
}

fn parse_meta(raw: &str) -> Vec<Meta> {
    raw.split('\x1e')
        .filter_map(|rec| {
            let f: Vec<&str> = rec.trim_end_matches('\n').split('\x1f').collect();
            if f.len() < 9 {
                return None;
            }
            let (weekday, hour) = f[7].split_once(' ').unwrap_or(("1", "0"));
            let mut m = Meta {
                id: f[0].to_string(),
                merge: f[1].split_whitespace().count() > 1,
                author: (f[2].to_string(), f[3].to_string()),
                committer: (f[4].to_string(), f[5].to_string()),
                time: f[6].parse().unwrap_or(0),
                weekday: weekday.parse::<usize>().unwrap_or(1).clamp(1, 7) - 1,
                hour: hour.parse::<usize>().unwrap_or(0).min(23),
                ..Meta::default()
            };
            for trailer in f[8].split('\x1d') {
                let Some((key, value)) = trailer.split_once(':') else {
                    continue;
                };
                let who = parse_ident(value);
                match key.trim().to_ascii_lowercase().as_str() {
                    "reviewed-by" | "approved-by" | "acked-by" | "tested-by" => {
                        m.reviewers.push(who)
                    }
                    "co-authored-by" => m.coauthors.push(who),
                    _ => {}
                }
            }
            Some(m)
        })
        .collect()
}

/// `Jane Doe <jane@x.org>` → (name, email); either may be empty.
fn parse_ident(s: &str) -> (String, String) {
    let s = s.trim();
    match s.split_once('<') {
        Some((name, rest)) => (
            name.trim().to_string(),
            rest.trim_end_matches('>').trim().to_string(),
        ),
        None if s.contains('@') => (String::new(), s.to_string()),
        None => (s.to_string(), String::new()),
    }
}

/// `12\t3\tpath` (binary files show `-`).
fn parse_numstat(line: &str) -> Option<(u64, u64, &str)> {
    let mut parts = line.splitn(3, '\t');
    let add = parts.next()?;
    let del = parts.next()?;
    let path = parts.next()?;
    Some((add.parse().unwrap_or(0), del.parse().unwrap_or(0), path))
}

/// Keys that make two (name, email) pairs the same person.
fn identity_keys(name: &str, email: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let email = email.trim().to_lowercase();
    if !email.is_empty() && email != WEB_FLOW {
        // 12345+login@users.noreply.github.com and login@users.noreply…
        if let Some(local) = email.strip_suffix("@users.noreply.github.com") {
            let login = local.split_once('+').map_or(local, |(_, l)| l);
            keys.push(format!("gh:{login}"));
        }
        keys.push(format!("e:{email}"));
    }
    let name: String = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if name.chars().count() >= 3 {
        keys.push(format!("n:{name}"));
        // A bare login as a name (e.g. `ziracmo`) matches its noreply address.
        if !name.contains(' ') {
            keys.push(format!("gh:{name}"));
        }
    }
    keys
}

/// Union-find over interned (name, email) pairs.
#[derive(Default)]
struct Identities {
    pairs: Vec<(String, String)>,
    pair_index: HashMap<(String, String), usize>,
    parent: Vec<usize>,
    by_key: HashMap<String, usize>,
}

impl Identities {
    fn intern(&mut self, pair: &(String, String)) -> usize {
        if let Some(&i) = self.pair_index.get(pair) {
            return i;
        }
        let i = self.pairs.len();
        self.pairs.push(pair.clone());
        self.pair_index.insert(pair.clone(), i);
        self.parent.push(i);
        for key in identity_keys(&pair.0, &pair.1) {
            match self.by_key.get(&key) {
                Some(&other) => self.union(i, other),
                None => {
                    self.by_key.insert(key, i);
                }
            }
        }
        i
    }

    fn find(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.parent[a.max(b)] = a.min(b);
        }
    }
}

struct Commit {
    author: usize,
    committer: usize,
    merge: bool,
    time: i64,
    weekday: usize,
    hour: usize,
    reviewers: Vec<usize>,
    coauthors: Vec<usize>,
    additions: u64,
    deletions: u64,
}

struct Acc {
    branch: String,
    ids: Identities,
    commits: Vec<Commit>,
    index: HashMap<String, usize>,
    files: HashMap<String, FileHeat>,
    start: i64,
    now: i64,
    scanned: u32,
    done: bool,
}

impl Acc {
    fn new(branch: &str, metas: Vec<Meta>, since: Option<i64>, now: i64) -> Self {
        let mut ids = Identities::default();
        let mut index = HashMap::new();
        let mut commits = Vec::with_capacity(metas.len());
        for m in metas {
            index.insert(m.id.clone(), commits.len());
            commits.push(Commit {
                author: ids.intern(&m.author),
                committer: ids.intern(&m.committer),
                merge: m.merge,
                time: m.time,
                weekday: m.weekday,
                hour: m.hour,
                reviewers: m.reviewers.iter().map(|p| ids.intern(p)).collect(),
                coauthors: m.coauthors.iter().map(|p| ids.intern(p)).collect(),
                additions: 0,
                deletions: 0,
            });
        }
        let oldest = commits.iter().map(|c| c.time).min().unwrap_or(now);
        Self {
            branch: branch.to_string(),
            ids,
            commits,
            index,
            files: HashMap::new(),
            start: since.unwrap_or(oldest).min(now),
            now,
            scanned: 0,
            done: false,
        }
    }

    fn add_lines(&mut self, commit: usize, add: u64, del: u64, path: &str) {
        let c = &mut self.commits[commit];
        c.additions += add;
        c.deletions += del;
        let heat = self
            .files
            .entry(path.to_string())
            .or_insert_with(|| FileHeat {
                path: path.to_string(),
                ..FileHeat::default()
            });
        heat.commits += 1;
        heat.additions += add;
        heat.deletions += del;
    }

    fn snapshot(&mut self) -> RepoStats {
        let roots: Vec<usize> = (0..self.ids.pairs.len())
            .map(|i| self.ids.find(i))
            .collect();
        let mut tally = Tally::new(&self.ids.pairs, &roots);
        let span_days = ((self.now - self.start) / DAY + 1).max(1) as u64;
        let bucket_days = span_days.div_ceil(MAX_BUCKETS).max(1);
        let buckets = span_days.div_ceil(bucket_days) as usize;
        let mut stats = RepoStats {
            branch: self.branch.clone(),
            activity: vec![0; buckets],
            bucket_days: bucket_days as u32,
            lines_scanned: self.scanned,
            lines_done: self.done,
            ..RepoStats::default()
        };
        let committed_by_other = self
            .commits
            .iter()
            .any(|c| roots[c.author] != roots[c.committer]);
        for c in &self.commits {
            stats.commits += 1;
            stats.additions += c.additions;
            stats.deletions += c.deletions;
            stats.punchcard[c.weekday][c.hour] += 1;
            let age = ((self.now - c.time).max(0) as u64 / DAY as u64 / bucket_days) as usize;
            if age < buckets {
                stats.activity[buckets - 1 - age] += 1;
            }
            if c.merge {
                stats.merges += 1;
                tally.add(Role::Merger, c.author, 0, 0);
            } else {
                tally.add(Role::Author, c.author, c.additions, c.deletions);
            }
            if committed_by_other && !self.ids.pairs[c.committer].1.eq_ignore_ascii_case(WEB_FLOW) {
                tally.add(Role::Committer, c.committer, 0, 0);
            }
            for &r in &c.reviewers {
                tally.add(Role::Reviewer, r, 0, 0);
            }
            for &r in &c.coauthors {
                tally.add(Role::Coauthor, r, 0, 0);
            }
        }
        [
            stats.authors,
            stats.mergers,
            stats.committers,
            stats.reviewers,
            stats.coauthors,
        ] = tally.finish();
        let mut files: Vec<FileHeat> = self.files.values().cloned().collect();
        files.sort_by(|a, b| {
            (b.commits, b.additions + b.deletions, &a.path).cmp(&(
                a.commits,
                a.additions + a.deletions,
                &b.path,
            ))
        });
        files.truncate(50);
        stats.files = files;
        stats
    }
}

#[derive(Clone, Copy)]
enum Role {
    Author,
    Merger,
    Committer,
    Reviewer,
    Coauthor,
}

/// A merged identity's counts, and how often each (name, email) pair was seen.
type Entry = (Person, HashMap<usize, u32>);

/// Per role, per merged identity: counts and the name / email variants seen.
struct Tally<'a> {
    pairs: &'a [(String, String)],
    roots: &'a [usize],
    roles: [HashMap<usize, Entry>; 5],
}

impl<'a> Tally<'a> {
    fn new(pairs: &'a [(String, String)], roots: &'a [usize]) -> Self {
        Self {
            pairs,
            roots,
            roles: Default::default(),
        }
    }

    fn add(&mut self, role: Role, pair: usize, add: u64, del: u64) {
        let (p, variants) = self.roles[role as usize]
            .entry(self.roots[pair])
            .or_default();
        p.commits += 1;
        p.additions += add;
        p.deletions += del;
        *variants.entry(pair).or_default() += 1;
    }

    fn finish(self) -> [Vec<Person>; 5] {
        let pairs = self.pairs;
        self.roles.map(|role| {
            let mut people: Vec<Person> = role
                .into_values()
                .map(|(mut p, variants)| {
                    // Most used spelling, preferring a full name over a login.
                    let pick = |field: fn(&(String, String)) -> &String| {
                        variants
                            .iter()
                            .filter(|(i, _)| !field(&pairs[**i]).is_empty())
                            .max_by_key(|(i, n)| {
                                let s = field(&pairs[**i]);
                                (s.contains(' '), **n, std::cmp::Reverse(s.clone()))
                            })
                            .map(|(i, _)| field(&pairs[*i]).clone())
                            .unwrap_or_default()
                    };
                    p.name = pick(|p| &p.0);
                    p.email = pick(|p| &p.1);
                    if p.name.is_empty() {
                        p.name = p.email.clone();
                    }
                    p
                })
                .collect();
            people.sort_by(|a, b| {
                (b.commits, b.additions + b.deletions, &a.name).cmp(&(
                    a.commits,
                    a.additions + a.deletions,
                    &b.name,
                ))
            });
            people
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, author: (&str, &str), time: i64) -> Meta {
        Meta {
            id: id.into(),
            author: (author.0.into(), author.1.into()),
            committer: (author.0.into(), author.1.into()),
            time,
            ..Meta::default()
        }
    }

    #[test]
    fn parses_log_records_and_trailers() {
        let raw = "\x1eabc\x1fp1 p2\x1fJane\x1fjane@x.org\x1fGitHub\x1fnoreply@github.com\x1f1700000000\x1f3 14\x1fReviewed-by: Bob <bob@x.org>\x1dCo-authored-by: Al <al@x.org>\x1dSigned-off-by: Jane <jane@x.org>\n";
        let m = &parse_meta(raw)[0];
        assert!(m.merge);
        assert_eq!((m.weekday, m.hour), (2, 14));
        assert_eq!(m.reviewers, [("Bob".into(), "bob@x.org".into())]);
        assert_eq!(m.coauthors, [("Al".into(), "al@x.org".into())]);
        assert_eq!(parse_numstat("-\t-\timg.png"), Some((0, 0, "img.png")));
        assert_eq!(parse_numstat("3\t1\tsrc/a.rs"), Some((3, 1, "src/a.rs")));
    }

    #[test]
    fn merges_identities_by_email_name_and_github_login() {
        let now = 100 * DAY;
        let metas = vec![
            meta("1", ("Alexandre", "alex@work.com"), now),
            meta(
                "2",
                ("alexandre", "12600085+ziracmo@users.noreply.github.com"),
                now,
            ),
            meta("3", ("ziracmo", "other@home.net"), now),
            meta("4", ("Bob", "bob@x.org"), now),
        ];
        let stats = Acc::new("main", metas, Some(now - 30 * DAY), now).snapshot();
        assert_eq!(stats.authors.len(), 2);
        assert_eq!(stats.authors[0].commits, 3);
        assert_eq!(stats.authors[0].name, "Alexandre");
        // Nobody committed someone else's work: no committer list.
        assert!(stats.committers.is_empty());
    }

    #[test]
    fn buckets_activity_ending_today() {
        let now = 100 * DAY;
        let metas = vec![
            meta("1", ("A", "a@x"), now),
            meta("2", ("A", "a@x"), now - 29 * DAY),
            meta("3", ("A", "a@x"), now - DAY / 2),
        ];
        let stats = Acc::new("main", metas, Some(now - 30 * DAY), now).snapshot();
        assert_eq!(stats.bucket_days, 1);
        assert_eq!(stats.activity.len(), 31);
        assert_eq!(*stats.activity.last().unwrap(), 2);
        assert_eq!(stats.activity[1], 1);
        // A year: weekly bars.
        let stats = Acc::new("main", vec![], Some(now - 365 * DAY), now).snapshot();
        assert_eq!(stats.bucket_days, 7);
    }

    #[test]
    fn buckets_days_at_local_midnight() {
        let now = 100 * DAY + 3600; // 01:00 UTC
        let times = [now, now - 2 * 3600, now - 29 * DAY, now - 40 * DAY];
        // UTC: the commit at 23:00 the day before lands on yesterday.
        let utc = bucket_days(times.into_iter(), 30, now, 0);
        assert_eq!((utc[29], utc[28], utc[0]), (1, 1, 1));
        // UTC+2: both are today (03:00 and 01:00).
        let east = bucket_days(times.into_iter(), 30, now, 7200);
        assert_eq!((east[29], east[28]), (2, 0));
        assert_eq!(east.iter().sum::<u32>(), 3);
    }
}
