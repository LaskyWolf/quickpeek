// SPDX-License-Identifier: GPL-3.0-or-later
//! `quickpeek-desktop`: makes Space on a selected Plasma desktop icon, or on an icon in a
//! Folder View widget, open `quickpeek`.
//!
//! Plasma loads a desktop package from ~/.local/share before /usr/share, so this installs a
//! patched copy of the system package there. The copy does not follow Plasma updates:
//! run `quickpeek-desktop install` again after every Plasma update.

use std::path::{Path, PathBuf};
use std::process::{Command, exit};

const USAGE: &str = "Usage: quickpeek-desktop install | uninstall";
const SYSTEM: &str = "/usr/share/plasma/plasmoids/org.kde.desktopcontainment";
const RESTART: &str = "systemctl --user restart plasma-plasmashell";

const PATCHES: [(&str, &str); 3] = [
    // 1. Plasma's module for running shell commands from QML
    ("import QtQml\n", "import QtQml\nimport org.kde.plasma.plasma5support as P5Support\n"),
    // 2. The command runner, next to the view's other helper objects
    (
        "                Folder.ShortCut {\n",
        r#"                P5Support.DataSource {
                    id: peekRunner
                    engine: "executable"
                    onNewData: source => disconnectSource(source)
                }

                Folder.ShortCut {
"#,
    ),
    // 3. Space on a selected icon: run quickpeek on it instead of starting a type-ahead search
    (
        "                Keys.onPressed: event => {\n                    // Start by assuming we might handle it\n",
        r#"                Keys.onPressed: event => {
                    if (event.key === Qt.Key_Space && event.modifiers === Qt.NoModifier
                            && dir.hasSelection() && currentItem) {
                        const url = currentItem.linkDestinationUrl.replace(/'/g, "'\\''");
                        peekRunner.connectSource("PEEK '" + url + "'");
                        event.accepted = true;
                        return;
                    }
                    // Start by assuming we might handle it
"#,
    ),
];

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    exit(1)
}

/// Applies every patch exactly once, or returns None if FolderView.qml changed shape.
fn patch(text: &str, quickpeek: &str) -> Option<String> {
    let mut text = text.to_string();
    for (old, new) in PATCHES {
        if text.matches(old).count() != 1 {
            return None;
        }
        text = text.replacen(old, &new.replace("PEEK", quickpeek), 1);
    }
    Some(text)
}

/// `quickpeek` next to this program, else on PATH. plasmashell's PATH may lack ~/.local/bin,
/// so the full path is baked into the patch.
fn quickpeek() -> Option<PathBuf> {
    let beside = std::env::current_exe().ok()?.with_file_name("quickpeek");
    if beside.is_file() {
        return Some(beside);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join("quickpeek")).find(|p| p.is_file())
}

fn install(local: &Path, marker: &Path, ours: bool) {
    if local.exists() && !ours {
        fail(&format!("{} exists and was not made by quickpeek-desktop. Move it away first.", local.display()));
    }
    let quickpeek = quickpeek().unwrap_or_else(|| fail("quickpeek is not installed; install it first."));
    let _ = std::fs::remove_dir_all(local);
    let copied = local.parent().is_some_and(|p| std::fs::create_dir_all(p).is_ok())
        && Command::new("cp").arg("-a").arg(SYSTEM).arg(local).status().is_ok_and(|s| s.success());
    if !copied {
        fail(&format!("Could not copy {SYSTEM} to {}.", local.display()));
    }
    let qml = local.join("contents/ui/FolderView.qml");
    let Some(text) = std::fs::read_to_string(&qml).ok().and_then(|t| patch(&t, &quickpeek.to_string_lossy())) else {
        let _ = std::fs::remove_dir_all(local);
        fail("This Plasma version's FolderView.qml changed shape; patch not applied, nothing installed.");
    };
    if std::fs::write(&qml, text).and_then(|_| std::fs::write(marker, "made by quickpeek-desktop\n")).is_err() {
        let _ = std::fs::remove_dir_all(local);
        fail(&format!("Could not write {}.", local.display()));
    }
    println!("Installed. Restart the desktop to load it:\n  {RESTART}");
}

fn main() {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| fail("HOME is not set.")));
    let local = home.join(".local/share/plasma/plasmoids/org.kde.desktopcontainment");
    let marker = local.join(".quickpeek-patch"); // proves the local copy is ours before we delete it
    let ours = marker.exists() || local.join(".peek-patch").exists(); // .peek-patch: older name of this tool
    match std::env::args().nth(1).as_deref() {
        Some("install") if std::env::args().count() == 2 => install(&local, &marker, ours),
        Some("uninstall") if std::env::args().count() == 2 => {
            if !ours {
                fail("Nothing to uninstall.");
            }
            if std::fs::remove_dir_all(&local).is_err() {
                fail(&format!("Could not remove {}.", local.display()));
            }
            println!("Removed. Restart the desktop:\n  {RESTART}");
        }
        _ => fail(USAGE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_once_or_refuses() {
        let qml = "import QtQml\n                Folder.ShortCut {\n                Keys.onPressed: event => {\n                    // Start by assuming we might handle it\n";
        let out = patch(qml, "/usr/bin/quickpeek").unwrap();
        assert!(out.contains("import org.kde.plasma.plasma5support as P5Support"));
        assert!(out.contains("connectSource(\"/usr/bin/quickpeek '\" + url"));
        assert!(out.contains(r#"replace(/'/g, "'\\''")"#));
        assert!(patch("import QtQml\n", "q").is_none());
    }
}
