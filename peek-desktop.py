#!/usr/bin/env python3
"""Make Space on a selected Plasma desktop icon open `peek`.

Plasma loads a desktop package from ~/.local/share before /usr/share, so this installs a
patched copy of the system desktop package there. The copy does not follow Plasma updates:
run `peek-desktop install` again after every Plasma update.

Usage: peek-desktop install | uninstall
"""
import shutil
import subprocess
import sys
from pathlib import Path

PACKAGE = "org.kde.desktopcontainment"
SYSTEM = Path("/usr/share/plasma/plasmoids") / PACKAGE
LOCAL = Path.home() / ".local/share/plasma/plasmoids" / PACKAGE
MARKER = LOCAL / ".peek-patch"  # proves the local copy is ours before we ever delete it

PATCHES = [
    # 1. Plasma's module for running shell commands from QML
    ("import QtQml\n",
     "import QtQml\nimport org.kde.plasma.plasma5support as P5Support\n"),
    # 2. The command runner, next to the view's other helper objects
    ("                Folder.ShortCut {\n",
     """                P5Support.DataSource {
                    id: peekRunner
                    engine: "executable"
                    onNewData: source => disconnectSource(source)
                }

                Folder.ShortCut {
"""),
    # 3. Space on a selected icon: run peek on it instead of starting a type-ahead search
    ("                Keys.onPressed: event => {\n                    // Start by assuming we might handle it\n",
     """                Keys.onPressed: event => {
                    if (event.key === Qt.Key_Space && event.modifiers === Qt.NoModifier
                            && dir.hasSelection() && currentItem) {
                        const url = currentItem.linkDestinationUrl.replace(/'/g, "'\\\\''");
                        peekRunner.connectSource("PEEK '" + url + "'");
                        event.accepted = true;
                        return;
                    }
                    // Start by assuming we might handle it
"""),
]


def install():
    if LOCAL.exists() and not MARKER.exists():
        sys.exit(f"{LOCAL} exists and was not made by peek-desktop. Move it away first.")
    if LOCAL.exists():
        shutil.rmtree(LOCAL)
    shutil.copytree(SYSTEM, LOCAL)
    qml = LOCAL / "contents/ui/FolderView.qml"
    text = qml.read_text()
    # plasmashell's PATH may lack ~/.local/bin, so bake in peek's full path
    peek = shutil.which("peek") or sys.exit("peek is not on PATH; install it first.")
    for old, new in PATCHES:
        new = new.replace("PEEK", peek)
        if text.count(old) != 1:
            shutil.rmtree(LOCAL)
            sys.exit("This Plasma version's FolderView.qml changed shape; patch not applied, nothing installed.")
        text = text.replace(old, new)
    qml.write_text(text)
    version = subprocess.run(["pacman", "-Q", "plasma-desktop"], capture_output=True, text=True).stdout.strip()
    MARKER.write_text(f"patched from {version or 'unknown plasma-desktop'}\n")
    print(f"Installed. Restart the desktop to load it:\n  systemctl --user restart plasma-plasmashell")


def uninstall():
    if not MARKER.exists():
        sys.exit("Nothing to uninstall.")
    shutil.rmtree(LOCAL)
    print("Removed. Restart the desktop:\n  systemctl --user restart plasma-plasmashell")


if __name__ == "__main__":
    actions = {"install": install, "uninstall": uninstall}
    if len(sys.argv) != 2 or sys.argv[1] not in actions:
        sys.exit(__doc__)
    actions[sys.argv[1]]()
