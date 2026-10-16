# Architecture

```
crates/
  mugetsu-git    Git engine: history through gix (in-process, parallel decoding),
                 everything else through the git CLI (config, hooks, credentials, fsmonitor)
                 graph.rs: lane layout in O(n log n) time, O(n) memory
  mugetsu-forge  GitHub: remote URL → owner/repo, pull requests, avatars
  mugetsu-app    GPUI UI (gpui-kit): workspace, tabs, views
```

The UI is drawn on the GPU with [GPUI](https://www.gpui.rs/). The commit list is virtualized: only visible
rows are laid out and painted, and only the active tab renders; background tabs keep their data in memory.

## Performance

Measured on git.git (85,593 commits, 1,017 refs, 4 cores):

| Step | Time |
| --- | --- |
| Load refs | 40 ms |
| Full history (with a commit-graph) | 0.66 s |
| Graph layout | 13 ms |
| Segments for 60 visible rows | 0.1 ms |

## Tests

```sh
cargo test --workspace
```

Three layers:

- **Unit tests**, next to the code (`#[cfg(test)] mod tests`): parsers, graph layout, ordering, forge URL
  parsing and PR grouping, the optimistic checkout model, theme and animation curves.
- **Integration tests** in `crates/mugetsu-git/tests/`: every git operation against real temporary
  repositories (`repo.rs`: history, refs, conflicts, rebase, restack; `ops.rs`: stage, commit, branches, tags,
  stashes, worktrees, remotes, push, pull).
- **App tests** in `crates/mugetsu-app/src/tests*`, real views in headless GPUI windows driven by native clicks
  and keys: `tests/components.rs` checks one view at a time (welcome screen, changes view, history menu),
  `tests/flows.rs` and `tests.rs` run whole user flows (checkout, stage and commit, rename, cherry-pick, revert,
  new branch or tag from a commit, conflicts, interactive rebase, palette, workspaces).

On Linux the app tests need `libxkbcommon-dev libxkbcommon-x11-dev` to link. CI (`.github/workflows/ci.yml`)
runs fmt, clippy and the whole suite on every pull request, on Linux and macOS.
