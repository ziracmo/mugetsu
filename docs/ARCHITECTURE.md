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

- `mugetsu-git`: unit tests (parsers, graph, ordering) and integration tests against real temporary
  repositories.
- `mugetsu-forge`: URL parsing, PR state mapping and grouping, avatar URLs.
- `mugetsu-app`: UI integration tests in headless windows (branch click and double-click checkout, keyboard
  navigation, folders and sidebar filter, right-click rename, collapsible commit body, commit search, sidebar
  toggle) and end-to-end flows (stage then commit from the UI, background tab dot after a fetch).
