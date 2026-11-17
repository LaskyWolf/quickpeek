// SPDX-License-Identifier: GPL-3.0-or-later
//! Backends of the folder and archive previews. Slow work (walking a folder, reading an archive)
//! runs on a worker thread; the views get JSON they parse with JSON.parse.
//! Workers make no Qt calls: Qt may already be torn down when one finishes after the window quit.

use crate::{files, helpers};
use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        /// A folder's first entries as icons, and its total size.
        #[qobject]
        #[qml_element]
        #[qproperty(QString, items)]
        #[qproperty(QString, summary)]
        #[qproperty(i32, more)]
        type Folder = super::FolderRust;

        #[qinvokable]
        fn load(self: Pin<&mut Folder>, path: &QString);

        /// The files inside an archive, as tree rows.
        #[qobject]
        #[qml_element]
        #[qproperty(QString, rows)]
        #[qproperty(QString, summary)]
        type Archive = super::ArchiveRust;

        #[qinvokable]
        fn load(self: Pin<&mut Archive>, path: &QString);
    }

    impl cxx_qt::Threading for Folder {}
    impl cxx_qt::Threading for Archive {}
}

const FOLDER_ITEMS: usize = 200; // icons shown in a folder preview
const COUNT_DEADLINE: Duration = Duration::from_secs(3); // a folder's total size gives up after this
const MAX_ENTRIES: usize = 20_000; // archive entries listed

#[derive(Default)]
pub struct FolderRust {
    items: QString,
    summary: QString,
    more: i32,
    loaded: PathBuf,
}

#[derive(Default)]
pub struct ArchiveRust {
    rows: QString,
    summary: QString,
    loaded: PathBuf,
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// (files, folders, bytes, complete) for everything under `folder`, giving up after `deadline`.
fn folder_summary(folder: &Path, deadline: Duration) -> (u64, u64, u64, bool) {
    let stop = Instant::now() + deadline;
    let (mut files, mut folders, mut size) = (0, 0, 0);
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            match entry.metadata() { // doesn't follow symlinks
                Ok(m) if m.is_dir() => {
                    folders += 1;
                    pending.push(entry.path());
                }
                m => {
                    files += 1;
                    size += m.map_or(0, |m| m.len());
                }
            }
        }
        if Instant::now() > stop {
            return (files, folders, size, false);
        }
    }
    (files, folders, size, true)
}

fn thumbnail(path: &Path) -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"));
    let key = helpers::thumbnail_key(&path.to_string_lossy());
    ["large", "normal"].iter().map(|size| cache.join("thumbnails").join(size).join(&key)).find(|t| t.exists())
}

impl qobject::Folder {
    fn load(mut self: Pin<&mut Self>, path: &QString) {
        let path = PathBuf::from(path.to_string());
        if path == self.rust().loaded {
            return;
        }
        self.as_mut().rust_mut().loaded = path.clone();
        let children = files::sorted_like_dolphin(&path, &path);
        let items: Vec<String> = children
            .iter()
            .take(FOLDER_ITEMS)
            .map(|child| {
                let text = child.to_string_lossy();
                let mime = helpers::mime_info(&text);
                let icon = if mime.icon.is_empty() { mime.generic_icon } else { mime.icon };
                let thumb = thumbnail(child).map_or(String::new(), |t| format!("file://{}", t.display()));
                let name = child.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                format!(r#"{{"n":{},"p":{},"i":{},"t":{}}}"#, json_str(&name), json_str(&text), json_str(&icon), json_str(&thumb))
            })
            .collect();
        let direct = children.len();
        self.as_mut().set_items(QString::from(format!("[{}]", items.join(",")).as_str()));
        self.as_mut().set_more(direct.saturating_sub(FOLDER_ITEMS) as i32);
        self.as_mut().set_summary(QString::from("Counting…"));
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let (files, folders, size, complete) = folder_summary(&path, COUNT_DEADLINE);
            let _ = thread.queue(move |mut folder| {
                if folder.rust().loaded == path {
                    let summary = format!(
                        "{direct} items here  ·  {files} files in {folders} subfolders  ·  {}{}{}",
                        if complete { "" } else { "at least " },
                        helpers::formatted_size(size),
                        if complete { "" } else { " (counting stopped after 3 s)" }
                    );
                    folder.as_mut().set_summary(QString::from(summary.as_str()));
                }
            });
        });
    }
}

