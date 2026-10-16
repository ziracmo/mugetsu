//! Weekly recap: what was committed over a week, on local and remote
//! branches, for one person or the whole team. Commits only (no forge API).

use std::collections::{HashMap, HashSet};

use crate::Repo;
use crate::stats::{Identities, Person, identity_keys, parse_numstat};

const DAY: i64 = 86_400;
/// Hot files kept per week.
const HOT_FILES: usize = 5;

/// One non-merge commit, with its line counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    pub id: String,
    pub name: String,
    pub email: String,
    /// Author time, unix seconds.
    pub time: i64,
    pub additions: u64,
    pub deletions: u64,
    pub files: Vec<String>,
}

impl Repo {
    /// Non-merge commits of the local and remote branches committed since
    /// `since` (unix seconds), with their line counts.
    pub fn activity(&self, since: i64) -> anyhow::Result<Vec<Activity>> {
        let since = format!("--since=@{since}");
        let raw = self.git().run([
            "log",
            "--branches",
            "--remotes",
            "--no-merges",
            "--no-show-signature",
            "--no-renames",
            "--numstat",
            "--format=%x1e%H%x1f%aN%x1f%aE%x1f%at",
            &since,
        ])?;
        Ok(parse_activity(&raw))
    }
}

fn parse_activity(raw: &str) -> Vec<Activity> {
    raw.split('\x1e')
        .filter_map(|record| {
            let mut lines = record.lines();
            let mut head = lines.next()?.split('\x1f');
            let mut commit = Activity {
                id: head.next()?.to_string(),
                name: head.next()?.to_string(),
                email: head.next()?.to_string(),
                time: head.next()?.trim().parse().ok()?,
                ..Activity::default()
            };
            for (add, del, path) in lines.filter_map(parse_numstat) {
                commit.additions += add;
                commit.deletions += del;
                commit.files.push(path.to_string());
            }
            Some(commit)
        })
        .collect()
}

/// Who "me" is: every identity the user commits with.
#[derive(Clone, Debug, Default)]
pub struct Me {
    keys: HashSet<String>,
}

impl Me {
    pub fn new<'a>(identities: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        Self {
            keys: identities
                .into_iter()
                .flat_map(|(name, email)| identity_keys(name, email))
                .collect(),
        }
    }

    pub fn owns(&self, name: &str, email: &str) -> bool {
        identity_keys(name, email)
            .iter()
            .any(|k| self.keys.contains(k))
    }
}

/// `[start, end)`, unix seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimeSpan {
    pub start: i64,
    pub end: i64,
}

impl TimeSpan {
    fn contains(self, t: i64) -> bool {
        (self.start..self.end).contains(&t)
    }
}

/// What a week added up to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Week {
    pub commits: u32,
    pub additions: u64,
    pub deletions: u64,
    /// Distinct files touched, across repositories.
    pub files: u32,
    /// Commits per local day, from the span's first day (Monday).
    pub days: [u32; 7],
    /// Commits per local hour.
    pub hours: [u32; 24],
    /// Commits per repository, most first.
    pub repos: Vec<(String, u32)>,
    /// `repo/path` touched by the most commits, first.
    pub hot_files: Vec<(String, u32)>,
    /// Authors, most commits first.
    pub people: Vec<Person>,
}

impl Week {
    pub fn active_days(&self) -> u32 {
        self.days.iter().filter(|&&n| n > 0).count() as u32
    }

    /// Index (Monday 0) of the day with the most commits.
    pub fn busiest_day(&self) -> Option<usize> {
        argmax(&self.days)
    }

    pub fn peak_hour(&self) -> Option<usize> {
        argmax(&self.hours)
    }
}

/// First index of the largest non-zero value.
fn argmax(values: &[u32]) -> Option<usize> {
    let max = *values.iter().max()?;
    (max > 0).then(|| values.iter().position(|&v| v == max).unwrap())
}

