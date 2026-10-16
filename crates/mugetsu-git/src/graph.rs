//! Lane layout for the commit graph.
//!
//! Instead of storing every pass-through line per row (O(rows × lanes) memory, which
//! explodes on repos with hundreds of concurrent lanes), the layout stores one
//! [`Span`] per parent edge plus a row → span bucket index. Drawing a visible row
//! asks [`Graph::row`] for its segments, computed on the fly in O(spans touching it).

use std::collections::{BTreeSet, HashMap};

use gix::ObjectId;
use smallvec::SmallVec;

use crate::log::Commit;

pub type Lane = u16;

const BUCKET: usize = 64;

/// A line segment inside one row half: from lane `from` at one edge to lane `to` at the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: Lane,
    pub to: Lane,
    pub color: u16,
}

/// An edge child → parent. Leaves the child node at (`top_row`, `top_lane`), bends
/// into `lane`, runs straight down, and bends into `bottom_lane` at `bottom_row`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub top_row: u32,
    pub top_lane: Lane,
    pub lane: Lane,
    pub bottom_row: u32,
    pub bottom_lane: Lane,
    pub color: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Node {
    pub lane: Lane,
    pub color: u16,
}

/// Segments of one row, ready to draw.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphRow {
    pub lane: Lane,
    pub color: u16,
    /// Segments from the row's top edge down to its center line.
    pub upper: SmallVec<[Edge; 8]>,
    /// Segments from the row's center line down to its bottom edge.
    pub lower: SmallVec<[Edge; 8]>,
    /// Lanes needed to draw this row.
    pub width: Lane,
}

impl GraphRow {
    pub fn is_merge(&self) -> bool {
        self.lower.iter().filter(|e| e.from == self.lane).count() > 1
    }
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub spans: Vec<Span>,
    buckets: Vec<Vec<u32>>,
    /// Widest row, for sizing the graph column.
    pub max_width: Lane,
}

impl Graph {
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn row(&self, row: usize) -> GraphRow {
        let node = self.nodes[row];
        let r = row as u32;
        let mut out = GraphRow {
            lane: node.lane,
            color: node.color,
            width: node.lane + 1,
            ..Default::default()
        };
        let Some(bucket) = self.buckets.get(row / BUCKET) else {
            return out;
        };
        for &si in bucket {
            let s = self.spans[si as usize];
            if s.top_row > r || s.bottom_row < r {
                continue;
            }
            let (upper, lower) = if s.top_row == r {
                (None, Some((s.top_lane, s.lane)))
            } else if s.bottom_row == r {
                (Some((s.lane, s.bottom_lane)), None)
            } else {
                (Some((s.lane, s.lane)), Some((s.lane, s.lane)))
            };
            if let Some((from, to)) = upper {
                push_unique(
                    &mut out.upper,
                    Edge {
                        from,
                        to,
                        color: s.color,
                    },
                );
                out.width = out.width.max(from.max(to) + 1);
            }
            if let Some((from, to)) = lower {
                push_unique(
                    &mut out.lower,
                    Edge {
                        from,
                        to,
                        color: s.color,
                    },
                );
                out.width = out.width.max(from.max(to) + 1);
            }
        }
        out
    }
}

fn push_unique(v: &mut SmallVec<[Edge; 8]>, e: Edge) {
    if !v.iter().any(|x| x.from == e.from && x.to == e.to) {
        v.push(e);
    }
}

struct Slot {
    color: u16,
    /// Pending edges waiting for the expected commit: (child row, child lane).
    starts: SmallVec<[(u32, Lane); 2]>,
}

/// Assigns lanes to commits given in child-before-parent order. O(n log n).
pub fn layout(commits: &[Commit]) -> Graph {
    layout_with(
        commits.len(),
        commits.iter().map(|c| (&c.id, c.parents.iter().copied())),
    )
}

