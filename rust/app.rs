// SPDX-License-Identifier: GPL-3.0-or-later
//! The window's backend: which file is shown, the files around it, and the socket other launches write to.

use crate::{embedded, files, helpers};
use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QUrl};
use std::io::Read;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, path)]
        #[qproperty(QUrl, source)]
        /// What ImageView shows: the file itself, or a preview taken from it
        #[qproperty(QUrl, picture)]
        #[qproperty(QString, details)]
        #[qproperty(QString, kind)]
        #[qproperty(QString, icon)]
        #[qproperty(QString, text)]
        /// Why the file can't be read at all ("" when it can): the window says it instead of a preview
        #[qproperty(QString, problem)]
        #[qproperty(i32, count)]
        #[qproperty(QUrl, previous)]
        #[qproperty(QUrl, next)]
        type Peek = super::PeekRust;

        /// Opens the file given on the command line.
        #[qinvokable]
        fn start(self: Pin<&mut Peek>);

        #[qinvokable]
        fn open(self: Pin<&mut Peek>, path: &QString);

        /// Another file in the folder: -1 previous, 1 next.
        #[qinvokable]
        fn step(self: Pin<&mut Peek>, delta: i32);

        /// Wayland only lets a window come forward with the launcher's activation token.
        #[qinvokable]
        fn activate(self: Pin<&mut Peek>, token: &QString);

        /// Reads the shown file's size again for the bottom bar, after the view changed the file.
        #[qinvokable]
        #[cxx_name = "refreshDetails"]
        fn refresh_details(self: Pin<&mut Peek>);

        /// Lets go of the preview's memory while the window is hidden.
        #[qinvokable]
        fn clear(self: Pin<&mut Peek>);

        #[qinvokable]
        #[cxx_name = "openExternal"]
        fn open_external(self: &Peek);

        #[qinvokable]
        #[cxx_name = "copyFile"]
        fn copy_file(self: &Peek);

        #[qinvokable]
        #[cxx_name = "copyPath"]
        fn copy_path(self: &Peek);

        /// Moves the file to the Trash and shows the next one. False if that failed.
        #[qinvokable]
        fn trash(self: Pin<&mut Peek>) -> bool;

        /// Renames the shown file. Returns an error message, or nothing when it worked.
        #[qinvokable]
        fn rename(self: Pin<&mut Peek>, name: &QString) -> QString;

        /// Another launch handed over `path`, or "--quit".
        #[qsignal]
        fn received(self: Pin<&mut Peek>, path: QString, token: QString);
    }

    impl cxx_qt::Threading for Peek {}
    impl cxx_qt::Initialize for Peek {}
}

use qobject::Peek;

const TEXT_LIMIT: u64 = 1_000_000; // bytes shown for text files, so a 2 GB log doesn't freeze the window

#[derive(Default)]
pub struct PeekRust {
    path: QString,
    source: QUrl,
    picture: QUrl,
    details: QString,
    kind: QString,
    icon: QString,
    text: QString,
    problem: QString,
    count: i32,
    previous: QUrl,
    next: QUrl,
    files: Vec<PathBuf>,
    index: usize,
}

