# peek

Quick file previewer for KDE Plasma. Select a file and press Space.

## Install

On Arch or CachyOS, from this folder:

```bash
makepkg -si
```

The command installs `peek`, `peek-desktop` and the "Quick Preview" entry in Dolphin's right-click menu. Optional features list the packages they need: see `optdepends` in `PKGBUILD`, or run `pacman -Qi peek`.

To make Space open peek on desktop icons:

```bash
peek-desktop install
systemctl --user restart plasma-plasmashell
```

Run `peek-desktop install` again after every Plasma update.

AI models download on first use into `~/.local/share/peek/models`.

## Update

After editing `peek.py`, commit, then run `makepkg -si` again. The version number comes from the commit count.

## Offline copy

`makepkg` leaves a `peek-*.pkg.tar.zst` file in this folder. Keep a copy on a USB stick or cloud drive. Install it on a fresh system with:

```bash
sudo pacman -U peek-*.pkg.tar.zst
```

## Tests

```bash
QT_QPA_PLATFORM=offscreen python3 test_peek.py
```