#[derive(Debug, PartialEq)]
struct Entry {
    name: String,
    size: Option<u64>,
    mtime: Option<i64>, // from libarchive
    modified: String,   // from 7z, already local time
    is_dir: bool,
}

// libarchive: tar in any compression, zip, 7z, rar, iso, cpio. Linked directly: no crate wraps listing with sizes and dates.
#[link(name = "archive")]
unsafe extern "C" {
    fn archive_read_new() -> *mut c_void;
    fn archive_read_support_filter_all(a: *mut c_void) -> c_int;
    fn archive_read_support_format_all(a: *mut c_void) -> c_int;
    fn archive_read_open_filename(a: *mut c_void, path: *const c_char, block: usize) -> c_int;
    fn archive_read_next_header(a: *mut c_void, entry: *mut *mut c_void) -> c_int;
    fn archive_read_data_skip(a: *mut c_void) -> c_int;
    fn archive_read_free(a: *mut c_void) -> c_int;
    fn archive_entry_pathname_utf8(e: *mut c_void) -> *const c_char;
    fn archive_entry_pathname(e: *mut c_void) -> *const c_char;
    fn archive_entry_size(e: *mut c_void) -> i64;
    fn archive_entry_size_is_set(e: *mut c_void) -> c_int;
    fn archive_entry_mtime(e: *mut c_void) -> i64;
    fn archive_entry_mtime_is_set(e: *mut c_void) -> c_int;
    fn archive_entry_filetype(e: *mut c_void) -> u32;
}

const ARCHIVE_EOF: c_int = 1;
const ARCHIVE_WARN: c_int = -20;
const AE_IFDIR: u32 = 0o040000;

fn list_libarchive(path: &Path) -> Option<Vec<Entry>> {
    let cpath = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut entries = Vec::new();
    // SAFETY: `a` is freed once below; entry pointers are only read before the next header
    unsafe {
        let a = archive_read_new();
        archive_read_support_filter_all(a);
        archive_read_support_format_all(a);
        let mut ok = archive_read_open_filename(a, cpath.as_ptr(), 1 << 16) == 0;
        let mut e = std::ptr::null_mut();
        while ok && entries.len() < MAX_ENTRIES {
            match archive_read_next_header(a, &mut e) {
                ARCHIVE_EOF => break,
                r if r < ARCHIVE_WARN => ok = false,
                _ => {
                    let mut name = archive_entry_pathname_utf8(e);
                    if name.is_null() {
                        name = archive_entry_pathname(e);
                    }
                    if !name.is_null() {
                        entries.push(Entry {
                            name: CStr::from_ptr(name).to_string_lossy().into_owned(),
                            size: (archive_entry_size_is_set(e) != 0).then(|| archive_entry_size(e).max(0) as u64),
                            mtime: (archive_entry_mtime_is_set(e) != 0).then(|| archive_entry_mtime(e)),
                            modified: String::new(),
                            is_dir: archive_entry_filetype(e) == AE_IFDIR,
                        });
                    }
                    archive_read_data_skip(a);
                }
            }
        }
        archive_read_free(a);
        ok.then_some(entries)
    }
}

/// `7z l -slt` prints "Key = value" blocks after a "----------" line.
fn parse_7z(out: &str) -> Option<Vec<Entry>> {
    let (_, blocks) = out.split_once("----------")?;
    let entries = blocks
        .split("\n\n")
        .filter_map(|block| {
            let fields: HashMap<&str, &str> = block.lines().filter_map(|l| l.split_once(" = ")).collect();
            Some(Entry {
                name: fields.get("Path")?.to_string(),
                size: fields.get("Size").and_then(|s| s.parse().ok()),
                mtime: None,
                modified: fields.get("Modified").map_or("", |m| m.get(..16).unwrap_or(m)).to_string(),
                is_dir: fields.get("Folder") == Some(&"+") || fields.get("Attributes").is_some_and(|a| a.starts_with('D')),
            })
        })
        .take(MAX_ENTRIES)
        .collect();
    Some(entries)
}