pub(crate) const ARCHIVES: &[&str] = &[
    "application/zip", "application/x-tar", "application/x-compressed-tar", "application/x-bzip2-compressed-tar",
    "application/x-bzip-compressed-tar", "application/x-xz-compressed-tar", "application/x-zstd-compressed-tar",
    "application/x-lzma-compressed-tar", "application/x-7z-compressed", "application/vnd.rar", "application/x-rar",
    "application/x-iso9660-image", "application/x-cd-image", "application/java-archive",
    "application/vnd.android.package-archive", "application/x-cpio", "application/gzip", "application/x-xz",
    "application/x-bzip2", "application/zstd",
];

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Why the file can't be opened for reading, or "" when it can.
fn unreadable(path: &Path) -> String {
    let opened = if path.is_dir() { std::fs::read_dir(path).map(drop) } else { std::fs::File::open(path).map(drop) };
    match opened {
        Ok(()) => String::new(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && path.is_symlink() => "This link points to a file that no longer exists.".into(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "This file no longer exists.".into(),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => "You don't have permission to read this file.".into(),
        Err(e) => format!("This file can't be read: {e}."),
    }
}

/// Which QML view shows the file.
fn kind_of(path: &Path) -> &'static str {
    let text = path.to_string_lossy();
    if path.is_dir() {
        "folder"
    } else if helpers::mime_inherits(&text, "application/pdf") {
        "pdf" // before images: Qt's PDF image plugin would read it as one
    } else if crate::model::is_model(path) {
        "model" // before text: .gltf and .obj are text files too
    } else if let Some(kind) = crate::media::kind(&text) {
        kind
    } else if path.extension().is_some_and(|e| ["ttf", "otf", "ttc", "otc", "woff", "woff2"].contains(&e.to_string_lossy().to_lowercase().as_str()))
        || helpers::mime_info(&text).name.starts_with("font/")
    {
        "font" // by suffix too: .otf/.otc are also OpenDocument template globs
    } else if is_sheet(path) {
        "sheet"
    } else if crate::office::handles(path) {
        "office"
    } else if embedded::has_preview(path) || helpers::can_read_image(&text) {
        "image"
    } else if helpers::mime_inherits(&text, "text/markdown") {
        "markdown"
    } else if helpers::mime_inherits(&text, "text/html") || helpers::mime_inherits(&text, "application/xhtml+xml") {
        "html"
    } else if ARCHIVES.contains(&helpers::mime_info(&text).name.as_str()) {
        "archive"
    } else if helpers::mime_inherits(&text, "text/plain") {
        "text"
    } else if embedded::thumbnail(path).is_some() {
        "image"
    } else {
        "other"
    }
}

/// An image Qt decodes itself; other "image" files show a preview taken from them.
fn direct_image(path: &Path) -> bool {
    !embedded::has_preview(path) && helpers::can_read_image(&path.to_string_lossy())
}

fn is_sheet(path: &Path) -> bool {
    let ext = path.extension().map_or(String::new(), |e| e.to_string_lossy().to_lowercase());
    ["xlsx", "xlsm", "xlsb", "xls", "ods", "csv", "tsv"].contains(&ext.as_str())
}

/// Every picture at most as wide as the window: Qt draws them full size otherwise.
fn fit_images(html: &str) -> String {
    html.replace("<img ", "<img style=\"max-width:100%\" ").replace("<IMG ", "<IMG style=\"max-width:100%\" ")
}

fn read_text(path: &Path) -> String {
    let mut bytes = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(TEXT_LIMIT).read_to_end(&mut bytes);
    }
    decode(&bytes)
}

/// UTF-8, or Windows-1252 when the bytes aren't UTF-8: the usual encoding of older Windows text files.
fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        // Only the last character is cut (the file was read up to TEXT_LIMIT): still UTF-8
        Err(e) if e.error_len().is_none() => String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned(),
        Err(_) => bytes.iter().map(|&b| windows_1252(b)).collect(),
    }
}

fn windows_1252(b: u8) -> char {
    const HIGH: &str = "€\u{81}‚ƒ„…†‡ˆ‰Š‹Œ\u{8d}Ž\u{8f}\u{90}‘’“”•–—˜™š›œ\u{9d}žŸ";
    match b {
        0x80..=0x9f => HIGH.chars().nth(usize::from(b - 0x80)).unwrap_or('\u{fffd}'),
        _ => char::from(b), // the rest of Windows-1252 is Latin-1, the first 256 Unicode code points
    }
}

fn url(path: &Path) -> QUrl {
    QUrl::from_local_file(&QString::from(path.to_string_lossy().as_ref()))
}

/// Accepts other launches on the same socket the launcher in main.rs writes to.
fn serve(thread: cxx_qt::CxxQtThread<Peek>) {
    let socket = crate::socket::path();
    let _ = std::fs::remove_file(&socket); // left behind by a crashed quickpeek
    let Ok(listener) = UnixListener::bind(&socket) else {
        eprintln!("quickpeek: cannot listen on {}", socket.display());
        return;
    };
    for mut stream in listener.incoming().flatten() {
        let mut message = String::new();
        if stream.read_to_string(&mut message).is_err() {
            continue;
        }
        let (token, path) = message.split_once('\n').unwrap_or(("", message.as_str()));
        let (token, path) = (QString::from(token), QString::from(path));
        let _ = thread.queue(move |peek| peek.received(path, token));
    }
}

