use std::collections::{BinaryHeap, HashMap};

use anyhow::Context;
use gix::ObjectId;
use smallvec::SmallVec;

use crate::Repo;

/// One commit as displayed in the history list.
#[derive(Clone, Debug)]
pub struct Commit {
    pub id: ObjectId,
    pub parents: SmallVec<[ObjectId; 2]>,
    pub author: String,
    pub email: String,
    /// Seconds since epoch (committer time, used for ordering).
    pub time: i64,
    pub summary: String,
}

impl Commit {
    pub fn short_id(&self) -> String {
        self.id.to_hex_with_len(8).to_string()
    }
}

/// Full history of a repository, in `git log --date-order` order
/// (children always before parents, ties broken by newest commit time).
#[derive(Clone, Debug, Default)]
pub struct History {
    pub commits: Vec<Commit>,
    pub index: HashMap<ObjectId, usize>,
}

impl History {
    pub fn from_commits(commits: Vec<Commit>) -> Self {
        let commits = date_order(commits);
        let index = commits.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        Self { commits, index }
    }

    pub fn len(&self) -> usize {
        self.commits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commits.is_empty()
    }

    pub fn row_of(&self, id: &ObjectId) -> Option<usize> {
        self.index.get(id).copied()
    }
}

impl Repo {
    /// Walks every commit reachable from `tips` (usually all refs + HEAD).
    pub fn history(&self, tips: &[ObjectId]) -> anyhow::Result<History> {
        let mut repo = gix::open(self.root()).context("opening repository with gix")?;
        repo.object_cache_size_if_unset(64 * 1024 * 1024);
        // Tags may point to tags, trees or blobs: keep only what peels to a commit.
        let tips: Vec<ObjectId> = tips
            .iter()
            .filter_map(|id| {
                repo.find_object(*id)
                    .ok()?
                    .peel_to_commit()
                    .ok()
                    .map(|c| c.id)
            })
            .collect();
        // Plain graph walk is ~10x faster than gix's time-sorted walk; ordering is
        // done afterwards by `date_order`, and metadata is decoded in parallel.
        let ids: Vec<ObjectId> = repo
            .rev_walk(tips)
            .all()?
            .map(|i| i.map(|i| i.id))
            .collect::<Result<_, _>>()?;
        let sync = repo.into_sync();
        let threads = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(8);
        let chunk = ids.len().div_ceil(threads).max(1);
        let commits = std::thread::scope(|scope| {
            let handles: Vec<_> = ids
                .chunks(chunk)
                .map(|part| {
                    let sync = &sync;
                    scope.spawn(move || -> anyhow::Result<Vec<Commit>> {
                        let mut repo = sync.to_thread_local();
                        repo.object_cache_size_if_unset(16 * 1024 * 1024);
                        part.iter().map(|id| decode(&repo, *id)).collect()
                    })
                })
                .collect();
            let mut all = Vec::with_capacity(ids.len());
            for h in handles {
                all.extend(h.join().expect("decode thread panicked")?);
            }
            anyhow::Ok(all)
        })?;
        Ok(History::from_commits(commits))
    }
}

fn decode(repo: &gix::Repository, id: ObjectId) -> anyhow::Result<Commit> {
    let commit = repo.find_commit(id)?;
    let decoded = commit.decode()?;
    let author = decoded.author()?;
    let committer = decoded.committer()?;
    Ok(Commit {
        id,
        parents: decoded.parents().collect(),
        author: author.name.to_string(),
        email: author.email.to_string(),
        time: committer.seconds(),
        summary: decoded.message_summary().to_string(),
    })
}

/// Reorders commits so every child precedes its parents, preferring newer commits
/// first (git's `--date-order`). Robust against clock skew in the input order.
pub fn date_order(commits: Vec<Commit>) -> Vec<Commit> {
    let n = commits.len();
    let index: HashMap<ObjectId, usize> =
        commits.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
    let mut pending_children = vec![0u32; n];
    for c in &commits {
        for p in &c.parents {
            if let Some(&pi) = index.get(p) {
                pending_children[pi] += 1;
            }
        }
    }
    // Max-heap on (time, reverse input position) so equal times keep input order.
    let mut ready: BinaryHeap<(i64, std::cmp::Reverse<usize>)> = commits
        .iter()
        .enumerate()
        .filter(|(i, _)| pending_children[*i] == 0)
        .map(|(i, c)| (c.time, std::cmp::Reverse(i)))
        .collect();
    let mut order = Vec::with_capacity(n);
    while let Some((_, std::cmp::Reverse(i))) = ready.pop() {
        order.push(i);
        for p in &commits[i].parents {
            if let Some(&pi) = index.get(p) {
                pending_children[pi] -= 1;
                if pending_children[pi] == 0 {
                    ready.push((commits[pi].time, std::cmp::Reverse(pi)));
                }
            }
        }
    }
    let mut slots: Vec<Option<Commit>> = commits.into_iter().map(Some).collect();
    order
        .into_iter()
        .map(|i| slots[i].take().expect("each commit emitted once"))
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn oid(n: u8) -> ObjectId {
        ObjectId::from_bytes_or_panic(&[n; 20])
    }

    pub fn commit(n: u8, time: i64, parents: &[u8]) -> Commit {
        Commit {
            id: oid(n),
            parents: parents.iter().map(|p| oid(*p)).collect(),
            author: String::new(),
            email: String::new(),
            time,
            summary: format!("c{n}"),
        }
    }

    #[test]
    fn date_order_puts_children_first_even_with_clock_skew() {
        // 3 -> 2 -> 1, but 3 has an older timestamp than its parent.
        let input = vec![commit(2, 20, &[1]), commit(1, 10, &[]), commit(3, 5, &[2])];
        let out: Vec<_> = date_order(input)
            .iter()
            .map(|c| c.summary.clone())
            .collect();
        assert_eq!(out, ["c3", "c2", "c1"]);
    }

    #[test]
    fn date_order_interleaves_branches_by_time() {
        //   4 merges 2 and 3, both children of 1
        let input = vec![
            commit(1, 1, &[]),
            commit(2, 2, &[1]),
            commit(3, 3, &[1]),
            commit(4, 4, &[2, 3]),
        ];
        let out: Vec<_> = date_order(input)
            .iter()
            .map(|c| c.summary.clone())
            .collect();
        assert_eq!(out, ["c4", "c3", "c2", "c1"]);
    }
}
