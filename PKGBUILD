# Personal package for KDEQuickPeek. Build and install from this folder: makepkg -si
pkgname=quickpeek
pkgver=1.0.0
pkgrel=1
pkgdesc="KDEQuickPeek: quick file previewer for KDE Plasma (Space to preview)"
arch=(x86_64)
makedepends=(cargo)
options=(!lto)  # GCC LTO objects from the C++ helpers are unreadable for the Rust linker; Cargo.toml does LTO
license=(GPL-3.0-or-later)
depends=(qt6-base qt6-declarative qt6-multimedia qt6-quick3d qt6-webengine qt6-svg qt6-imageformats
         libarchive kirigami qqc2-desktop-style)  # qt6-webengine ships Qt PDF
optdepends=(
  'onnxruntime-cpu: AI tools (subject picking, finding text in images)'
  'kimageformats: PSD, XCF, AVIF, HEIF and other picture formats'
  'ffmpeg: video thumbnails, conversions and audio waveforms'
  'onlyoffice-bin: Word, Excel, PowerPoint and OpenDocument pages, presentations with Morph (AUR)'
  'assimp: converting 3D models to other formats'
  'blender: 3D preview of .blend files'
  'krita: full-size preview of Krita archives (.krz)'
  'poppler: PDF details and PDF to pictures'
  'ghostscript: PostScript preview'
  '7zip: more archive formats'
  'waifu2x-ncnn-vulkan: image enlarging'
  'plasma-workspace: set as wallpaper'
  'dolphin: show in folder'
)
source=(quickpeek.desktop quickpeek-app.desktop quickpeek.svg)
sha256sums=(SKIP SKIP SKIP)

build() {
  cd "$startdir"
  cargo build --release --locked --workspace --target-dir "$srcdir/target"
}

package() {
  install -Dm755 "$srcdir/target/release/quickpeek" "$pkgdir/usr/bin/quickpeek"
  install -Dm755 "$srcdir/target/release/quickpeek-window" "$pkgdir/usr/bin/quickpeek-window"
  install -Dm755 "$srcdir/target/release/quickpeek-desktop" "$pkgdir/usr/bin/quickpeek-desktop"
  install -Dm644 quickpeek.desktop "$pkgdir/usr/share/kio/servicemenus/quickpeek.desktop"
  install -Dm644 quickpeek-app.desktop "$pkgdir/usr/share/applications/quickpeek.desktop"
  install -Dm644 quickpeek.svg "$pkgdir/usr/share/icons/hicolor/scalable/apps/quickpeek.svg"
}
