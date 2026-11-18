#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# KDEQuickPeek installer, for KDE Plasma on Arch, Fedora, Ubuntu (26.04+) and openSUSE families. Needs Qt 6.10+.
#
#   Install or update:  curl -fsSL https://raw.githubusercontent.com/LaskyWolf/quickpeek/main/install.sh | bash
#   Uninstall:          curl -fsSL https://raw.githubusercontent.com/LaskyWolf/quickpeek/main/install.sh | bash -s -- --uninstall
#
# Builds KDEQuickPeek from source against your distribution's Qt and installs it into your home
# folder only (~/.local). The system packages it needs come from your distribution and are
# installed with sudo, after your package manager asks you to confirm.
set -euo pipefail

REPO="${QUICKPEEK_REPO:-https://github.com/LaskyWolf/quickpeek}"
REF="${QUICKPEEK_REF:-main}"
SRC="${QUICKPEEK_SRC:-}"  # a local checkout instead of downloading (for testing)
MIN_QT=6.10  # 6.10.2 is the oldest Qt tested; 6.8 lacks Flickable.acceptedButtons (qml/PdfReader.qml)
MIN_RUST=1.92  # the newest rust-version among the locked crates (ort)
BIN="$HOME/.local/bin"
MENU="$HOME/.local/share/kio/servicemenus"
APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor/scalable/apps"
FILES=("$BIN/quickpeek" "$BIN/quickpeek-window" "$BIN/quickpeek-desktop" "$MENU/quickpeek.desktop"
       "$APPS/quickpeek.desktop" "$ICONS/quickpeek.svg")
# Left by the Python version of this installer
OLD_APP="$HOME/.local/share/quickpeek/app"
OLD_VENV="$HOME/.local/share/quickpeek/venv"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
fail() { printf '\033[31mError: %s\033[0m\n' "$*" >&2; exit 1; }

