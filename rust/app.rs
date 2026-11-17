// SPDX-License-Identifier: GPL-3.0-or-later
//! The window's backend: which file is shown, the files around it, and the socket other launches write to.

use crate::{files, helpers};
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
        #[qproperty(QString, details)]
        #[qproperty(QString, kind)]
        #[qproperty(QString, icon)]
        #[qproperty(QString, text)]
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
    details: QString,
    kind: QString,
    icon: QString,
    text: QString,
    count: i32,
    previous: QUrl,
    next: QUrl,
    files: Vec<PathBuf>,
    index: usize,
}

const ARCHIVES: &[&str] = &[
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

/// Which QML view shows the file.
fn kind_of(path: &Path) -> &'static str {
    let text = path.to_string_lossy();
    if path.is_dir() {
        "folder"
    } else if helpers::mime_inherits(&text, "application/pdf") {
        "pdf" // before images: Qt's PDF image plugin would read it as one
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
    } else if helpers::can_read_image(&text) {
        "image"
    } else if helpers::mime_inherits(&text, "text/markdown") {
        "markdown"
    } else if helpers::mime_inherits(&text, "text/html") || helpers::mime_inherits(&text, "application/xhtml+xml") {
        "html"
    } else if ARCHIVES.contains(&helpers::mime_info(&text).name.as_str()) {
        "archive"
    } else if helpers::mime_inherits(&text, "text/plain") {
        "text"
    } else {
        "other"
    }
}

fn is_sheet(path: &Path) -> bool {
    let ext = path.extension().map_or(String::new(), |e| e.to_string_lossy().to_lowercase());
    ["xlsx", "xlsm", "xlsb", "xls", "ods", "csv", "tsv"].contains(&ext.as_str())
}

fn read_text(path: &Path) -> String {
    let mut bytes = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(TEXT_LIMIT).read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).into_owned()
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
        let Ok(path) = PathBuf::from(path.to_string()).canonicalize() else { return };
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
            if count > 1 && kind_of(p) == "image" { url(p) } else { QUrl::default() }
        };
        let (previous, next) = (neighbour(count - 1), neighbour(1));
        let text = path.to_string_lossy();
        let mime = helpers::mime_info(&text);
        let size = match std::fs::metadata(&path) {
            Ok(m) if m.is_file() => helpers::formatted_size(m.len()),
            _ => String::new(),
        };
        let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let kind = kind_of(&path);
        let details = format!("<b>{}</b><br>{}  {size}  ·  {}/{count}", escape(&name), escape(&mime.comment), index + 1);
        let icon = if mime.icon.is_empty() { mime.generic_icon } else { mime.icon };

        // The view switches on `kind`, so the content it reads is set first
        self.as_mut().set_text(QString::from(if matches!(kind, "text" | "markdown" | "html") { read_text(&path) } else { String::new() }.as_str()));
        self.as_mut().set_source(url(&path));
        self.as_mut().set_path(QString::from(text.as_ref()));
        self.as_mut().set_details(QString::from(details.as_str()));
        self.as_mut().set_icon(QString::from(icon.as_str()));
        self.as_mut().set_count(count as i32);
        self.as_mut().set_previous(previous);
        self.as_mut().set_next(next);
        self.as_mut().set_kind(QString::from(kind));
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
