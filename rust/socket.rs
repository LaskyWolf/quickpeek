// SPDX-License-Identifier: GPL-3.0-or-later
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

/// Where the window listens for files from later launches: `quickpeek-<uid>` in the temp folder,
/// the same place Qt's QLocalServer used in the Python app.
pub fn path() -> PathBuf {
    let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0);
    std::env::temp_dir().join(format!("quickpeek-{uid}"))
}
