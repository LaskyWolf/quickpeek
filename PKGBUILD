# Personal package for peek. Build and install from this folder: makepkg -si
pkgname=peek
pkgver=r11.9f11831
pkgrel=1
pkgdesc="Quick file previewer for KDE Plasma (Space to preview)"
arch=(any)
license=(custom)
depends=(python pyside6 qt6-declarative qt6-multimedia qt6-quick3d qt6-webengine glib2)
optdepends=(
  'python-pymupdf: PDF editor'
  'python-numpy: subject picking and background removal'
  'python-onnxruntime-cpu: AI tools (subject picking, background removal)'
  'python-scipy: cleaner subject cut-outs'
  'python-pillow: more image formats'
  'python-openpyxl: spreadsheet preview'
  'python-pygments: code highlighting'
  'ffmpeg: video thumbnails and conversions'
  'tesseract: text in images (OCR)'
  'poppler: PDF thumbnails'
  'ghostscript: PostScript preview'
  '7zip: more archive formats'
  'libarchive: archive preview'
  'waifu2x-ncnn-vulkan: image enlarging'
  'plasma-workspace: set as wallpaper'
  'dolphin: show in folder'
)
source=(peek.py peek-desktop.py peek.desktop)
sha256sums=(SKIP SKIP SKIP)

pkgver() {
  cd "$startdir"
  printf "r%s.%s" "$(git rev-list --count HEAD)" "$(git rev-parse --short HEAD)"
}

package() {
  install -Dm755 peek.py "$pkgdir/usr/bin/peek"
  install -Dm755 peek-desktop.py "$pkgdir/usr/bin/peek-desktop"
  install -Dm644 peek.desktop "$pkgdir/usr/share/kio/servicemenus/peek.desktop"
}