/// Entries of an archive; 7z reads what libarchive can't (a lone .gz or .xz, some rar). None if neither can.
fn list_archive(path: &Path) -> Option<Vec<Entry>> {
    list_libarchive(path).or_else(|| {
        // -p with no password stops 7z asking for one
        let out = std::process::Command::new("7z")
            .args(["l", "-slt", "-p", "--"])
            .arg(path)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?; // ponytail: no timeout; the worker thread just stays busy if 7z hangs
        parse_7z(&String::from_utf8_lossy(&out.stdout))
    })
}

struct Row<'a> {
    leaf: &'a str,
    depth: usize,
    entry: Option<&'a Entry>, // None for a folder
    open: bool,
}

struct Tree<'a> {
    rows: Vec<Row<'a>>,
    files: usize,
    folders: usize,
    total: u64,
}

/// Rows of the archive's folder tree, each folder followed by its contents, names in case-insensitive order.
fn tree(entries: &[Entry]) -> Tree<'_> {
    struct Node<'a> {
        leaf: &'a str,
        entry: Option<&'a Entry>,
        children: Vec<usize>,
    }
    let mut nodes = vec![Node { leaf: "", entry: None, children: Vec::new() }];
    let mut folders: HashMap<&str, usize> = HashMap::from([("", 0)]);
    fn folder<'a>(name: &'a str, nodes: &mut Vec<Node<'a>>, folders: &mut HashMap<&'a str, usize>) -> usize {
        if let Some(&i) = folders.get(name) {
            return i;
        }
        let (parent, leaf) = name.rsplit_once('/').unwrap_or(("", name));
        let parent = folder(parent, nodes, folders);
        nodes.push(Node { leaf, entry: None, children: Vec::new() });
        let i = nodes.len() - 1;
        nodes[parent].children.push(i);
        folders.insert(name, i);
        i
    }
    let mut sorted: Vec<&Entry> = entries.iter().collect();
    sorted.sort_by_cached_key(|e| e.name.to_lowercase());
    let (mut files, mut total) = (0, 0);
    for entry in sorted {
        let name = entry.name.trim_matches('/');
        if name.is_empty() {
            continue;
        }
        if entry.is_dir {
            folder(name, &mut nodes, &mut folders);
            continue;
        }
        files += 1;
        total += entry.size.unwrap_or(0);
        let (parent, leaf) = name.rsplit_once('/').unwrap_or(("", name));
        let parent = folder(parent, &mut nodes, &mut folders);
        nodes.push(Node { leaf, entry: Some(entry), children: Vec::new() });
        let i = nodes.len() - 1;
        nodes[parent].children.push(i);
    }
    let expand_top = nodes[0].children.len() <= 3; // a single top folder opens by itself
    let mut rows = Vec::new();
    let mut stack: Vec<(usize, usize)> = nodes[0].children.iter().rev().map(|&i| (i, 0)).collect();
    while let Some((i, depth)) = stack.pop() {
        let node = &nodes[i];
        rows.push(Row { leaf: node.leaf, depth, entry: node.entry, open: node.entry.is_none() && depth == 0 && expand_top });
        stack.extend(node.children.iter().rev().map(|&c| (c, depth + 1)));
    }
    Tree { rows, files, folders: folders.len() - 1, total }
}

