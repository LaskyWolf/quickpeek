# Personal package for KDEQuickPeek. Build and install from this folder: makepkg -si
pkgname=quickpeek
pkgver=r29.ab7fc0a
pkgrel=1
pkgdesc="KDEQuickPeek: quick file previewer for KDE Plasma (Space to preview)"
arch=(any)
license=(custom)
depends=(python pyside6 qt6-declarative qt6-multimedia qt6-quick3d qt6-webengine glib2)
optdepends=(
  'python-pymupdf: PDF editor'
  'python-numpy: subject picking and finding text in images'
  'python-onnxruntime-cpu: AI tools (subject picking, finding text in images)'
  'python-scipy: cleaner subject cut-outs, finding text in images'
  'python-pillow: more image formats'
  'python-openpyxl: spreadsheet preview'
  'python-pygments: code highlighting'
  'ffmpeg: video thumbnails, conversions and audio waveforms'
  'krita: full-size preview of Krita archives (.krz)'
  'poppler: PDF thumbnails'
  'ghostscript: PostScript preview'
  '7zip: more archive formats'
  'libarchive: archive preview'
  'waifu2x-ncnn-vulkan: image enlarging'
  'plasma-workspace: set as wallpaper'
  'dolphin: show in folder'
)
source=(quickpeek.py quickpeek-desktop.py quickpeek.desktop)
sha256sums=(SKIP SKIP SKIP)

pkgver() {
  cd "$startdir"
  printf "r%s.%s" "$(git rev-list --count HEAD)" "$(git rev-parse --short HEAD)"
}

package() {
  install -Dm755 quickpeek.py "$pkgdir/usr/bin/quickpeek"
  install -Dm755 quickpeek-desktop.py "$pkgdir/usr/bin/quickpeek-desktop"
  install -Dm644 quickpeek.desktop "$pkgdir/usr/share/kio/servicemenus/quickpeek.desktop"
}
