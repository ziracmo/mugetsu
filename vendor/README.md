# Vendored crates

- `gpui-component` 0.7.3 (Apache-2.0), patched in `Cargo.toml`.
  Local change: `src/menu/popup_menu.rs`, popup menus (dropdowns and context
  menus) get a 1px light-to-dark grey gradient edge around an opaque surface.
  Shortcut hints in menu rows use the row's text colour, dimmed, instead of a
  fixed muted grey, so they stay readable on the highlighted (accent) row.
