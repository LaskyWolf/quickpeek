// SPDX-License-Identifier: GPL-3.0-or-later
//! The files next to the shown one, in the order Dolphin shows them.

use crate::helpers::{self, DolphinSort};
use std::cmp::Ordering;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// Value of `key` in the `[section]` of an INI text; keys compare case-insensitively like Dolphin's.
fn ini_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == format!("[{section}]");
        } else if inside {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    return Some(v.trim());
                }
            }
        }
    }
    None
}

/// Sorting Dolphin saved for `folder`: per-folder settings first, then the global default.
fn saved_sort(folder: &Path) -> DolphinSort {
    let mut sort = DolphinSort {
        found: false,
        role: "text".into(),
        descending: false,
        folders_first: true,
        hidden_last: false,
        show_hidden: false,
    };
    let view_props = home().join(".local/share/dolphin/view_properties");
    let dolphinrc = fs::read_to_string(home().join(".config/dolphinrc")).unwrap_or_default();
    let mut texts = Vec::new();
    if ini_value(&dolphinrc, "General", "GlobalViewProps") != Some("true") {
        // Dolphin 24+ stores per-folder view settings as an extended attribute on the folder
        texts.push(helpers::read_xattr(&folder.to_string_lossy(), "user.kde.fm.viewproperties#1"));
        let relative = folder.strip_prefix("/").unwrap_or(folder);
        for file in [folder.join(".directory"), view_props.join(relative).join(".directory")] {
            texts.push(fs::read_to_string(file).unwrap_or_default());
        }
    }
    texts.push(fs::read_to_string(view_props.join("global/.directory")).unwrap_or_default());
    if let Some(text) = texts.iter().find(|t| t.lines().any(|l| l.trim() == "[Dolphin]")) {
        let flag = |key, default| ini_value(text, "Dolphin", key).map_or(default, |v| v == "true");
        sort.role = ini_value(text, "Dolphin", "SortRole").unwrap_or("text").to_string();
        sort.descending = ini_value(text, "Dolphin", "SortOrder") == Some("1");
        sort.folders_first = flag("SortFoldersFirst", true);
        sort.hidden_last = flag("SortHiddenLast", false);
        sort.show_hidden = flag("HiddenFilesShown", false);
    }
    sort
}

/// Dolphin compares names without the extension first, then extensions.
/// A dot between digits ("v1.2") is not an extension.
fn split_extension(name: &str) -> (&str, &str) {
    let chars: Vec<(usize, char)> = name.char_indices().collect();
    let digit = |i: usize| chars.get(i).is_some_and(|(_, c)| c.is_ascii_digit());
    (1..chars.len())
        .rev()
        .find(|&i| chars[i].1 == '.' && !(digit(i - 1) && digit(i + 1)))
        .map_or((name, ""), |i| name.split_at(chars[i].0))
}

fn compare_names(a: &str, b: &str) -> Ordering {
    let ((stem_a, ext_a), (stem_b, ext_b)) = (split_extension(a), split_extension(b));
    let order = |x, y| helpers::collate(x, y).cmp(&0);
    order(stem_a, stem_b).then_with(|| order(ext_a, ext_b))
}

struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    meta: Option<fs::Metadata>,
}

impl Entry {
    fn time(&self, pick: fn(&fs::Metadata) -> std::io::Result<SystemTime>) -> SystemTime {
        self.meta
            .as_ref()
            .and_then(|m| pick(m).or_else(|_| m.modified()).ok()) // no birth time on some filesystems
            .unwrap_or(SystemTime::UNIX_EPOCH)
    }
}

/// Entries of `folder` in the order Dolphin shows them. `include` stays even if hidden.
pub fn sorted_like_dolphin(folder: &Path, include: &Path) -> Vec<PathBuf> {
    let live = helpers::dolphin_live_sort(&folder.to_string_lossy());
    sort_folder(folder, include, &if live.found { live } else { saved_sort(folder) })
}

fn sort_folder(folder: &Path, include: &Path, sort: &DolphinSort) -> Vec<PathBuf> {
    let mut entries: Vec<Entry> = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            if !sort.show_hidden && name.starts_with('.') && path != include {
                return None;
            }
            let meta = fs::metadata(&path).ok(); // follows symlinks, like Dolphin
            Some(Entry { is_dir: meta.as_ref().is_some_and(|m| m.is_dir()), path, name, meta })
        })
        .collect();
    let mime_comment = |e: &Entry| helpers::mime_info(&e.path.to_string_lossy()).comment;
    let by_role = |a: &Entry, b: &Entry| -> Ordering {
        match sort.role.to_lowercase().as_str() {
            "size" => {
                let size = |e: &Entry| if e.is_dir { 0 } else { e.meta.as_ref().map_or(0, |m| m.size()) };
                size(a).cmp(&size(b))
            }
            "modificationtime" => a.time(fs::Metadata::modified).cmp(&b.time(fs::Metadata::modified)),
            "accesstime" => a.time(fs::Metadata::accessed).cmp(&b.time(fs::Metadata::accessed)),
            "creationtime" => a.time(fs::Metadata::created).cmp(&b.time(fs::Metadata::created)),
            "type" => helpers::collate(&mime_comment(a), &mime_comment(b)).cmp(&0),
            _ => Ordering::Equal,
        }
    };
    entries.sort_by(|a, b| {
        let order = by_role(a, b).then_with(|| compare_names(&a.name, &b.name));
        if sort.descending { order.reverse() } else { order }
    });
    // Grouping never flips with the sort direction; sort_by is stable, so the order above survives
    entries.sort_by_key(|e| (sort.hidden_last && e.name.starts_with('.'), sort.folders_first && !e.is_dir));
    entries.into_iter().map(|e| e.path).collect()
}

pub fn siblings(path: &Path) -> Vec<PathBuf> {
    match path.parent() {
        Some(folder) => sorted_like_dolphin(folder, path),
        None => vec![path.to_path_buf()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sort_like_dolphin() {
        let mut names = vec!["file-2.txt", "v1.10", "file.txt", "a.tar.gz", "v1.2", "a.b"];
        names.sort_by(|a, b| compare_names(a, b));
        assert_eq!(names, ["a.b", "a.tar.gz", "file.txt", "file-2.txt", "v1.2", "v1.10"]);
    }

    #[test]
    fn reads_ini_keys_case_insensitively() {
        let text = "[Other]\nSortRole=size\n[Dolphin]\nsortrole=modificationtime\nSortOrder=1\n";
        assert_eq!(ini_value(text, "Dolphin", "SortRole"), Some("modificationtime"));
        assert_eq!(ini_value(text, "Dolphin", "SortOrder"), Some("1"));
        assert_eq!(ini_value(text, "Other", "SortOrder"), None);
    }

    #[test]
    fn groups_folders_and_keeps_the_shown_hidden_file() {
        let dir = std::env::temp_dir().join(format!("quickpeek-test-{}", std::process::id()));
        fs::create_dir_all(dir.join("zfolder")).unwrap();
        for name in ["b.txt", "a.txt", ".hidden", ".other"] {
            fs::write(dir.join(name), "x").unwrap();
        }
        let sort = DolphinSort {
            found: true,
            role: "text".into(),
            descending: true,
            folders_first: true,
            hidden_last: false,
            show_hidden: false,
        };
        let names: Vec<String> = sort_folder(&dir, &dir.join(".hidden"), &sort)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(names, ["zfolder", "b.txt", "a.txt", ".hidden"]);
    }
}
