// SPDX-License-Identifier: GPL-3.0-or-later
//! KDEQuickPeek launcher. `quickpeek FILE` hands FILE to the QuickPeek window that is already
//! running, without loading Qt, so Space in Dolphin answers in a few milliseconds.
//! When no window runs yet, it starts the full app.

use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

// ponytail: the previewers still live in the Python app; this goes once they are ported to Rust
const PYTHON_APP: &str = match option_env!("QUICKPEEK_PY") {
    Some(path) => path,
    None => "/usr/lib/quickpeek/quickpeek.py",
};

/// Same place QLocalServer uses for the name `quickpeek-<uid>`: the temp folder.
fn socket_path() -> PathBuf {
    let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0);
    std::env::temp_dir().join(format!("quickpeek-{uid}"))
}

/// A plain path or a file:// URL as a local path. None for other URLs (like desktop:/).
fn local_path(arg: &str) -> Option<PathBuf> {
    match arg.strip_prefix("file://") {
        Some(rest) => Some(PathBuf::from(percent_decode(rest))),
        None if arg.contains(":/") => None,
        None => Some(PathBuf::from(arg)),
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Hand a message to the running window, in the format it reads: activation token, newline, message.
fn send_to_running(message: &str) -> bool {
    let Ok(mut socket) = UnixStream::connect(socket_path()) else {
        return false;
    };
    let token = std::env::var("XDG_ACTIVATION_TOKEN").unwrap_or_default();
    socket.write_all(format!("{token}\n{message}").as_bytes()).is_ok()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let message = match args.as_slice() {
        [flag] if flag == "--quit" => Some(flag.clone()),
        [arg] => local_path(arg)
            .and_then(|p| p.canonicalize().ok()) // the window compares resolved paths
            .map(|p| p.to_string_lossy().into_owned()),
        _ => None,
    };
    if let Some(message) = &message {
        if send_to_running(message) {
            if message == "--quit" {
                println!("stopped");
            }
            return ExitCode::SUCCESS;
        }
    }
    // No window yet, or an argument only the full app understands: start the full app.
    let error = Command::new("python3").arg(PYTHON_APP).args(&args).exec();
    eprintln!("quickpeek: cannot start {PYTHON_APP}: {error}");
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_paths_and_file_urls() {
        assert_eq!(local_path("/tmp/a b.txt"), Some(PathBuf::from("/tmp/a b.txt")));
        assert_eq!(local_path("file:///tmp/a%20b%C3%A8.txt"), Some(PathBuf::from("/tmp/a bè.txt")));
        assert_eq!(local_path("desktop:/a.txt"), None);
        assert_eq!(percent_decode("100%"), "100%");
    }
}
