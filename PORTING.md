# Rust port

QuickPeek is moving from Python (`quickpeek.py`) to Rust + QML (cxx-qt 0.10, Qt 6).
Until every item below is ported, both apps ship:

- `quickpeek` (Rust, `rust/main.rs`, package `launcher/`) hands the file to the running window without loading Qt.
- `quickpeek-window` (Rust + QML, `rust/window.rs`) is the new window. Try it: `QUICKPEEK_NEXT=1 quickpeek FILE` after `quickpeek --quit`.
- With no window running, it starts `quickpeek-window` when `QUICKPEEK_NEXT=1` is set, else the Python app.
- The switch flips to Rust by default when the checklist is done; then `quickpeek.py` is deleted.

## Layout

- `quickpeek-window FILE --snapshot=OUT.png` saves the window after 1.5 s and quits: run it with `QT_QPA_PLATFORM=offscreen` to check a screen without opening a window.

- `rust/` Rust sources. `rust/app.rs` is the window's backend object; `rust/files.rs` lists folders in Dolphin's order.
- `qml/` the screens. `Main.qml` is the window; each previewer is one `qml/<Kind>View.qml`.
- `cpp/` small C++ helpers for Qt APIs cxx-qt-lib doesn't wrap (QMimeDatabase, QCollator, QtDBus).
- Previews follow the "load the cheap thing first, swap in the full thing" pattern of the Python app.

## Checklist

Shell
- [x] Background window, socket server, `--quit`, Space on the same file closes, activation token
- [x] Files in Dolphin's order (live D-Bus, saved view settings, natural name key)
- [x] Bottom bar: drag icon, name, type, size, position, Open button
- [x] Keys: Space/Q/Esc close, Return open, arrows, Ctrl+C, Ctrl+Shift+C, F2, Del, F11
- [ ] Key I: side panel
- [x] Previous/next arrows on the preview edges
- [ ] Right-click menu: Open, Open With, Show in Folder, Copy, Copy Path, Rename, Trash, Wallpaper, Full Screen
- [ ] Window size and volume remembered
- [ ] Side panel: file details per type
- [ ] Cache folder, pruning, `move_old_files` (drop)

Previewers
- [x] Images: zoom/pan, animated GIF/WebP, SVG, preload neighbours
- [ ] Images: Krita/ORA/XCF/PSD/Clip, thumbnails of other files, AI tools toolbar
- [x] Plain text
- [x] Code highlighting, Markdown/HTML, search popup (Ctrl+F)
- [x] Folders, archives (libarchive, 7z)
- [x] Fonts
- [x] Audio with waveform, video with seek and speed
- [x] PDF reading, search, text selection
- [x] Spreadsheets (xlsx/ods/csv)
- [ ] Office pages via ONLYOFFICE x2t, Word view
- [ ] Slides with PowerPoint Morph
- [ ] 3D models (balsam/assimp/Blender), fly mode
- [ ] PostScript (ghostscript)

Tools
- [ ] PDF editor: pressure pen, text boxes, arrows, signing, save annotations
- [ ] File conversions (ffmpeg, assimp, x2t, images)
- [ ] AI: one runtime (`ort`) and one downloader for subject cut-out (SAM2), background matte, OCR (PaddleOCR)
- [ ] Image enlarging (waifu2x)

Release
- [ ] Rust app is the default, `quickpeek.py` and its tests deleted
- [ ] `install.sh` downloads a prebuilt binary from the GitHub release
- [ ] README updated
