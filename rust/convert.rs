// SPDX-License-Identifier: GPL-3.0-or-later
//! Slow conversions (Office → PDF, later 3D and Krita) cached in ~/.cache/quickpeek.
//! One runs at a time: starting a job, or dropping its `Job`, kills the one before, so flipping fast
//! through files never piles up converters.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

const CACHE_LIMIT: u64 = 500 * 1024 * 1024; // oldest converted files are deleted beyond this
const X2T: &str = "/opt/onlyoffice/desktopeditors/converter/x2t"; // ONLYOFFICE's document converter
pub const X2T_PDF: u32 = 513; // x2t's format code for PDF

static CURRENT: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".cache"));
    base.join("quickpeek")
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// Changes whenever the file does, so an edited file is never shown from a stale cache.
/// The same key as the Python app, so both share the cache.
pub fn cache_key(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    let nanos = meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128;
    Some(crate::helpers::md5_hex(&format!("{}:{nanos}:{}", path.display(), meta.size())))
}

/// The running job. Dropping it kills the converter.
pub struct Job(Arc<AtomicBool>);

impl Drop for Job {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Runs commands for a job, killing them when the job is cancelled.
pub struct Runner(Arc<AtomicBool>);

impl Runner {
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub fn run(&self, command: &mut Command) -> io::Result<()> {
        if self.cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        let mut child = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        loop {
            if let Some(status) = child.try_wait()? {
                return if status.success() { Ok(()) } else { Err(io::Error::other(format!("{status}"))) };
            }
            if self.cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Makes `out` (a file or folder in the cache) on a worker thread, unless it is there already:
/// `work` writes the temporary `part`, renamed to `out` when it succeeds. Then `done` gets `out`, or None
/// when it failed. A cancelled job never calls `done`.
pub fn start(
    out: PathBuf,
    work: impl FnOnce(&Path, &Runner) -> io::Result<()> + Send + 'static,
    done: impl FnOnce(Option<PathBuf>) + Send + 'static,
) -> Job {
    let flag = Arc::new(AtomicBool::new(false));
    if let Some(old) = CURRENT.lock().unwrap().replace(flag.clone()) {
        old.store(true, Ordering::Relaxed);
    }
    let runner = Runner(flag.clone());
    std::thread::spawn(move || {
        if out.exists() {
            touch(&out); // recently used: pruning keeps it longer
            return done(Some(out));
        }
        let part = part_of(&out);
        remove(&part); // left by a killed run
        let _ = std::fs::create_dir_all(cache_dir().join("tmp"));
        let ok = work(&part, &runner).is_ok() && !runner.cancelled() && std::fs::rename(&part, &out).is_ok();
        if !ok {
            remove(&part);
        }
        if runner.cancelled() {
            return;
        }
        if ok {
            prune(&cache_dir(), CACHE_LIMIT);
        }
        done(ok.then_some(out));
    });
    Job(flag)
}

/// "<key>.pdf" → "<key>.part.pdf": a killed job never looks finished.
fn part_of(out: &Path) -> PathBuf {
    let name = out.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let part = match name.split_once('.') {
        Some((stem, rest)) => format!("{stem}.part.{rest}"),
        None => format!("{name}.part"),
    };
    out.with_file_name(part)
}

fn touch(path: &Path) {
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

fn remove(path: &Path) {
    let _ = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
}

fn size(path: &Path) -> u64 {
    match std::fs::read_dir(path) {
        Ok(entries) => entries.flatten().map(|e| size(&e.path())).sum(),
        Err(_) => std::fs::metadata(path).map_or(0, |m| m.len()),
    }
}

/// Keeps the converted files under `limit`, newest first.
fn prune(cache: &Path, limit: u64) {
    let Ok(entries) = std::fs::read_dir(cache) else { return };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            name != "tmp" && !name.contains(".part.") && !name.ends_with(".xml") // working files of running jobs
        })
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let mut total = 0;
    for (_, path) in files {
        total += size(&path);
        if total > limit {
            remove(&path);
        }
    }
}

/// ONLYOFFICE's converter, when it and the fonts it lists on its first launch are there.
pub fn x2t() -> Option<(PathBuf, PathBuf)> {
    let fonts = home().join(".local/share/onlyoffice/desktopeditors/data/fonts");
    (Path::new(X2T).exists() && fonts.join("AllFonts.js").exists()).then(|| (PathBuf::from(X2T), fonts))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Converts `src` into `dst` with x2t; `format` is x2t's code for the output (X2T_PDF…).
pub fn x2t_convert(src: &Path, dst: &Path, format: u32, runner: &Runner) -> io::Result<()> {
    let (x2t, fonts) = x2t().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    // x2t leaves its unpacked files behind: each run gets its own folder, deleted after
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let work = cache_dir().join("tmp").join(format!("x2t-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&work)?;
    let task = work.join("task.xml");
    let [src, dst, tmp, fonts_dir, all_fonts] = [src, dst, &work, &fonts, &fonts.join("AllFonts.js")].map(|p| escape(&p.to_string_lossy()));
    std::fs::write(
        &task,
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<TaskQueueDataConvert>
  <m_sFileFrom>{src}</m_sFileFrom>
  <m_sFileTo>{dst}</m_sFileTo>
  <m_nFormatTo>{format}</m_nFormatTo>
  <m_sTempDir>{tmp}</m_sTempDir>
  <m_sFontDir>{fonts_dir}</m_sFontDir>
  <m_sAllFontsPath>{all_fonts}</m_sAllFontsPath>
  <m_bIsNoBase64>true</m_bIsNoBase64>
</TaskQueueDataConvert>"#
        ),
    )?;
    let result = runner.run(Command::new(x2t).arg(&task));
    let _ = std::fs::remove_dir_all(&work);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_names_never_look_finished() {
        assert_eq!(part_of(Path::new("/c/abc.pdf")), Path::new("/c/abc.part.pdf"));
        assert_eq!(part_of(Path::new("/c/abc.morph")), Path::new("/c/abc.part.morph"));
    }

    #[test]
    fn prune_keeps_the_newest_under_the_limit() {
        let dir = std::env::temp_dir().join(format!("quickpeek-prune-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("tmp")).unwrap();
        for (n, name) in ["old.pdf", "mid.pdf", "new.pdf", "job.part.pdf"].iter().enumerate() {
            std::fs::write(dir.join(name), [0u8; 100]).unwrap();
            let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1000 + n as u64);
            std::fs::File::options().write(true).open(dir.join(name)).unwrap().set_modified(time).unwrap();
        }
        prune(&dir, 250);
        let left = |name: &str| dir.join(name).exists();
        assert!(left("new.pdf") && left("mid.pdf") && !left("old.pdf"));
        assert!(left("job.part.pdf") && left("tmp"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
