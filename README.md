<div align="center">

<img src=".github/assets/banner.svg" width="100%" alt="QuickPeek: press Space in Dolphin, see inside any file" />

[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0--or--later-blue)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-cxx--qt-DEA584?logo=rust&logoColor=white)](Cargo.toml)
[![Qt 6](https://img.shields.io/badge/Qt-6%20(QML)-41CD52?logo=qt&logoColor=white)](qml/Main.qml)

**Select a file in Dolphin, press Space, see what is inside.**
Previews for images, PDF, video, 3D models, code and more.

</div>

> **Hobby project.** I made this for fun, to learn. It has no goal beyond that. It comes with no support: issues are open but I cannot promise imminent fixes. It may break or change at any time.

## Highlights

| | |
| --- | --- |
| ⌨️ **One key** | Press Space in Dolphin to open, Space or Q to close. Arrow keys move through the folder in Dolphin's order. |
| 🔎 **Read what is in it** | Search PDFs, find and copy text in pictures (OCR), pick a subject with AI. |
| ✍️ **Annotate safely** | Draw, highlight and write on a PDF. The original never changes. |
| 🧊 **Native Qt 6** | Light and dark follow your system theme. |

## What it previews

- **Images:** zoom, pick a subject with AI, find and copy text in the picture (OCR), enlarge 2x or 4x.
- **PDF:** read, search, draw, highlight and write on it. The original is never changed: the edit is saved as a new PDF.
- **Word and e-books:** pages with text and picture selection.
- **Presentations:** slide by slide, with Morph transitions.
- **Spreadsheets:** `.xlsx`, `.xls`, `.ods`, `.csv`.
- **Video and audio:** playback, thumbnails, waveforms.
- **3D models and `.blend` files:** fly around the model with the mouse wheel and keys.
- **Revit, Affinity, Krita and Clip Studio files:** embedded preview.
- **Code and text:** syntax highlighting.
- **Archives, fonts and folders:** contents, font samples, icon grid.

Move between files in the same folder with the arrow keys, in the order Dolphin shows them.

Inside a preview:

| Key | Action |
|---|---|
| Space or Q | Close |
| Left, Right, Up, Down | Previous or next file |
| Return | Open with the default app |
| I or C | Details and tools panel |
| Ctrl+C, Ctrl+Shift+C | Copy file, copy path |
| F2, Delete | Rename, move to trash |
| F11 | Full screen |

Right-click in the preview for the file menu: Open With, Show in Folder, Set as Wallpaper.

## Install

On Arch, Fedora, Ubuntu 26.04 or newer, openSUSE Tumbleweed and distributions based on them (Qt 6.10 or newer; Debian 13 has Qt 6.8 and is not supported):

```bash
curl -fsSL https://raw.githubusercontent.com/LaskyWolf/quickpeek/main/install.sh | bash
```

The script installs Rust, a C++ compiler and the Qt and KDE packages it needs with your package manager (it asks for your password through `sudo`). It then builds KDEQuickPeek from source against your distribution's Qt and puts it in `~/.local`. It needs Qt 6.10 or newer. If your distribution's Rust is older than 1.92, it offers to install rustup into your home folder.

The first build takes a few minutes. Run the same command again to update: it builds the latest version. To uninstall:

```bash
curl -fsSL https://raw.githubusercontent.com/LaskyWolf/quickpeek/main/install.sh | bash -s -- --uninstall
```

On Arch or CachyOS you can build a package instead, from a clone of this repository:

```bash
makepkg -si
```

Both ways install `quickpeek`, `quickpeek-window`, `quickpeek-desktop`, the "Quick Preview" entry in Dolphin's right-click menu and a "KDEQuickPeek" entry in the application menu. The menu entry only opens a short window that says it works: previews start from Dolphin.

Some previews use other programs when they are installed: ffmpeg (video and audio), poppler (PDF details), 7zip (more archive formats), assimp and blender (3D models), krita, onlyoffice (Office files and presentations). See `optdepends` in `PKGBUILD` for the full list.

To make Space open quickpeek on desktop icons and in Folder View widgets:

```bash
quickpeek-desktop install
systemctl --user restart plasma-plasmashell
```

Run `quickpeek-desktop install` again after every Plasma update.

AI models download on first use into `~/.local/share/quickpeek/models`.

## Tech stack

- Rust with cxx-qt (Qt 6 bindings), QML screens: Qt Quick 3D, Qt Multimedia, Qt PDF, Kirigami.
- Optional: ONNX Runtime (AI tools), loaded at run time.
- Optional programs: ffmpeg, OnlyOffice, assimp, Blender, Krita, poppler, 7zip.

## Project structure

```
rust/                 the app: `main.rs` is the `quickpeek` launcher, `window.rs` the window
qml/                  the screens, one file per previewer
cpp/                  small C++ helpers for Qt APIs
launcher/             Cargo package of the `quickpeek` launcher
rust/desktop_patch.rs `quickpeek-desktop`: Space on Plasma desktop icons
quickpeek.desktop     Dolphin right-click entry
quickpeek-app.desktop application menu entry
quickpeek.svg         icon
install.sh            curl installer
PKGBUILD              Arch package
docs/                 notes
```

## Tests

```bash
cargo test --workspace
```

## License

GPL-3.0-or-later. See `LICENSE`.