impl cxx_qt::Initialize for Peek {
    fn initialize(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        std::thread::spawn(move || serve(thread));
    }
}

impl qobject::Peek {
    fn current(&self) -> Option<&PathBuf> {
        self.rust().files.get(self.rust().index)
    }

    fn start(self: Pin<&mut Self>) {
        if let Some(arg) = std::env::args().nth(1) {
            self.open(&QString::from(arg.as_str()));
        }
    }

    fn open(mut self: Pin<&mut Self>, path: &QString) {
        let given = PathBuf::from(path.to_string());
        // A broken link or a deleted file has no canonical path: keep it, show_file says what's wrong
        let Ok(path) = given.canonicalize().or_else(|_| std::path::absolute(&given)) else { return };
        let files = files::siblings(&path);
        let index = files.iter().position(|p| *p == path).unwrap_or(0);
        let mut rust = self.as_mut().rust_mut();
        rust.files = files;
        rust.index = index;
        self.show_file();
    }

    fn step(mut self: Pin<&mut Self>, delta: i32) {
        let count = self.rust().files.len() as i64;
        if count == 0 {
            return;
        }
        let index = (self.rust().index as i64 + delta as i64).rem_euclid(count) as usize;
        self.as_mut().rust_mut().index = index;
        self.show_file();
    }

    fn show_file(mut self: Pin<&mut Self>) {
        let Some(path) = self.current().cloned() else { return };
        let files = &self.rust().files;
        let (index, count) = (self.rust().index, files.len());
        let neighbour = |delta: usize| {
            let p = &files[(index + delta) % count];
            if count > 1 && kind_of(p) == "image" && direct_image(p) { url(p) } else { QUrl::default() }
        };
        let (previous, next) = (neighbour(count - 1), neighbour(1));
        let text = path.to_string_lossy();
        let mime = helpers::mime_info(&text);
        let problem = unreadable(&path);
        let kind = if problem.is_empty() { kind_of(&path) } else { "other" };
        let details = self.bar_text(&path, &mime.comment);
        let icon = if mime.icon.is_empty() { mime.generic_icon } else { mime.icon };

        // The view switches on `kind`, so the content it reads is set first. A different kind of file unloads
        // the old view before that: a video player handed a PDF would try to play it.
        if self.kind().to_string() != kind {
            self.as_mut().set_kind(QString::default());
        }
        let content = match kind {
            "text" => read_text(&path),
            "markdown" => fit_images(&helpers::markdown_to_html(&read_text(&path))),
            "html" => fit_images(&read_text(&path)),
            _ => String::new(),
        };
        self.as_mut().set_text(QString::from(content.as_str()));
        self.as_mut().set_source(url(&path));
        let direct = kind == "image" && direct_image(&path);
        self.as_mut().set_picture(if direct { url(&path) } else { QUrl::default() });
        if kind == "image" && !direct {
            self.as_mut().load_picture(path.clone());
        }
        self.as_mut().set_problem(QString::from(problem.as_str()));
        self.as_mut().set_path(QString::from(text.as_ref()));
        self.as_mut().set_details(QString::from(details.as_str()));
        self.as_mut().set_icon(QString::from(icon.as_str()));
        self.as_mut().set_count(count as i32);
        self.as_mut().set_previous(previous);
        self.as_mut().set_next(next);
        self.as_mut().set_kind(QString::from(kind));
    }

    /// "<b>name</b><br>type  size  ·  position/count" for the bottom bar.
    fn bar_text(&self, path: &Path, comment: &str) -> String {
        let size = match std::fs::metadata(path) {
            Ok(m) if m.is_file() => helpers::formatted_size(m.len()),
            _ => String::new(),
        };
        let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let (index, count) = (self.rust().index, self.rust().files.len());
        format!("<b>{}</b><br>{}  {size}  ·  {}/{count}", escape(&name), escape(comment), index + 1)
    }

