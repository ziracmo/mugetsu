# Vendored crates

- `gpui-component` 0.7.3 (Apache-2.0), patched in `Cargo.toml`.
  Local change: `src/menu/popup_menu.rs`, popup menus (dropdowns and context
  menus) get a 1px light-to-dark grey gradient edge around an opaque surface.
  Shortcut hints in menu rows use the row's text colour, dimmed, instead of a
  fixed muted grey, so they stay readable on the highlighted (accent) row.
  A submenu opens on the side its width fits (upstream checked the parent's
  left edge, so it ran off the window); long labels end in an ellipsis with a
  tooltip, and `PopupMenuItem::tooltip` sets one by hand.
  `src/diff/state.rs`, `src/diff/mod.rs`: row heights are hinted again after
  the list drops them on a width change (else the scroll stopped at the rows
  already measured, often the file scrolled to); the diff tracks the file
  being read (`active_file`, `DiffEvent::ActiveFileChanged`) for a scroll spy.