/// Adds up the commits of `repos` (name, commits) authored in `span`, only
/// `me`'s when given. A commit seen in several repositories (worktrees of
/// one) counts once. Days and hours are local, `utc_offset` seconds east.
pub fn week(
    repos: &[(String, Vec<Activity>)],
    span: TimeSpan,
    utc_offset: i64,
    me: Option<&Me>,
) -> Week {
    let first_day = (span.start + utc_offset).div_euclid(DAY);
    let mut out = Week::default();
    let mut seen = HashSet::new();
    let mut per_repo: Vec<(String, u32)> = Vec::new();
    let mut files: HashMap<String, u32> = HashMap::new();
    let mut ids = Identities::default();
    // (pair index, commits, additions, deletions) per author pair.
    let mut tally: HashMap<usize, (u32, u64, u64)> = HashMap::new();
    for (repo, commits) in repos {
        let mut count = 0;
        for c in commits {
            if !span.contains(c.time)
                || me.is_some_and(|me| !me.owns(&c.name, &c.email))
                || !seen.insert(c.id.as_str())
            {
                continue;
            }
            count += 1;
            out.commits += 1;
            out.additions += c.additions;
            out.deletions += c.deletions;
            let local = c.time + utc_offset;
            let day = (local.div_euclid(DAY) - first_day).clamp(0, 6) as usize;
            out.days[day] += 1;
            out.hours[(local.rem_euclid(DAY) / 3600) as usize] += 1;
            for f in &c.files {
                *files.entry(format!("{repo}/{f}")).or_default() += 1;
            }
            let pair = ids.intern(&(c.name.clone(), c.email.clone()));
            let t = tally.entry(pair).or_default();
            t.0 += 1;
            t.1 += c.additions;
            t.2 += c.deletions;
        }
        if count > 0 {
            match per_repo.iter_mut().find(|(r, _)| r == repo) {
                Some((_, n)) => *n += count,
                None => per_repo.push((repo.clone(), count)),
            }
        }
    }
    per_repo.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out.repos = per_repo;
    out.files = files.len() as u32;
    let mut hot: Vec<(String, u32)> = files.into_iter().collect();
    hot.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    hot.truncate(HOT_FILES);
    out.hot_files = hot;

    // Merge the pairs of one person; the name of their busiest pair wins.
    let mut people: HashMap<usize, (Person, u32)> = HashMap::new();
    let mut pairs: Vec<_> = tally.into_iter().collect();
    pairs.sort_by_key(|&(pair, _)| pair);
    for (pair, (commits, add, del)) in pairs {
        let root = ids.find(pair);
        let (name, email) = ids.pairs[pair].clone();
        let entry = people.entry(root).or_default();
        let p = &mut entry.0;
        if commits > entry.1 || p.name.is_empty() {
            entry.1 = commits;
            p.name = if name.is_empty() { email.clone() } else { name };
            p.email = email;
        }
        p.commits += commits;
        p.additions += add;
        p.deletions += del;
    }
    let mut people: Vec<Person> = people.into_values().map(|(p, _)| p).collect();
    people.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.name.cmp(&b.name)));
    out.people = people;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: &str, who: (&str, &str), time: i64, add: u64, files: &[&str]) -> Activity {
        Activity {
            id: id.into(),
            name: who.0.into(),
            email: who.1.into(),
            time,
            additions: add,
            deletions: 1,
            files: files.iter().map(|f| f.to_string()).collect(),
        }
    }

    #[test]
    fn parses_log_with_numstat() {
        let raw = "\x1eabc\x1fJane\x1fjane@x.org\x1f1700000000\n\n3\t1\tsrc/a.rs\n-\t-\tlogo.png\n\x1edef\x1fBob\x1fbob@x.org\x1f1700000100\n";
        let a = parse_activity(raw);
        assert_eq!(a.len(), 2);
        assert_eq!((a[0].additions, a[0].deletions), (3, 1));
        assert_eq!(a[0].files, ["src/a.rs", "logo.png"]);
        assert_eq!((a[1].name.as_str(), a[1].time), ("Bob", 1700000100));
        assert!(a[1].files.is_empty());
    }

    #[test]
    fn adds_up_a_week_by_local_day_and_person() {
        // Monday 00:00 UTC.
        let monday = 19_725 * DAY;
        let span = TimeSpan {
            start: monday,
            end: monday + 7 * DAY,
        };
        let me = ("Alexandre", "12600085+ziracmo@users.noreply.github.com");
        let repos = vec![
            (
                "app".to_string(),
                vec![
                    commit("1", me, monday + 9 * 3600, 10, &["a.rs"]),
                    commit(
                        "2",
                        ("ziracmo", "a@home.net"),
                        monday + DAY + 3600,
                        5,
                        &["a.rs", "b.rs"],
                    ),
                    commit("3", ("Bob", "bob@x.org"), monday + 2 * DAY, 1, &["c.rs"]),
                    // Last week: left out.
                    commit("0", me, monday - 3600, 99, &["old.rs"]),
                ],
            ),
            // A worktree of the same repository: commit 1 counts once.
            (
                "app-wt".to_string(),
                vec![commit("1", me, monday + 9 * 3600, 10, &["a.rs"])],
            ),
        ];
        let team = week(&repos, span, 0, None);
        assert_eq!(team.commits, 3);
        assert_eq!(team.additions, 16);
        assert_eq!(team.days[..3], [1, 1, 1]);
        assert_eq!(team.hours[9], 1);
        assert_eq!(team.repos, [("app".to_string(), 3)]);
        assert_eq!(team.files, 3);
        assert_eq!(team.hot_files[0], ("app/a.rs".to_string(), 2));
        assert_eq!(team.people.len(), 2);
        assert_eq!(team.people[0].commits, 2);
        assert_eq!(team.active_days(), 3);

        let mine = week(&repos, span, 0, Some(&Me::new([me])));
        assert_eq!(mine.commits, 2);
        assert_eq!(mine.people.len(), 1);

        // UTC+2: the 23:00 commit of the Sunday before is Monday 01:00 local,
        // but outside the span (spans are absolute).
        let east = week(&repos, span, 2 * 3600, None);
        assert_eq!(east.hours[11], 1);
        assert_eq!(east.busiest_day(), Some(0));
        assert_eq!(Week::default().peak_hour(), None);
    }
}
