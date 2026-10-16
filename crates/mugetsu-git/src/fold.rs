//! Folding merged branches: a merge commit hides the commits its other parents
//! bring in (the merged branch) until it is expanded, like Fork does.
//!
//! A commit stays visible when it is reachable from a tip, a pinned commit
//! (ref, HEAD) or an expanded merge without going through a folded merge's
//! non-first parent. One pass in history order, so O(n), plus the layout.

use std::collections::{HashMap, HashSet};

use gix::ObjectId;

use crate::graph::{Graph, layout_with};
use crate::log::History;

const NONE: u32 = u32::MAX;
const MANY: u32 = u32::MAX - 1;

/// What the history list shows once merges are folded.
#[derive(Clone, Debug, Default)]
pub struct Folded {
    /// History rows shown, in order.
    pub rows: Vec<u32>,
    /// History row → shown row, [`NONE`] when hidden.
    shown: Vec<u32>,
    /// Hidden history row → the merge (history row) hiding it.
    owner: Vec<u32>,
    /// Merges (history rows) that fold something: `true` while folded.
    pub toggles: HashMap<u32, bool>,
    /// Lanes of the shown rows (indexed by shown row).
    pub graph: Graph,
}

impl Folded {
    /// Folds every merge except `expanded`; `pinned` commits (refs, HEAD) stay visible.
    pub fn new(
        history: &History,
        pinned: &HashSet<ObjectId>,
        expanded: &HashSet<ObjectId>,
    ) -> Self {
        let commits = &history.commits;
        let n = commits.len();
        let mut has_child = vec![false; n];
        for c in commits {
            for p in &c.parents {
                if let Some(r) = history.row_of(p) {
                    has_child[r] = true;
                }
            }
        }
        // NONE: not reached yet; MANY: reached by a plain edge; m: reached only
        // through expanded merge m (folding m would hide it).
        let mut reach: Vec<u32> = (0..n)
            .map(|r| {
                if !has_child[r] || pinned.contains(&commits[r].id) {
                    MANY
                } else {
                    NONE
                }
            })
            .collect();
        let mut owner = vec![NONE; n];
        let mut shown = vec![NONE; n];
        let mut rows = Vec::with_capacity(n);
        let mut toggles = HashMap::new();
        for (r, c) in commits.iter().enumerate() {
            let parents = c.parents.iter().filter_map(|p| history.row_of(p));
            if reach[r] == NONE {
                // Hidden: everything it alone leads to belongs to the same fold.
                let o = owner[r];
                toggles.insert(o, true);
                for p in parents {
                    if reach[p] == NONE && owner[p] == NONE {
                        owner[p] = o;
                    }
                }
                continue;
            }
            if reach[r] != MANY {
                toggles.insert(reach[r], false);
            }
            shown[r] = rows.len() as u32;
            rows.push(r as u32);
            let open = c.parents.len() > 1 && expanded.contains(&c.id);
            for (i, p) in c.parents.iter().enumerate() {
                let Some(p) = history.row_of(p) else { continue };
                if i == 0 {
                    reach[p] = MANY;
                } else if open {
                    reach[p] = if reach[p] == NONE { r as u32 } else { MANY };
                } else if owner[p] == NONE {
                    owner[p] = r as u32;
                }
            }
        }
        let graph = layout_with(
            rows.len(),
            rows.iter().map(|&r| {
                let c = &commits[r as usize];
                let shown = &shown;
                (
                    &c.id,
                    c.parents
                        .iter()
                        .copied()
                        .filter(move |p| history.row_of(p).is_none_or(|p| shown[p] != NONE)),
                )
            }),
        );
        Self {
            rows,
            shown,
            owner,
            toggles,
            graph,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// History row of shown row `ix`.
    pub fn history_row(&self, ix: usize) -> usize {
        self.rows[ix] as usize
    }

    /// Shown row of history row `row`, `None` when folded away.
    pub fn shown_row(&self, row: usize) -> Option<usize> {
        self.shown
            .get(row)
            .copied()
            .filter(|&s| s != NONE)
            .map(|s| s as usize)
    }

    /// Merge (history row) to expand so that hidden history row `row` gets closer to view.
    pub fn owner(&self, row: usize) -> Option<usize> {
        self.owner
            .get(row)
            .copied()
            .filter(|_| self.shown_row(row).is_none())
            .filter(|&o| o != NONE)
            .map(|o| o as usize)
    }

    /// `Some(true)` for a folded merge, `Some(false)` for an expanded one that folds.
    pub fn toggle(&self, row: usize) -> Option<bool> {
        self.toggles.get(&(row as u32)).copied()
    }

    /// Some shown merge folds something (the list needs its arrow gutter).
    pub fn has_toggles(&self) -> bool {
        self.toggles
            .keys()
            .any(|&m| self.shown_row(m as usize).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::tests::{commit, oid as id};

    /// 6 = merge(4, 5); 5 → 3 → 2 (merged branch); 4 → 2 → 1.
    fn history() -> History {
        History::from_commits(vec![
            commit(6, 6, &[4, 5]),
            commit(5, 5, &[3]),
            commit(4, 4, &[2]),
            commit(3, 3, &[2]),
            commit(2, 2, &[1]),
            commit(1, 1, &[]),
        ])
    }

    fn shown(h: &History, f: &Folded) -> Vec<u8> {
        f.rows
            .iter()
            .map(|&r| h.commits[r as usize].id.as_bytes()[0])
            .collect()
    }

    #[test]
    fn merged_branch_is_folded_by_default() {
        let h = history();
        let f = Folded::new(&h, &HashSet::new(), &HashSet::new());
        assert_eq!(shown(&h, &f), vec![6, 4, 2, 1]);
        let merge = h.row_of(&id(6)).unwrap();
        assert_eq!(f.toggle(merge), Some(true));
        assert!(f.has_toggles());
        // The folded merge draws a single line.
        assert!(!f.graph.row(0).is_merge());
        assert_eq!(f.owner(h.row_of(&id(3)).unwrap()), Some(merge));
        assert_eq!(f.owner(h.row_of(&id(4)).unwrap()), None);
    }

    #[test]
    fn expanded_merge_shows_its_branch() {
        let h = history();
        let f = Folded::new(&h, &HashSet::new(), &HashSet::from([id(6)]));
        assert_eq!(shown(&h, &f), vec![6, 5, 4, 3, 2, 1]);
        assert_eq!(f.toggle(h.row_of(&id(6)).unwrap()), Some(false));
        assert!(f.graph.row(0).is_merge());
    }

    #[test]
    fn refs_inside_a_merged_branch_stay_visible() {
        let h = history();
        let f = Folded::new(&h, &HashSet::from([id(5)]), &HashSet::new());
        assert_eq!(shown(&h, &f), vec![6, 5, 4, 3, 2, 1]);
        assert_eq!(f.toggle(h.row_of(&id(6)).unwrap()), None);
        assert!(!f.has_toggles());
    }

    #[test]
    fn nested_merges_unfold_one_level_at_a_time() {
        // 9 = merge(8, 7); 7 = merge(5, 6) on the merged branch.
        let h = History::from_commits(vec![
            commit(9, 9, &[8, 7]),
            commit(8, 8, &[1]),
            commit(7, 7, &[5, 6]),
            commit(6, 6, &[1]),
            commit(5, 5, &[1]),
            commit(1, 1, &[]),
        ]);
        let f = Folded::new(&h, &HashSet::new(), &HashSet::new());
        assert_eq!(shown(&h, &f), vec![9, 8, 1]);
        let six = h.row_of(&id(6)).unwrap();
        assert_eq!(f.owner(six), h.row_of(&id(9)));
        let f = Folded::new(&h, &HashSet::new(), &HashSet::from([id(9)]));
        assert_eq!(shown(&h, &f), vec![9, 8, 7, 5, 1]);
        assert_eq!(f.owner(six), h.row_of(&id(7)));
    }
}