# Root in a container needs no sudo; QUICKPEEK_ASSUME_YES=1 answers the package manager's question
as_root() { if [ "$(id -u)" -eq 0 ]; then "$@"; else sudo "$@"; fi; }
YES="${QUICKPEEK_ASSUME_YES:-}"
quit_running() { if [ -x "$BIN/quickpeek" ]; then QT_QPA_PLATFORM=offscreen "$BIN/quickpeek" --quit >/dev/null 2>&1 || true; fi; }
ask_from_tty() { if [ -z "$YES" ] && [ -r /dev/tty ]; then "$@" </dev/tty; else "$@"; fi; }
version_ge() { [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n 1)" = "$2" ]; }

uninstall() {
    quit_running
    if [ -x "$BIN/quickpeek-desktop" ]; then "$BIN/quickpeek-desktop" uninstall >/dev/null 2>&1 || true; fi
    rm -f "${FILES[@]}"
    rm -rf "$OLD_APP" "$OLD_VENV"
    say "Removed. Settings and downloaded AI models stay in ~/.config/quickpeek and ~/.local/share/quickpeek."
    exit 0
}
[ "${1:-}" = "--uninstall" ] && uninstall

[ -r /etc/os-release ] || fail "can't tell which Linux distribution this is (/etc/os-release is missing)."
# shellcheck source=/dev/null
. /etc/os-release
family="$ID ${ID_LIKE:-}"
export PATH="$HOME/.cargo/bin:$PATH"  # rustup's toolchain, if there is one
rust=()  # the distribution's Rust, only when there is no cargo yet (rustup provides one too)

# 1. System packages: compilers, Qt development files, and the Qt/KDE modules the previews use
install_packages() {
    case "$family" in
        *arch*)
            command -v cargo >/dev/null || rust=(rust)
            pkgs=("${rust[@]}" gcc qt6-base qt6-declarative qt6-multimedia qt6-quick3d qt6-webengine qt6-svg
                  qt6-imageformats libarchive kirigami qqc2-desktop-style)  # qt6-webengine ships Qt PDF
            say "Installing with pacman: ${pkgs[*]}"
            ask_from_tty as_root pacman -S --needed ${YES:+--noconfirm} "${pkgs[@]}" ;;
        *fedora*)
            command -v cargo >/dev/null || rust=(rust cargo)
            pkgs=("${rust[@]}" gcc-c++ qt6-qtbase-devel qt6-qtdeclarative-devel qt6-qtpdf-devel libarchive-devel
                  qt6-qtmultimedia qt6-qtquick3d qt6-qtsvg qt6-qtimageformats kf6-kirigami kf6-qqc2-desktop-style)
            say "Installing with dnf: ${pkgs[*]}"
            ask_from_tty as_root dnf install ${YES:+-y} "${pkgs[@]}" ;;
        *suse*)
            command -v cargo >/dev/null || rust=(rust cargo)
            pkgs=("${rust[@]}" gcc-c++ qt6-base-common-devel qt6-widgets-devel qt6-dbus-devel qt6-network-devel
                  qt6-qml-devel qt6-quick-devel qt6-quickcontrols2-devel qt6-pdf-devel libarchive-devel
                  qt6-declarative-imports qt6-multimedia-imports qt6-quick3d-imports qt6-pdf-imports libQt6Svg6
                  qt6-imageformats kf6-kirigami-imports kf6-qqc2-desktop-style)
            say "Installing with zypper: ${pkgs[*]}"
            ask_from_tty as_root zypper ${YES:+--non-interactive} install "${pkgs[@]}" ;;
        *debian*|*ubuntu*)
            command -v cargo >/dev/null || rust=(rustc cargo)
            pkgs=("${rust[@]}" g++ qmake6 qmake6-bin qt6-base-dev qt6-base-dev-tools qt6-declarative-dev
                  qt6-declarative-dev-tools qt6-pdf-dev libarchive-dev qml6-module-qtqml qml6-module-qtcore
                  qml6-module-qtquick qml6-module-qtquick-controls qml6-module-qtquick-layouts
                  qml6-module-qtquick-window qml6-module-qtquick-dialogs qml6-module-qtquick-shapes
                  qml6-module-qtquick-pdf qml6-module-qtmultimedia qml6-module-qtquick3d
                  qml6-module-qtquick3d-helpers qml6-module-qtquick3d-assetutils qt6-svg-plugins
                  qt6-image-formats-plugins qml6-module-org-kde-kirigami qml6-module-org-kde-desktop)
            say "Installing with apt: ${pkgs[*]}"
            as_root apt-get update -qq
            # Some releases split or merge packages (qmake6 vs qmake6-bin): install the ones this release has
            available=()
            for p in "${pkgs[@]}"; do apt-cache show "$p" >/dev/null 2>&1 && available+=("$p"); done
            ask_from_tty as_root apt-get install ${YES:+-y} "${available[@]}" ;;
        *)
            say "Unknown distribution ($ID): skipping system packages. Install Rust, a C++ compiler, Qt $MIN_QT or newer with its development files, and libarchive yourself." ;;
    esac
}

# 2. Qt: cxx-qt finds it through qmake
check_qt() {
    if [ -z "${QMAKE:-}" ]; then
        for q in qmake6 /usr/lib/qt6/bin/qmake /usr/lib64/qt6/bin/qmake; do
            if command -v "$q" >/dev/null; then QMAKE=$(command -v "$q"); break; fi
        done
    fi
    [ -n "${QMAKE:-}" ] || fail "qmake6 not found. Install your distribution's Qt 6 development package (qt6-base-dev, qt6-qtbase-devel or qt6-base-common-devel), or set QMAKE=/path/to/qmake6."
    export QMAKE
    qt=$("$QMAKE" -query QT_VERSION)
    version_ge "$qt" "$MIN_QT" || fail "your Qt is $qt; KDEQuickPeek needs Qt $MIN_QT or newer. Update your distribution, or use a newer release of it."
    say "Using Qt $qt ($QMAKE)."
}