/// [`layout`] over any child-before-parent sequence of (commit, parents).
pub fn layout_with<'a, P>(len: usize, commits: impl Iterator<Item = (&'a ObjectId, P)>) -> Graph
where
    P: Iterator<Item = ObjectId>,
{
    let mut slots: Vec<Option<Slot>> = Vec::new();
    let mut free: BTreeSet<usize> = BTreeSet::new();
    // A commit may be awaited by several lanes (its children's first-parent lines);
    // they all converge into the leftmost when it shows up.
    let mut expected: HashMap<ObjectId, SmallVec<[usize; 2]>> = HashMap::new();
    let mut graph = Graph {
        nodes: Vec::with_capacity(len),
        ..Default::default()
    };
    let mut next_color: u16 = 0;

    let alloc = |slots: &mut Vec<Option<Slot>>, free: &mut BTreeSet<usize>| -> usize {
        match free.pop_first() {
            Some(i) => i,
            None => {
                slots.push(None);
                slots.len() - 1
            }
        }
    };

    let mut rows = 0u32;
    for (row, (id, mut parents)) in commits.enumerate() {
        let row = row as u32;
        rows = row + 1;
        let (lane, color) = match expected.remove(id) {
            Some(lanes) => {
                let lane = *lanes.iter().min().expect("non-empty");
                let color = slots[lane]
                    .as_ref()
                    .expect("awaiting lane is occupied")
                    .color;
                for i in lanes {
                    let slot = slots[i].take().expect("awaiting lane is occupied");
                    for (top_row, top_lane) in slot.starts {
                        graph.spans.push(Span {
                            top_row,
                            top_lane,
                            lane: i as Lane,
                            bottom_row: row,
                            bottom_lane: lane as Lane,
                            color: slot.color,
                        });
                    }
                    if i != lane {
                        free.insert(i);
                    }
                }
                (lane, color)
            }
            None => {
                let i = alloc(&mut slots, &mut free);
                let c = next_color;
                next_color = next_color.wrapping_add(1);
                (i, c)
            }
        };
        graph.nodes.push(Node {
            lane: lane as Lane,
            color,
        });

        match parents.next() {
            // The first parent always continues straight down in this lane.
            Some(first) => {
                slots[lane] = Some(Slot {
                    color,
                    starts: smallvec::smallvec![(row, lane as Lane)],
                });
                expected.entry(first).or_default().push(lane);
            }
            None => {
                free.insert(lane);
            }
        }
        for parent in parents {
            if let Some(&existing) = expected.get(&parent).and_then(|l| l.first()) {
                // Merge parent already awaited elsewhere: join that line.
                slots[existing]
                    .as_mut()
                    .unwrap()
                    .starts
                    .push((row, lane as Lane));
                continue;
            }
            let target = alloc(&mut slots, &mut free);
            let c = next_color;
            next_color = next_color.wrapping_add(1);
            slots[target] = Some(Slot {
                color: c,
                starts: smallvec::smallvec![(row, lane as Lane)],
            });
            expected.entry(parent).or_default().push(target);
        }
        while slots.last().is_some_and(Option::is_none) {
            slots.pop();
            free.remove(&slots.len());
        }
    }

    // Edges to commits outside the walked history (shallow clones) run off the bottom.
    let end = rows;
    for (i, slot) in slots.into_iter().enumerate() {
        if let Some(slot) = slot {
            for (top_row, top_lane) in slot.starts {
                graph.spans.push(Span {
                    top_row,
                    top_lane,
                    lane: i as Lane,
                    bottom_row: end,
                    bottom_lane: i as Lane,
                    color: slot.color,
                });
            }
        }
    }

    let buckets = (rows as usize).div_ceil(BUCKET).max(1);
    graph.buckets = vec![Vec::new(); buckets];
    for (si, s) in graph.spans.iter().enumerate() {
        let first = s.top_row as usize / BUCKET;
        let last = (s.bottom_row as usize / BUCKET).min(buckets - 1);
        for b in &mut graph.buckets[first..=last] {
            b.push(si as u32);
        }
        graph.max_width = graph
            .max_width
            .max(s.lane.max(s.top_lane).max(s.bottom_lane) + 1);
    }
    graph.max_width = graph
        .max_width
        .max(graph.nodes.iter().map(|n| n.lane + 1).max().unwrap_or(0));
    graph
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::tests::commit;

    fn pairs(edges: &[Edge]) -> Vec<(Lane, Lane)> {
        let mut v: Vec<_> = edges.iter().map(|x| (x.from, x.to)).collect();
        v.sort();
        v
    }

    fn rows(commits: &[Commit]) -> Vec<GraphRow> {
        let g = layout(commits);
        (0..g.len()).map(|i| g.row(i)).collect()
    }

    #[test]
    fn linear_history_stays_in_lane_zero() {
        let rows = rows(&[commit(3, 3, &[2]), commit(2, 2, &[1]), commit(1, 1, &[])]);
        assert!(rows.iter().all(|r| r.lane == 0 && r.width == 1));
        assert_eq!(pairs(&rows[0].upper), vec![]);
        assert_eq!(pairs(&rows[0].lower), vec![(0, 0)]);
        assert_eq!(pairs(&rows[1].upper), vec![(0, 0)]);
        assert_eq!(pairs(&rows[2].lower), vec![]);
        assert!(rows.iter().all(|r| r.color == rows[0].color));
    }

    #[test]
    fn merge_branches_out_then_converges() {
        // 4 = merge(2, 3); 2 and 3 both from 1.
        let rows = rows(&[
            commit(4, 4, &[2, 3]),
            commit(3, 3, &[1]),
            commit(2, 2, &[1]),
            commit(1, 1, &[]),
        ]);
        assert!(rows[0].is_merge());
        assert_eq!(pairs(&rows[0].lower), vec![(0, 0), (0, 1)]);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(pairs(&rows[1].upper), vec![(0, 0), (1, 1)]);
        // 2 stays in lane 0 while lane 1 also waits for 1; both converge at 1.
        assert_eq!(rows[2].lane, 0);
        assert_eq!(rows[3].lane, 0);
        assert_eq!(pairs(&rows[3].upper), vec![(0, 0), (1, 0)]);
        assert_ne!(rows[1].color, rows[0].color);
    }

    #[test]
    fn independent_tips_get_their_own_lanes() {
        let rows = rows(&[commit(3, 3, &[1]), commit(2, 2, &[1]), commit(1, 1, &[])]);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[2].lane, 0);
        assert_eq!(pairs(&rows[2].upper), vec![(0, 0), (1, 0)]);
        assert_eq!(rows[1].width, 2);
    }

    #[test]
    fn freed_lanes_are_reused() {
        // Two consecutive short-lived side branches should both reuse lane 1.
        let rows = rows(&[
            commit(7, 7, &[6, 5]),
            commit(5, 6, &[4]),
            commit(6, 5, &[4]),
            commit(4, 4, &[3, 2]),
            commit(2, 3, &[1]),
            commit(3, 2, &[1]),
            commit(1, 1, &[]),
        ]);
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[4].lane, 1);
        assert!(rows.iter().all(|r| r.width <= 2), "{rows:#?}");
    }

    #[test]
    fn missing_parents_run_off_the_bottom() {
        let g = layout(&[commit(2, 2, &[1])]);
        assert_eq!(pairs(&g.row(0).lower), vec![(0, 0)]);
    }

    #[test]
    fn spans_crossing_bucket_boundaries_are_found() {
        // Tip 255 points straight to root 1 while a 150-commit chain runs beside it.
        let mut commits = vec![commit(255, 1000, &[1])];
        for n in (2..=151u8).rev() {
            commits.push(commit(n, n as i64 + 1, &[n - 1]));
        }
        commits.push(commit(1, 1, &[]));
        let g = layout(&commits);
        for row in [1, 63, 64, 100, 150] {
            assert!(
                g.row(row).upper.iter().any(|e| (e.from, e.to) == (0, 0)),
                "row {row}: {:?}",
                g.row(row)
            );
        }
        assert_eq!(pairs(&g.row(151).upper), vec![(0, 0), (1, 0)]);
    }
}