    fn refresh_details(mut self: Pin<&mut Self>) {
        let Some(path) = self.current().cloned() else { return };
        let details = self.bar_text(&path, &helpers::mime_info(&path.to_string_lossy()).comment);
        self.as_mut().set_details(QString::from(details.as_str()));
    }

    /// Extracts the preview on a worker thread: a .clip or .af can be hundreds of MB.
    fn load_picture(self: Pin<&mut Self>, path: PathBuf) {
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let picture = embedded::picture(&path);
            let _ = thread.queue(move |mut peek| {
                if peek.path().to_string() != path.to_string_lossy() || peek.kind().to_string() != "image" {
                    return; // the window moved on to another file meanwhile
                }
                match picture {
                    Some(p) => peek.as_mut().set_picture(url(&p)),
                    None => peek.as_mut().set_kind(QString::from("other")), // damaged or no preview inside
                }
            });
        });
    }

    fn activate(self: Pin<&mut Self>, token: &QString) {
        if !token.is_empty() {
            // SAFETY: only the Qt thread touches the environment
            unsafe { std::env::set_var("XDG_ACTIVATION_TOKEN", token.to_string()) };
        }
    }

    fn clear(mut self: Pin<&mut Self>) {
        self.as_mut().set_kind(QString::from(""));
        self.as_mut().set_source(QUrl::default());
        self.as_mut().set_picture(QUrl::default());
        self.as_mut().set_text(QString::default());
        self.as_mut().set_previous(QUrl::default());
        self.as_mut().set_next(QUrl::default());
    }

    fn open_external(&self) {
        if let Some(path) = self.current() {
            helpers::open_url(&path.to_string_lossy());
        }
    }

    fn copy_file(&self) {
        if let Some(path) = self.current() {
            helpers::copy_file(&path.to_string_lossy(), kind_of(path) == "image");
        }
    }

    fn copy_path(&self) {
        if let Some(path) = self.current() {
            helpers::copy_text(&path.to_string_lossy());
        }
    }

    fn trash(mut self: Pin<&mut Self>) -> bool {
        let Some(path) = self.current().cloned() else { return false };
        if !helpers::move_to_trash(&path.to_string_lossy()) {
            return false;
        }
        let mut rust = self.as_mut().rust_mut();
        let index = rust.index;
        rust.files.remove(index);
        if rust.files.is_empty() {
            drop(rust);
            self.clear();
            return true;
        }
        rust.index = index % rust.files.len();
        drop(rust);
        self.show_file();
        true
    }

    fn rename(mut self: Pin<&mut Self>, name: &QString) -> QString {
        let Some(path) = self.current().cloned() else { return QString::default() };
        let name = name.to_string().trim().to_string();
        let new = path.with_file_name(&name);
        let error = if name.is_empty() || new == path {
            return QString::default();
        } else if name.contains('/') {
            format!("Can't use “{name}”: a name can't contain /.")
        } else if new.exists() {
            format!("Can't use “{name}”: a file with that name exists.")
        } else if let Err(error) = std::fs::rename(&path, &new) {
            format!("Couldn't rename: {error}.")
        } else {
            let mut rust = self.as_mut().rust_mut();
            let index = rust.index;
            rust.files[index] = new;
            drop(rust);
            self.show_file();
            String::new()
        };
        QString::from(error.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8_and_windows_1252() {
        assert_eq!(decode("café ✓".as_bytes()), "café ✓");
        assert_eq!(decode(b"caf\xe9 \x80 \x93x\x94"), "café € “x”");
        assert_eq!(decode(&"ab✓".as_bytes()[..4]), "ab"); // cut inside the last character
    }

    #[test]
    fn says_why_a_file_cant_be_read() {
        let dir = std::env::temp_dir().join(format!("qp-unreadable-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join("link");
        let _ = std::os::unix::fs::symlink(dir.join("missing"), &link);
        assert_eq!(unreadable(&link), "This link points to a file that no longer exists.");
        assert_eq!(unreadable(&dir.join("gone")), "This file no longer exists.");
        assert_eq!(unreadable(&dir), "");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