/// (rows JSON, summary) shown for an archive.
fn describe_archive(entries: Option<Vec<Entry>>, packed: u64) -> (String, String) {
    let Some(entries) = entries else {
        return ("[]".into(), "Can't read this archive (is 7z installed?)".into());
    };
    let tree = tree(&entries);
    let mut icons: HashMap<&str, String> = HashMap::new(); // one mime lookup per extension
    let rows: Vec<String> = tree
        .rows
        .iter()
        .map(|row| {
            let (icon, size, modified) = match row.entry {
                None => ("folder".to_string(), String::new(), String::new()),
                Some(e) => (
                    icons
                        .entry(row.leaf.rsplit_once('.').map_or(row.leaf, |(_, ext)| ext))
                        .or_insert_with(|| helpers::mime_icon_for_name(row.leaf))
                        .clone(),
                    e.size.map_or(String::new(), helpers::formatted_size),
                    e.mtime.map_or_else(|| e.modified.clone(), helpers::local_time),
                ),
            };
            format!(
                r#"{{"n":{},"d":{},"f":{},"o":{},"i":{},"s":{},"m":{}}}"#,
                json_str(row.leaf),
                row.depth,
                row.entry.is_none(),
                row.open,
                json_str(&icon),
                json_str(&size),
                json_str(&modified)
            )
        })
        .collect();
    let cut = if entries.len() >= MAX_ENTRIES { format!(" (first {MAX_ENTRIES})") } else { String::new() };
    let summary = format!(
        "{} files{cut}, {} folders  ·  {} unpacked, {} packed",
        tree.files,
        tree.folders,
        helpers::formatted_size(tree.total),
        helpers::formatted_size(packed)
    );
    (format!("[{}]", rows.join(",")), summary)
}

impl qobject::Archive {
    fn load(mut self: Pin<&mut Self>, path: &QString) {
        let path = PathBuf::from(path.to_string());
        if path == self.rust().loaded {
            return;
        }
        self.as_mut().rust_mut().loaded = path.clone();
        self.as_mut().set_rows(QString::from("[]"));
        self.as_mut().set_summary(QString::from("Reading archive…"));
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let entries = list_archive(&path);
            let packed = std::fs::metadata(&path).map_or(0, |m| m.len());
            let _ = thread.queue(move |mut archive| {
                if archive.rust().loaded == path {
                    let (rows, summary) = describe_archive(entries, packed);
                    archive.as_mut().set_rows(QString::from(rows.as_str()));
                    archive.as_mut().set_summary(QString::from(summary.as_str()));
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry { name: name.into(), size: Some(10), mtime: None, modified: String::new(), is_dir }
    }

    #[test]
    fn builds_the_tree_with_implicit_folders() {
        let entries = [entry("b/z.txt", false), entry("a.txt", false), entry("B/", true), entry("b/c/d.txt", false)];
        let tree = tree(&entries);
        let rows: Vec<(&str, usize, bool)> = tree.rows.iter().map(|r| (r.leaf, r.depth, r.open)).collect();
        // "B/" and "b/..." are different folders; sorting is case-insensitive and stable
        assert_eq!(rows, [("a.txt", 0, false), ("B", 0, true), ("b", 0, true), ("c", 1, false), ("d.txt", 2, false), ("z.txt", 1, false)]);
        assert_eq!((tree.files, tree.folders, tree.total), (3, 3, 30));
    }

    #[test]
    fn parses_7z_listings() {
        let out = "Listing archive: x.gz\n--\nPath = x.gz\nType = gzip\n\n----------\nPath = x\nSize = 1234\nModified = 2024-01-02 10:11:12.1234567\n\nPath = dir\nSize = \nFolder = +\n\n";
        assert_eq!(
            parse_7z(out).unwrap(),
            [
                Entry { name: "x".into(), size: Some(1234), mtime: None, modified: "2024-01-02 10:11".into(), is_dir: false },
                Entry { name: "dir".into(), size: None, mtime: None, modified: String::new(), is_dir: true },
            ]
        );
        assert!(parse_7z("ERROR: not an archive").is_none());
    }

    #[test]
    fn escapes_json() {
        assert_eq!(json_str("a\"b\\c\nd"), r#""a\"b\\c\u000ad""#);
    }
}
