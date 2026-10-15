//! Groups slash-separated ref names into folders, like Fork's sidebar:
//! `feat/a`, `feat/b` → a `feat` folder holding `a` and `b`.

/// Names pinned above folders at the root, in this order.
const PINNED: [&str; 5] = ["main", "master", "develop", "dev", "trunk"];

#[derive(Debug, PartialEq, Eq)]
pub struct TreeNode<T> {
    /// Last path segment, shown in the sidebar.
    pub label: String,
    /// Full path (`feat/a` for a leaf, `feat` for a folder).
    pub path: String,
    pub kind: NodeKind<T>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NodeKind<T> {
    Folder(Vec<TreeNode<T>>),
    Leaf(T),
}

impl<T> TreeNode<T> {
    /// Number of leaves below (1 for a leaf).
    pub fn leaf_count(&self) -> usize {
        match &self.kind {
            NodeKind::Leaf(_) => 1,
            NodeKind::Folder(children) => children.iter().map(TreeNode::leaf_count).sum(),
        }
    }
}

/// Builds the folder tree. Root order: pinned names, folders, then leaves;
/// inside folders: folders then leaves, each alphabetical (case-insensitive).
pub fn build<T>(items: impl IntoIterator<Item = (String, T)>) -> Vec<TreeNode<T>> {
    let mut root = Vec::new();
    for (name, value) in items {
        insert(&mut root, "", &name, value);
    }
    sort(&mut root, true);
    root
}

fn insert<T>(nodes: &mut Vec<TreeNode<T>>, prefix: &str, rest: &str, value: T) {
    match rest.split_once('/') {
        Some((dir, tail)) if !dir.is_empty() && !tail.is_empty() => {
            let path = format!("{prefix}{dir}");
            let index = match nodes
                .iter()
                .position(|n| n.path == path && matches!(n.kind, NodeKind::Folder(_)))
            {
                Some(i) => i,
                None => {
                    nodes.push(TreeNode {
                        label: dir.to_string(),
                        path: path.clone(),
                        kind: NodeKind::Folder(Vec::new()),
                    });
                    nodes.len() - 1
                }
            };
            if let NodeKind::Folder(children) = &mut nodes[index].kind {
                insert(children, &format!("{path}/"), tail, value);
            }
        }
        _ => nodes.push(TreeNode {
            label: rest.to_string(),
            path: format!("{prefix}{rest}"),
            kind: NodeKind::Leaf(value),
        }),
    }
}

fn sort<T>(nodes: &mut [TreeNode<T>], is_root: bool) {
    nodes.sort_by_cached_key(|n| {
        let pinned = if is_root && matches!(n.kind, NodeKind::Leaf(_)) {
            PINNED.iter().position(|p| *p == n.label)
        } else {
            None
        };
        (
            pinned.is_none(),
            pinned,
            matches!(n.kind, NodeKind::Leaf(_)),
            n.label.to_lowercase(),
        )
    });
    for n in nodes {
        if let NodeKind::Folder(children) = &mut n.kind {
            sort(children, false);
        }
    }
}

/// Case-insensitive match of every whitespace-separated word of `query`.
pub fn matches(name: &str, query: &str) -> bool {
    let name = name.to_lowercase();
    query
        .split_whitespace()
        .all(|word| name.contains(&word.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(nodes: &[TreeNode<()>]) -> Vec<String> {
        nodes
            .iter()
            .map(|n| match &n.kind {
                NodeKind::Leaf(()) => n.label.clone(),
                NodeKind::Folder(c) => format!("{}/[{}]", n.label, shape(c).join(",")),
            })
            .collect()
    }

    #[test]
    fn groups_by_folder_with_pinned_first() {
        let names = [
            "reduction-band",
            "feat/zeta",
            "develop",
            "feat/aia/deep",
            "Alerts",
            "master",
            "feat/alpha",
            "fix/x",
        ];
        let tree = build(names.iter().map(|n| (n.to_string(), ())));
        assert_eq!(
            shape(&tree),
            [
                "master",
                "develop",
                "feat/[aia/[deep],alpha,zeta]",
                "fix/[x]",
                "Alerts",
                "reduction-band",
            ]
        );
        assert_eq!(tree[2].path, "feat");
        assert_eq!(tree[2].leaf_count(), 3);
        let NodeKind::Folder(feat) = &tree[2].kind else {
            panic!()
        };
        assert_eq!(feat[0].path, "feat/aia");
        assert_eq!(feat[1].path, "feat/alpha");
    }

    #[test]
    fn odd_names_stay_leaves() {
        let tree = build(["/x", "y/"].iter().map(|n| (n.to_string(), ())));
        assert_eq!(shape(&tree), ["/x", "y/"]);
    }

    #[test]
    fn search_matches_all_words() {
        assert!(matches("feat/am-i-attacking", "ATTACK feat"));
        assert!(!matches("feat/am-i-attacking", "fix attack"));
        assert!(matches("anything", "  "));
    }
}
