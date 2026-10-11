#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# KDEQuickPeek installer, for KDE Plasma on Arch, Fedora, Debian and openSUSE families.
#
#   Install or update:  curl -fsSL https://raw.githubusercontent.com/LaskyWolf/quickpeek/main/install.sh | bash
#   Uninstall:          curl -fsSL https://raw.githubusercontent.com/LaskyWolf/quickpeek/main/install.sh | bash -s -- --uninstall
#
# Installs into your home folder only (~/.local). The system packages it needs come from your
# distribution and are installed with sudo, after your package manager asks you to confirm.
set -euo pipefail

REPO="${QUICKPEEK_REPO:-https://github.com/LaskyWolf/quickpeek}"
REF="${QUICKPEEK_REF:-main}"
SOURCE="${QUICKPEEK_SOURCE:-}"  # a local checkout instead of downloading (for testing)
BIN="$HOME/.local/bin"
APP="$HOME/.local/share/quickpeek/app"
VENV="$HOME/.local/share/quickpeek/venv"
MENU="$HOME/.local/share/kio/servicemenus"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
fail() { printf '\033[31mError: %s\033[0m\n' "$*" >&2; exit 1; }

# Root in a container needs no sudo; QUICKPEEK_ASSUME_YES=1 answers the package manager's question
as_root() { if [ "$(id -u)" -eq 0 ]; then "$@"; else sudo "$@"; fi; }
YES="${QUICKPEEK_ASSUME_YES:-}"
quit_running() { if [ -x "$BIN/quickpeek" ]; then QT_QPA_PLATFORM=offscreen "$BIN/quickpeek" --quit >/dev/null 2>&1 || true; fi; }
ask_from_tty() { if [ -z "$YES" ] && [ -r /dev/tty ]; then "$@" </dev/tty; else "$@"; fi; }

uninstall() {
    quit_running
    if [ -x "$BIN/quickpeek-desktop" ]; then "$BIN/quickpeek-desktop" uninstall >/dev/null 2>&1 || true; fi
    rm -f "$BIN/quickpeek" "$BIN/quickpeek-desktop" "$MENU/quickpeek.desktop"
    rm -rf "$HOME/.local/share/quickpeek/app" "$VENV"
    say "Removed. Settings and downloaded AI models stay in ~/.config/quickpeek and ~/.local/share/quickpeek."
    exit 0
}
[ "${1:-}" = "--uninstall" ] && uninstall

[ -r /etc/os-release ] || fail "can't tell which Linux distribution this is (/etc/os-release is missing)."
. /etc/os-release
family="$ID ${ID_LIKE:-}"

# 1. System packages: Python, PySide6 and the Qt modules the previews use
install_packages() {
    case "$family" in
        *arch*)
            pkgs=(python pyside6 qt6-declarative qt6-multimedia qt6-quick3d qt6-wayland glib2)
            say "Installing with pacman: ${pkgs[*]}"
            ask_from_tty as_root pacman -S --needed ${YES:+--noconfirm} "${pkgs[@]}" ;;
        *fedora*)
            pkgs=(python3-pyside6 qt6-qtdeclarative qt6-qtmultimedia qt6-qtquick3d qt6-qtwayland qt6-qtpdf gstreamer1-plugins-good glib2)
            say "Installing with dnf: ${pkgs[*]}"
            ask_from_tty as_root dnf install ${YES:+-y} "${pkgs[@]}" ;;
        *suse*)
            py=$(python3 -c 'import sys; print(f"python{sys.version_info[0]}{sys.version_info[1]}")' 2>/dev/null || echo python3)
            pkgs=("$py-pyside6" qt6-declarative-imports qt6-multimedia-imports qt6-quick3d-imports gstreamer-plugins-good glib2-tools)
            say "Installing with zypper: ${pkgs[*]}"
            ask_from_tty as_root zypper ${YES:+--non-interactive} install "${pkgs[@]}" ;;
        *debian*|*ubuntu*)
            pkgs=(python3 python3-venv python3-pyside6.qtwidgets python3-pyside6.qtquick python3-pyside6.qtqml
                  python3-pyside6.qtmultimedia python3-pyside6.qtmultimediawidgets python3-pyside6.qtpdf
                  python3-pyside6.qtpdfwidgets python3-pyside6.qtnetwork qml6-module-qtquick3d
                  qml6-module-qtquick3d-helpers qml6-module-qtquick3d-assetutils qml6-module-qtquick
                  qt6-wayland libglib2.0-bin)
            say "Installing with apt: ${pkgs[*]}"
            as_root apt-get update -qq
            # Packages a release doesn't have are skipped; the check below decides if Python can do without them
            available=()
            for p in "${pkgs[@]}"; do apt-cache show "$p" >/dev/null 2>&1 && available+=("$p"); done
            ask_from_tty as_root apt-get install ${YES:+-y} "${available[@]}" ;;
        *)
            say "Unknown distribution ($ID): skipping system packages." ;;
    esac
}

