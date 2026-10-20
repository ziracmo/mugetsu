# dmgbuild settings for Mugetsu.dmg, used by scripts/dmg-macos.sh.
# Run: dmgbuild -s packaging/macos/dmg-settings.py -D app=<path to Mugetsu.app> Mugetsu <out.dmg>
# from the repository root. Positions are icon centres in the 660x400 background (see dmg-background.py).
app = defines["app"]  # noqa: F821 (provided by dmgbuild)

# LZMA: about 30% smaller than the default zlib (UDZO); opens on macOS 10.15+.
format = "ULMO"
files = [app]
symlinks = {"Applications": "/Applications"}
hide_extension = ["Mugetsu.app"]

# dmgbuild picks up dmg-background@2x.png next to it and makes a Retina TIFF.
background = "packaging/macos/dmg-background.png"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
window_rect = ((200, 120), (660, 428))  # content is 400 high; the rest is the title bar
default_view = "icon-view"
show_icon_preview = False
arrange_by = None
grid_offset = (0, 0)
grid_spacing = 100
label_pos = "bottom"
text_size = 13
icon_size = 128
icon_locations = {
    "Mugetsu.app": (170, 190),
    "Applications": (490, 190),
}