# 3. Rust: the distribution's, unless it is older than the crates need; then rustup, after asking
check_rust() {
    have=$(rustc --version 2>/dev/null | cut -d' ' -f2 || true)
    if [ -n "$have" ] && command -v cargo >/dev/null && version_ge "$have" "$MIN_RUST"; then
        say "Using Rust $have."
        return
    fi
    if command -v rustup >/dev/null; then
        say "Your Rust is ${have:-missing}; KDEQuickPeek needs $MIN_RUST or newer. Updating rustup's stable toolchain."
        rustup toolchain install stable --profile minimal
        export RUSTUP_TOOLCHAIN=stable
        return
    fi
    cat <<EOF

Your Rust is ${have:-missing}; KDEQuickPeek needs Rust $MIN_RUST or newer to build.
The installer can install rustup (https://rustup.rs), the official Rust installer, into ~/.cargo
and ~/.rustup in your home folder. Your system's Rust stays as it is.
Remove it later with: rustup self uninstall

EOF
    if [ -z "$YES" ]; then
        [ -r /dev/tty ] || fail "can't ask for confirmation without a terminal. Run again with QUICKPEEK_ASSUME_YES=1, or install Rust $MIN_RUST or newer yourself."
        printf 'Install rustup into your home folder? [y/N] '
        read -r answer </dev/tty
        case "$answer" in [yY]*) ;; *) fail "Rust $MIN_RUST or newer is needed. Install it, then run this installer again." ;; esac
    fi
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path \
        || fail "rustup failed to install; the lines above say why."
    hash -r  # forget where the old rustc was
    say "Installed Rust $(rustc --version | cut -d' ' -f2) with rustup."
}

install_packages
check_qt
check_rust

# 4. The source, built in the cache folder: the build needs a few GB, more than a RAM-backed /tmp may have
cache="${XDG_CACHE_HOME:-$HOME/.cache}"
mkdir -p "$cache"
work=$(mktemp -d "$cache/quickpeek-build.XXXXXX")
trap 'rm -rf "$work"' EXIT
if [ -n "$SRC" ]; then
    [ -f "$SRC/Cargo.toml" ] || fail "QUICKPEEK_SRC=$SRC is not a KDEQuickPeek checkout (no Cargo.toml)."
    mkdir "$work/src"
    tar -C "$SRC" --exclude=./target --exclude=./.git -cf - . | tar -xf - -C "$work/src"
else
    say "Downloading KDEQuickPeek ($REF)"
    mkdir "$work/src"
    curl -fsSL "$REPO/archive/$REF.tar.gz" | tar -xz -C "$work/src" --strip-components=1 \
        || fail "can't download $REPO/archive/$REF.tar.gz. Check your internet connection."
fi

say "Building KDEQuickPeek. The first build takes a few minutes."
(cd "$work/src" && cargo build --release --locked --workspace --target-dir "$work/target") \
    || fail "the build failed; the lines above say why. Report it at $REPO/issues with your distribution and Qt version ($qt)."

# 5. Install, replacing a running window and the Python version's leftovers
quit_running
rm -rf "$OLD_APP" "$OLD_VENV"
install -Dm755 "$work/target/release/quickpeek" "$BIN/quickpeek"
install -Dm755 "$work/target/release/quickpeek-window" "$BIN/quickpeek-window"
install -Dm755 "$work/src/quickpeek-desktop.py" "$BIN/quickpeek-desktop"
# Dolphin's right-click entry, with the full path: Dolphin's PATH may lack ~/.local/bin.
# Plasma 6 only loads executable service menus from the home folder.
mkdir -p "$MENU" "$APPS" "$ICONS"
sed "s|^Exec=quickpeek |Exec=$BIN/quickpeek |" "$work/src/quickpeek.desktop" > "$MENU/quickpeek.desktop"
chmod 755 "$MENU/quickpeek.desktop"
# The entry in the application menu, with its icon
sed "s|^Exec=quickpeek|Exec=$BIN/quickpeek|" "$work/src/quickpeek-app.desktop" > "$APPS/quickpeek.desktop"
install -Dm644 "$work/src/quickpeek.svg" "$ICONS/quickpeek.svg"

say "Installed KDEQuickPeek:"
printf '    %s\n' "${FILES[@]}"
case ":$PATH:" in *":$BIN:"*) ;; *) echo "Add $BIN to your PATH to run 'quickpeek' from a terminal." ;; esac
cat <<EOF

Open a file: right-click it in Dolphin → Quick Preview, or run: quickpeek FILE
Space on desktop icons (optional, needs python3):  quickpeek-desktop install && systemctl --user restart plasma-plasmashell
More previews, if you install them with your package manager: ffmpeg (video, audio), poppler (PDF
details), 7zip (archives), assimp, blender, krita, onlyoffice (Office files and presentations).
Update: run the same curl command again.
EOF
