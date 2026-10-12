# KDEQuickPeek

Quick file previewer for KDE Plasma. Select a file and press Space.

> **Hobby project.** I made this for fun, to learn. It has no goal beyond that. It comes with no support: issues are open but I cannot promise imminent fixes. It may break or change at any time.

## Install

On Arch, Fedora, Debian, openSUSE and distributions based on them:

```bash
curl -fsSL https://raw.githubusercontent.com/LaskyWolf/quickpeek/main/install.sh | bash
```

The script installs the Qt packages it needs with your package manager (it asks for your password through `sudo`), then puts KDEQuickPeek in `~/.local`. If your distribution's PySide6 lacks a part, it downloads PySide6 from PyPI instead (about 600 MB; the window then uses the plain Qt look instead of your Plasma theme).

Run the same command again to update. To uninstall:

```bash
curl -fsSL https://raw.githubusercontent.com/LaskyWolf/quickpeek/main/install.sh | bash -s -- --uninstall
```

On Arch or CachyOS you can build a package instead, from a clone of this repository:

```bash
makepkg -si
```

Both ways install `quickpeek`, `quickpeek-desktop` and the "Quick Preview" entry in Dolphin's right-click menu.

Some previews use other programs when they are installed: ffmpeg (video and audio), poppler (PDF thumbnails), 7zip or libarchive (archives), assimp and blender (3D models), krita, onlyoffice (Office files and presentations). See `optdepends` in `PKGBUILD` for the full list.

To make Space open quickpeek on desktop icons:

```bash
quickpeek-desktop install
systemctl --user restart plasma-plasmashell
```

Run `quickpeek-desktop install` again after every Plasma update.

AI models download on first use into `~/.local/share/quickpeek/models`.

## Tests

```bash
QT_QPA_PLATFORM=offscreen python3 test_quickpeek.py
```

## License

GPL-3.0-or-later. See `LICENSE`.