# 2. Can this Python run quickpeek? Imports every Qt module it uses and loads the 3D QML modules.
check_python() {
    QT_QPA_PLATFORM=offscreen "$1" - <<'EOF' >/dev/null 2>"$work/check.log"
import sys
assert sys.version_info >= (3, 11)
from PySide6.QtCore import QUrl
from PySide6.QtWidgets import QApplication
from PySide6 import QtMultimedia, QtMultimediaWidgets, QtPdf, QtPdfWidgets, QtNetwork, QtQuick
from PySide6.QtQml import QQmlComponent, QQmlEngine
app = QApplication([])
engine = QQmlEngine()
component = QQmlComponent(engine)
component.setData(b"import QtQuick\nimport QtQuick3D\nimport QtQuick3D.Helpers\nimport QtQuick3D.AssetUtils\nItem {}", QUrl())
assert component.status() == QQmlComponent.Ready, component.errorString()
EOF
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
install_packages
PYTHON=$(command -v python3 || true)
if [ -n "$PYTHON" ] && check_python "$PYTHON"; then
    say "Using your system's PySide6 ($PYTHON)."
else
    # The distribution lacks a part: PySide6 from PyPI brings its own Qt (about 600 MB, plain Qt look)
    [ -n "$PYTHON" ] || fail "python3 is not installed."
    tail -n 1 "$work/check.log"
    say "Your distribution's PySide6 is missing a part; installing PySide6 from PyPI into $VENV"
    "$PYTHON" -m venv "$VENV" || fail "can't create a Python environment: install your distribution's python3-venv package."
    "$VENV/bin/pip" install --quiet --upgrade pip
    "$VENV/bin/pip" install --quiet PySide6
    PYTHON="$VENV/bin/python"
    check_python "$PYTHON" || { tail -n 1 "$work/check.log"; fail "PySide6 from PyPI does not start either; the line above says what is missing."; }
fi

# 3. The app itself
if [ -n "$SOURCE" ]; then
    cp "$SOURCE"/quickpeek.py "$SOURCE"/quickpeek-desktop.py "$SOURCE"/quickpeek.desktop "$SOURCE"/LICENSE "$work"/
else
    say "Downloading KDEQuickPeek ($REF)"
    curl -fsSL "$REPO/archive/$REF.tar.gz" | tar -xz -C "$work" --strip-components=1
fi
mkdir -p "$BIN" "$APP" "$MENU"
cp "$work"/quickpeek.py "$work"/quickpeek-desktop.py "$work"/LICENSE "$APP"/
for name in quickpeek quickpeek-desktop; do
    printf '#!/bin/sh\nexec "%s" "%s/%s.py" "$@"\n' "$PYTHON" "$APP" "$name" > "$BIN/$name"
    chmod +x "$BIN/$name"
done
# Dolphin's right-click entry; Plasma 6 only loads executable service menus from the home folder
sed "s|^Exec=quickpeek |Exec=$BIN/quickpeek |" "$work"/quickpeek.desktop > "$MENU/quickpeek.desktop"
chmod +x "$MENU/quickpeek.desktop"

quit_running  # an older version still running in the background
say "Installed KDEQuickPeek."
case ":$PATH:" in *":$BIN:"*) ;; *) echo "Add $BIN to your PATH to run 'quickpeek' from a terminal." ;; esac
cat <<EOF

Open a file: right-click it in Dolphin → Quick Preview, or run: quickpeek FILE
Space on desktop icons (optional):  quickpeek-desktop install && systemctl --user restart plasma-plasmashell
More previews, if you install them with your package manager: ffmpeg (video, audio), poppler (PDF
thumbnails), 7zip/libarchive (archives), assimp, blender, krita, onlyoffice (Office files and presentations).
Update: run the same curl command again.
EOF
