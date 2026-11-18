# Personal package for KDEQuickPeek. Build and install from this folder: makepkg -si
pkgname=quickpeek
pkgver=r80.3c93be1
pkgrel=1
pkgdesc="KDEQuickPeek: quick file previewer for KDE Plasma (Space to preview)"
arch=(x86_64)
makedepends=(cargo)
options=(!lto)  # GCC LTO objects from the C++ helpers are unreadable for the Rust linker; Cargo.toml does LTO
license=(GPL-3.0-or-later)
depends=(python pyside6 qt6-declarative qt6-multimedia qt6-quick3d qqc2-desktop-style kirigami glib2)
optdepends=(
  'python-pymupdf: PDF editor'
  'python-numpy: subject picking and finding text in images'
  'onnxruntime-cpu: AI tools in the new window (subject picking, finding text in images)'
  'python-onnxruntime-cpu: AI tools (subject picking, finding text in images)'
  'python-scipy: cleaner subject cut-outs, finding text in images'
  'python-pillow: more image formats'
  'python-openpyxl: spreadsheet preview'
  'python-pygments: code highlighting'
  'ffmpeg: video thumbnails, conversions and audio waveforms'
  'onlyoffice-bin: Word, Excel, PowerPoint and OpenDocument pages, presentations with Morph (AUR)'
  'assimp: converting 3D models to other formats'
  'blender: 3D preview of .blend files'
  'krita: full-size preview of Krita archives (.krz)'
  'poppler: PDF thumbnails'
  'ghostscript: PostScript preview'
  '7zip: more archive formats'
  'libarchive: archive preview'
  'waifu2x-ncnn-vulkan: image enlarging'
  'plasma-workspace: set as wallpaper'
  'dolphin: show in folder'
)
source=(quickpeek-desktop.py quickpeek.desktop quickpeek-app.desktop quickpeek.svg)
sha256sums=(SKIP SKIP SKIP SKIP)

pkgver() {
  cd "$startdir"
  printf "r%s.%s" "$(git rev-list --count HEAD)" "$(git rev-parse --short HEAD)"
}

build() {
  cd "$startdir"
  QUICKPEEK_PY=/usr/lib/quickpeek/quickpeek.py cargo build --release --locked --workspace --target-dir "$srcdir/target"
}

package() {
  install -Dm755 "$srcdir/target/release/quickpeek" "$pkgdir/usr/bin/quickpeek"
  install -Dm755 "$srcdir/target/release/quickpeek-window" "$pkgdir/usr/bin/quickpeek-window"
  install -Dm644 "$startdir/quickpeek.py" "$pkgdir/usr/lib/quickpeek/quickpeek.py"
  install -Dm755 quickpeek-desktop.py "$pkgdir/usr/bin/quickpeek-desktop"
  install -Dm644 quickpeek.desktop "$pkgdir/usr/share/kio/servicemenus/quickpeek.desktop"
  install -Dm644 quickpeek-app.desktop "$pkgdir/usr/share/applications/quickpeek.desktop"
  install -Dm644 quickpeek.svg "$pkgdir/usr/share/icons/hicolor/scalable/apps/quickpeek.svg"
}
