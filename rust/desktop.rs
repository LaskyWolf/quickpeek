// SPDX-License-Identifier: GPL-3.0-or-later
//! What the file menu asks of the desktop: the apps that open a file, Dolphin, the wallpaper.

use crate::helpers;
use cxx_qt_lib::QString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type Desktop = super::DesktopRust;

        /// Apps that open `path`, the default first, as JSON [[name, icon, .desktop file]].
        #[qinvokable]
        fn apps(self: &Desktop, path: &QString) -> QString;

        #[qinvokable]
        #[cxx_name = "openWith"]
        fn open_with(self: &Desktop, desktop_file: &QString, path: &QString);

        /// Opens the folder in the file manager with the file selected.
        #[qinvokable]
        #[cxx_name = "showInFolder"]
        fn show_in_folder(self: &Desktop, path: &QString);

        #[qinvokable]
        #[cxx_name = "setWallpaper"]
        fn set_wallpaper(self: &Desktop, path: &QString);

        /// True when Plasma's wallpaper tool is installed.
        #[qinvokable]
        #[cxx_name = "canSetWallpaper"]
        fn can_set_wallpaper(self: &Desktop) -> bool;
    }
}

#[derive(Default)]
pub struct DesktopRust {}

const WALLPAPER: &str = "plasma-apply-wallpaperimage";

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// $XDG_<name>, or `fallback` under the home folder.
fn xdg(name: &str, fallback: &str) -> PathBuf {
    std::env::var_os(name).filter(|v| !v.is_empty()).map_or_else(|| home().join(fallback), PathBuf::from)
}

/// Folders holding .desktop files, the user's first.
fn application_dirs() -> Vec<PathBuf> {
    let system = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or("/usr/local/share:/usr/share".into());
    std::iter::once(xdg("XDG_DATA_HOME", ".local/share"))
        .chain(system.split(':').map(PathBuf::from))
        .map(|d| d.join("applications"))
        .collect()
}

/// Values of `key` in `[section]` of an ini-style file (mimeapps.list, mimeinfo.cache, .desktop).
fn ini_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == format!("[{section}]");
        } else if let Some((k, v)) = line.split_once('=').filter(|_| inside) {
            if k.trim() == key {
                return Some(v.trim());
            }
        }
    }
    None
}

/// .desktop names for `types`: the user's defaults, then every app that says it opens them; each once, in order.
fn desktop_names(types: &[String], defaults: &str, caches: &[String]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let lists = std::iter::once((defaults, "Default Applications")).chain(caches.iter().map(|c| (c.as_str(), "MIME Cache")));
    for (text, section) in lists {
        for t in types {
            for name in ini_value(text, section, t).unwrap_or("").split(';').filter(|n| !n.is_empty()) {
                if !names.iter().any(|n| n == name) {
                    names.push(name.to_string());
                }
            }
        }
    }
    names
}

fn apps_for(path: &Path) -> Vec<(String, String, PathBuf)> {
    if !path.is_file() {
        return Vec::new();
    }
    let types = helpers::mime_ancestors(&path.to_string_lossy());
    let dirs = application_dirs();
    let defaults = std::fs::read_to_string(xdg("XDG_CONFIG_HOME", ".config").join("mimeapps.list")).unwrap_or_default();
    let caches: Vec<String> = dirs.iter().filter_map(|d| std::fs::read_to_string(d.join("mimeinfo.cache")).ok()).collect();
    desktop_names(&types, &defaults, &caches)
        .into_iter()
        .filter_map(|name| {
            // ponytail: desktop IDs with a '-' standing for a subfolder (kde-foo.desktop in kde/) aren't found
            let file = dirs.iter().map(|d| d.join(&name)).find(|f| f.is_file())?;
            let entry = std::fs::read_to_string(&file).ok()?;
            let get = |key| ini_value(&entry, "Desktop Entry", key);
            if get("NoDisplay") == Some("true") || get("Hidden") == Some("true") {
                return None;
            }
            Some((get("Name").unwrap_or(&name).to_string(), get("Icon").unwrap_or("").to_string(), file))
        })
        .collect()
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(program).is_file()))
}

/// Starts a program without waiting for it; a thread reaps it so it doesn't linger as a zombie.
fn spawn(program: &str, args: &[&std::ffi::OsStr]) {
    if let Ok(mut child) = Command::new(program).args(args).stdin(Stdio::null()).spawn() {
        std::thread::spawn(move || child.wait());
    }
}

impl qobject::Desktop {
    fn apps(&self, path: &QString) -> QString {
        let apps: Vec<serde_json::Value> = apps_for(Path::new(&path.to_string()))
            .into_iter()
            .map(|(name, icon, file)| serde_json::json!([name, icon, file.to_string_lossy()]))
            .collect();
        QString::from(serde_json::Value::Array(apps).to_string().as_str())
    }

    fn open_with(&self, desktop_file: &QString, path: &QString) {
        // gio fills in the app's own command line from its .desktop file
        spawn("gio", &["launch".as_ref(), desktop_file.to_string().as_ref(), path.to_string().as_ref()]);
    }

    fn show_in_folder(&self, path: &QString) {
        let path = PathBuf::from(path.to_string());
        if on_path("dolphin") {
            spawn("dolphin", &["--select".as_ref(), path.as_os_str()]);
        } else if let Some(folder) = path.parent() {
            helpers::open_url(&folder.to_string_lossy());
        }
    }

    fn set_wallpaper(&self, path: &QString) {
        spawn(WALLPAPER, &[path.to_string().as_ref()]);
    }

    fn can_set_wallpaper(&self) -> bool {
        on_path(WALLPAPER)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_default_apps_first_and_once() {
        let defaults = "[Added Associations]\ntext/plain=kate.desktop;\n[Default Applications]\ntext/x-python=kate.desktop\n";
        let cache = "[MIME Cache]\ntext/plain=org.kde.kwrite.desktop;kate.desktop;\nimage/png=gwenview.desktop;\n".to_string();
        let types = ["text/x-python".to_string(), "text/plain".to_string()];
        assert_eq!(desktop_names(&types, defaults, &[cache]), ["kate.desktop", "org.kde.kwrite.desktop"]);
        assert_eq!(ini_value("[Desktop Entry]\nName=Kate\nName[de]=K\n", "Desktop Entry", "Name"), Some("Kate"));
    }
}
