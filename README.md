# KDEQuickPeek

Quick file previewer for KDE Plasma. Select a file and press Space.

## Install

On Arch or CachyOS, from this folder:

```bash
makepkg -si
```

The command installs `quickpeek`, `quickpeek-desktop` and the "Quick Preview" entry in Dolphin's right-click menu. Optional features list the packages they need: see `optdepends` in `PKGBUILD`, or run `pacman -Qi quickpeek`.

To make Space open quickpeek on desktop icons:

```bash
quickpeek-desktop install
systemctl --user restart plasma-plasmashell
```

Run `quickpeek-desktop install` again after every Plasma update.

AI models download on first use into `~/.local/share/quickpeek/models`.

## Update

After editing `quickpeek.py`, commit, then run `makepkg -si` again. The version number comes from the commit count.

## Offline copy

`makepkg` leaves a `quickpeek-*.pkg.tar.zst` file in this folder. Keep a copy on a USB stick or cloud drive. Install it on a fresh system with:

```bash
sudo pacman -U quickpeek-*.pkg.tar.zst
```

## Tests

```bash
QT_QPA_PLATFORM=offscreen python3 test_quickpeek.py
```
