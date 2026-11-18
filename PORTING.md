# Rust port

QuickPeek is Rust + QML (cxx-qt 0.10, Qt 6). Two binaries:

- `quickpeek` (`rust/main.rs`, package `launcher/`) hands the file to the running window without loading Qt, or starts the window.
- `quickpeek-window` (`rust/window.rs`) is the window.

## Layout

- `quickpeek-window FILE --snapshot=OUT.png` saves the window after 1.5 s and quits (add `--side` to open the side panel): run it with `QT_QPA_PLATFORM=offscreen` to check a screen without opening a window.

- `rust/` Rust sources. `rust/app.rs` is the window's backend object; `rust/files.rs` lists folders in Dolphin's order.
- `qml/` the screens. `Main.qml` is the window; each previewer is one `qml/<Kind>View.qml`.
- `cpp/` small C++ helpers for Qt APIs cxx-qt-lib doesn't wrap (QMimeDatabase, QCollator, QtDBus).
- Previews follow the "load the cheap thing first, swap in the full thing" pattern.

## Checklist

Shell
- [x] Background window, socket server, `--quit`, Space on the same file closes, activation token
- [x] Files in Dolphin's order (live D-Bus, saved view settings, natural name key)
- [x] Bottom bar: drag icon, name, type, size, position, Open button
- [x] Keys: Space/Q/Esc close, Return open, arrows, Ctrl+C, Ctrl+Shift+C, F2, Del, F11
- [x] Key I: side panel
- [x] Previous/next arrows on the preview edges
- [x] Right-click menu: Open, Open With, Show in Folder, Copy, Copy Path, Rename, Trash, Wallpaper, Full Screen
- [x] Window size and volume remembered
- [x] Side panel: file details per type, tools page
- [x] Cache folder and pruning (`rust/convert.rs`); `move_old_files` dropped

Previewers
- [x] Images: zoom/pan, animated GIF/WebP, SVG, preload neighbours
- [x] Images: Krita/ORA/XCF/PSD/Clip, Blender/Revit/Affinity previews, thumbnails of other files
- [x] Images: AI tools toolbar
- [x] Plain text
- [x] Code highlighting, Markdown/HTML, search popup (Ctrl+F)
- [x] Folders, archives (libarchive, 7z)
- [x] Fonts
- [x] Audio with waveform, video with seek and speed
- [x] PDF reading, search, text selection
- [x] Spreadsheets (xlsx/ods/csv)
- [x] Office pages via ONLYOFFICE x2t, Word view
- [x] Slides with PowerPoint Morph
- [x] 3D models (balsam/assimp/Blender), fly mode
- [x] PostScript (ghostscript)

Tools
- [x] PDF editor: pressure pen, text boxes, arrows, highlighter, save annotations
- [x] File conversions (ffmpeg, assimp, x2t, images)
- [x] AI: one runtime (`ort`) and one downloader for subject cut-out (SAM2), background matte, OCR (PaddleOCR)
- [x] Image enlarging (waifu2x)

Release
- [x] `install.sh` builds from source
- [x] README updated
