// SPDX-License-Identifier: GPL-3.0-or-later
//! Backend of the side panel's tools page: what the file can be converted into, and the conversion.
//! Results land next to the original under a free name; nothing is ever overwritten.
//! Conversions run on worker threads and are not cancelled when you flip to another file:
//! the result is only dropped from the page.

use crate::{convert, helpers, model};
use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
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
        #[qproperty(QString, path)]
        #[qproperty(QString, targets)] // JSON [[code, button text, icon]]
        #[qproperty(QString, status)] // the latest job as styled text: progress, the made file, or the error
        #[qproperty(QString, result)] // the file or folder the latest job made
        #[qproperty(bool, busy)]
        type Tools = super::ToolsRust;

        /// Converts the file to the target `code` (an extension like ".png", "/" to extract, "up2"/"up4" to enlarge).
        #[qinvokable]
        fn start(self: Pin<&mut Tools>, code: &QString);
    }

    impl cxx_qt::Threading for Tools {}
    impl cxx_qt::Initialize for Tools {}
}

use qobject::Tools;

#[derive(Default)]
pub struct ToolsRust {
    path: QString,
    targets: QString,
    status: QString,
    result: QString,
    busy: bool,
    job: u64, // bumped by every new job and file: late results of older ones are dropped
}

// x2t's format codes for the outputs it writes
const X2T_CODES: &[(&str, u32)] = &[
    (".docx", 65), (".odt", 67), (".rtf", 68), (".txt", 69), (".html", 70), (".epub", 72), (".fb2", 73),
    (".pptx", 129), (".odp", 131), (".xlsx", 257), (".ods", 259), (".csv", 260), (".pdf", convert::X2T_PDF),
];
const WORD: &[&str] = &[".docx", ".docm", ".doc", ".odt", ".rtf", ".epub", ".fb2"];
const SHEET: &[&str] = &[".xlsx", ".xlsm", ".xls", ".ods", ".csv"];
const SLIDES: &[&str] = &[".pptx", ".ppt", ".odp"];
const PAGES: &[&str] = &[".djvu", ".xps", ".oxps"]; // x2t turns these into PDF
const RASTER: &[&str] = &[".png", ".jpg", ".webp", ".avif", ".tiff", ".bmp"];
const AUDIO_OUT: &[&str] = &[".mp3", ".ogg", ".opus", ".flac", ".wav", ".m4a"];
const VIDEO_OUT: &[&str] = &[".mp4", ".webm", ".mkv", ".gif", ".mp3"];
const MODEL_OUT: &[&str] = &[".glb", ".gltf", ".obj", ".stl", ".fbx", ".ply", ".dae"];
// Office files and e-books x2t reads: never sent to Qt's text engine even when their type is text
const NOT_TEXT: &[&str] = &[
    ".docx", ".xlsx", ".pptx", ".odt", ".ods", ".odp", ".docm", ".xlsm", ".doc", ".xls", ".ppt", ".rtf",
    ".epub", ".fb2", ".djvu", ".xps", ".oxps",
];

/// What a file can become: [(code, label)]. `image` is Qt's (writable formats, animated) when it reads the file,
/// `text` whether its type is text, `have` whether a tool is installed.
fn targets(mime: &str, suffix: &str, image: Option<(&[String], bool)>, text: bool, have: &dyn Fn(&str) -> bool) -> Vec<(&'static str, &'static str)> {
    let plain = |exts: &[&'static str]| exts.iter().map(|e| (*e, "")).collect::<Vec<_>>();
    let is_model = model::MODELS.contains(&suffix.trim_start_matches('.'));
    let out = if mime.starts_with("video/") {
        let label = |e: &str| match e { ".mp3" => "audio only", ".gif" => "animation", _ => "" };
        if have("ffmpeg") { VIDEO_OUT.iter().map(|e| (*e, label(e))).collect() } else { Vec::new() }
    } else if mime.starts_with("audio/") {
        if have("ffmpeg") { plain(AUDIO_OUT) } else { Vec::new() }
    } else if mime == "application/pdf" {
        [(".pdf", "smaller file", "gs"), (".png", "one image per page", "pdftoppm"), (".docx", "editable, layout may shift", "x2t")]
            .into_iter()
            .filter(|t| have(t.2))
            .map(|(e, l, _)| (e, l))
            .collect()
    } else if WORD.contains(&suffix) && have("x2t") {
        plain(&[".docx", ".odt", ".rtf", ".pdf", ".txt", ".html", ".epub"])
    } else if SHEET.contains(&suffix) && have("x2t") {
        [".xlsx", ".ods", ".csv", ".pdf"].into_iter().map(|e| (e, if e == ".csv" { "first sheet" } else { "" })).collect()
    } else if SLIDES.contains(&suffix) && have("x2t") {
        plain(&[".pptx", ".odp", ".pdf"])
    } else if PAGES.contains(&suffix) && have("x2t") {
        plain(&[".pdf"])
    } else if is_model && have("assimp") || suffix == ".blend" && have("blender") && have("assimp") {
        plain(MODEL_OUT)
    } else if crate::app::ARCHIVES.contains(&mime) && have("bsdtar") {
        vec![("/", "extract into a folder")]
    } else if let Some((writable, animated)) = image {
        let mut out = if have("waifu2x") { vec![("up2", "2× bigger (AI)"), ("up4", "4× bigger (AI)")] } else { Vec::new() };
        out.extend(RASTER.iter().filter(|e| **e == ".jpg" || writable.iter().any(|w| w == &e[1..])).map(|e| (*e, "")));
        out.push((".pdf", ""));
        if animated && have("ffmpeg") {
            out.extend([(".mp4", "video"), (".webm", "video")]);
        }
        out
    } else if mime == "text/markdown" || mime == "text/html" || text {
        let mut out = vec![(".pdf", "")];
        if mime == "text/markdown" {
            out.push((".html", ""));
        }
        out
    } else {
        Vec::new()
    };
    out.into_iter().filter(|(e, label)| *e != suffix || !label.is_empty()).collect() // keeps "PDF → smaller PDF"
}

/// The word on the button and in the progress line: "PNG", "Folder".
fn format_name(code: &str) -> String {
    match code {
        "/" => "Folder".into(),
        "up2" | "up4" => "PNG".into(),
        _ => code[1..].to_uppercase(),
    }
}

/// `stem.ext` in `folder`, or `stem (2).ext`… when taken: converting never overwrites a file.
fn free_path(folder: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut candidate = folder.join(format!("{stem}{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = folder.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    candidate
}

/// ".png", or "" without an extension, lowercase.
fn suffix(path: &Path) -> String {
    path.extension().map_or(String::new(), |e| format!(".{}", e.to_string_lossy().to_lowercase()))
}

/// The name without its extension; "a.tar.gz" → "a".
fn stem(path: &Path) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tar = format!(".tar{}", suffix(path));
    match name.to_lowercase().ends_with(&tar).then(|| name.get(..name.len() - tar.len())).flatten() {
        Some(stem) => stem.to_string(),
        None => path.file_stem().unwrap_or_default().to_string_lossy().into_owned(),
    }
}

fn which(program: &str) -> bool {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).any(|dir| dir.join(program).is_file())
}

/// The photo model folder of the installed waifu2x-ncnn-vulkan.
fn waifu2x_models() -> Option<PathBuf> {
    ["/usr/share/waifu2x-ncnn-vulkan", "/usr/lib/waifu2x-ncnn-vulkan"]
        .iter()
        .flat_map(|root| ["models-upconv_7_photo", "models-cunet"].map(|name| Path::new(root).join(name)))
        .find(|p| p.is_dir())
}

fn have(tool: &str) -> bool {
    match tool {
        "x2t" => convert::x2t().is_some(),
        "blender" => model::blender_command().is_some(),
        "waifu2x" => which("waifu2x-ncnn-vulkan") && waifu2x_models().is_some(),
        tool => which(tool),
    }
}

/// ffmpeg's options for an output type.
fn ffmpeg_args(ext: &str) -> &'static [&'static str] {
    match ext {
        ".mp4" => &["-c:v", "libx264", "-preset", "veryfast", "-crf", "22", "-pix_fmt", "yuv420p",
                    "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", "-c:a", "aac", "-movflags", "+faststart"],
        ".webm" => &["-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-b:v", "0", "-crf", "32", "-c:a", "libopus"],
        ".mkv" => &["-c", "copy"], // same streams in a new box: instant, no quality loss
        ".gif" => &["-vf", "fps=12,scale=480:-1:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse", "-loop", "0"],
        ".mp3" => &["-vn", "-q:a", "2"],
        ".flac" => &[],
        _ => &["-vn"], // .ogg .opus .wav .m4a
    }
}

/// Runs `command`; the error is the last line it printed.
// ponytail: no timeout; a hung tool only keeps its worker thread busy
fn run(command: &mut Command) -> Result<(), String> {
    let out = command.stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let text = if out.stderr.is_empty() { out.stdout } else { out.stderr };
    Err(String::from_utf8_lossy(&text).trim().lines().last().unwrap_or("failed").to_string())
}

fn qt_image(src: &Path, dst: &Path) -> Result<(), String> {
    let error = helpers::convert_image(&src.to_string_lossy(), &dst.to_string_lossy());
    if error.is_empty() { Ok(()) } else { Err(error) }
}

/// Writes `path` converted to `code` next to it and returns the new path. Worker thread.
fn convert_file(path: &Path, code: &str, mime: &str) -> Result<PathBuf, String> {
    let folder = path.parent().unwrap_or(Path::new("/"));
    let (suffix, stem) = (suffix(path), stem(path));
    if let Some(scale) = code.strip_prefix("up") {
        // waifu2x reads few formats; Qt reads them all
        let dst = free_path(folder, &format!("{stem} ({scale}x)"), ".png");
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let tmp = convert::cache_dir().join("tmp").join(format!("upscale-{}-{nanos}.png", std::process::id()));
        let _ = std::fs::create_dir_all(convert::cache_dir().join("tmp"));
        let models = waifu2x_models().ok_or("waifu2x's models are missing")?;
        let result = qt_image(path, &tmp).and_then(|_| {
            run(Command::new("waifu2x-ncnn-vulkan").arg("-i").arg(&tmp).arg("-o").arg(&dst).args(["-s", scale, "-n", "1", "-m"]).arg(models))
        });
        let _ = std::fs::remove_file(&tmp);
        return result.map(|_| dst);
    }
    if code == "/" {
        let dst = free_path(folder, &stem, "");
        std::fs::create_dir(&dst).map_err(|e| e.to_string())?;
        return run(Command::new("bsdtar").arg("-xf").arg(path).arg("-C").arg(&dst)).map(|_| dst);
    }
    if mime == "application/pdf" && code == ".pdf" {
        let dst = free_path(folder, &format!("{stem} (compressed)"), ".pdf");
        // /ebook = 150 dpi images: much smaller, still sharp on screen
        let mut out = std::ffi::OsString::from("-sOutputFile=");
        out.push(&dst);
        return run(Command::new("gs").args(["-q", "-dBATCH", "-dNOPAUSE", "-dSAFER", "-sDEVICE=pdfwrite", "-dPDFSETTINGS=/ebook"]).arg(out).arg(path))
            .map(|_| dst);
    }
    if mime == "application/pdf" && code == ".png" {
        let dst = free_path(folder, &format!("{stem} pages"), "");
        std::fs::create_dir(&dst).map_err(|e| e.to_string())?;
        return run(Command::new("pdftoppm").args(["-r", "150", "-png"]).arg(path).arg(dst.join("page"))).map(|_| dst);
    }
    let dst = free_path(folder, &stem, code);
    let is_model = model::MODELS.contains(&suffix.trim_start_matches('.'));
    if mime.starts_with("audio/") || mime.starts_with("video/") || ([".mp4", ".webm"].contains(&code) && !is_model) {
        let mut command = Command::new("ffmpeg");
        command.args(["-y", "-loglevel", "error", "-i"]).arg(path).args(ffmpeg_args(code)).args(["-map_metadata", "0"]).arg(&dst);
        return run(&mut command).map(|_| dst);
    }
    let x2t_reads = [WORD, SHEET, SLIDES, PAGES, &[".pdf"]].iter().any(|list| list.contains(&suffix.as_str()));
    if let Some((_, format)) = X2T_CODES.iter().find(|(e, _)| *e == code).filter(|_| x2t_reads) {
        return convert::x2t_convert(path, &dst, *format, &convert::Runner::default()).map(|_| dst).map_err(|e| format!("x2t: {e}"));
    }
    if is_model || suffix == ".blend" {
        // Blender exports glTF once (cached for the 3D view too); assimp does the rest
        let source = if suffix == ".blend" { model::blend_to_glb(path, &|| true, &|_| {})? } else { path.to_path_buf() };
        return run(Command::new("assimp").arg("export").arg(source).arg(&dst)).map(|_| dst);
    }
    if RASTER.contains(&code) || code == ".pdf" && helpers::can_read_image(&path.to_string_lossy()) {
        return qt_image(path, &dst).map(|_| dst);
    }
    Err("no converter for this pair".into())
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

impl cxx_qt::Initialize for Tools {
    fn initialize(self: Pin<&mut Self>) {
        self.on_path_changed(|tools| tools.load()).release();
    }
}

impl qobject::Tools {
    fn load(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().job += 1; // a result belongs to the file it came from
        self.as_mut().set_status(QString::default());
        self.as_mut().set_result(QString::default());
        self.as_mut().set_busy(false);
        let path = PathBuf::from(self.path().to_string());
        let list = if path.as_os_str().is_empty() || path.is_dir() {
            Vec::new()
        } else {
            let p = path.to_string_lossy();
            let mime = helpers::mime_info(&p).name;
            let writable = helpers::writable_image_formats();
            let image = helpers::can_read_image(&p).then(|| (writable.as_slice(), helpers::image_animated(&p)));
            let text = helpers::mime_inherits(&p, "text/plain");
            targets(&mime, &suffix(&path), image, text, &have)
                .into_iter()
                .map(|(code, label)| {
                    let name = format_name(code);
                    let icon = match code {
                        "/" => "folder".into(),
                        "up2" | "up4" => "tools-wizard".into(),
                        _ => helpers::mime_icon_for_name(&format!("x{code}")),
                    };
                    serde_json::json!([code, if label.is_empty() { name } else { format!("{name}  —  {label}") }, icon])
                })
                .collect()
        };
        self.set_targets(QString::from(serde_json::Value::Array(list).to_string().as_str()));
    }

    fn start(mut self: Pin<&mut Self>, code: &QString) {
        let code = code.to_string();
        let path = PathBuf::from(self.path().to_string());
        let p = path.to_string_lossy().into_owned();
        let mime = helpers::mime_info(&p).name;
        let name = format_name(&code);
        self.as_mut().rust_mut().job += 1;
        let job = self.rust().job;
        self.as_mut().set_result(QString::default());
        let text = mime == "text/markdown" || mime == "text/html" || helpers::mime_inherits(&p, "text/plain");
        if (code == ".pdf" || code == ".html") && text && !NOT_TEXT.contains(&suffix(&path).as_str()) {
            // Qt's text engine wants the Qt thread; it's fast enough there
            let dst = free_path(path.parent().unwrap_or(Path::new("/")), &path.file_stem().unwrap_or_default().to_string_lossy(), &code);
            let ok = helpers::text_to_document(&p, &dst.to_string_lossy(), &mime);
            return self.finish(&name, if ok { Ok(dst) } else { Err("Qt could not write it".into()) });
        }
        let progress = if let Some(scale) = code.strip_prefix("up") { format!("{name}<br><small>Enlarging {scale}×…</small>") } else { format!("{name}…") };
        self.as_mut().set_status(QString::from(progress.as_str()));
        self.as_mut().set_busy(true);
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let result = convert_file(&path, &code, &mime);
            let _ = thread.queue(move |tools| {
                if tools.rust().job == job {
                    tools.finish(&name, result);
                }
            });
        });
    }

    fn finish(mut self: Pin<&mut Self>, name: &str, result: Result<PathBuf, String>) {
        self.as_mut().set_busy(false);
        let status = match &result {
            // the name is a link: click it to preview the new file here
            Ok(made) => format!("✓ <a href=\"result\">{}</a>", escape(&made.file_name().unwrap_or_default().to_string_lossy())),
            Err(error) => format!("<b>{name} failed</b><br><small>{}</small>", escape(&error.chars().take(300).collect::<String>())),
        };
        self.as_mut().set_status(QString::from(status.as_str()));
        if let Ok(made) = result {
            self.set_result(QString::from(made.to_string_lossy().as_ref()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(mime: &str, suffix: &str, image: Option<(&[String], bool)>, text: bool, missing: &str) -> Vec<&'static str> {
        targets(mime, suffix, image, text, &|tool| tool != missing).into_iter().map(|(code, _)| code).collect()
    }

    #[test]
    fn targets_per_type() {
        assert_eq!(codes("video/mp4", ".mp4", None, false, ""), [".webm", ".mkv", ".gif", ".mp3"]);
        assert!(codes("video/mp4", ".mp4", None, false, "ffmpeg").is_empty());
        assert_eq!(codes("audio/mpeg", ".mp3", None, false, ""), [".ogg", ".opus", ".flac", ".wav", ".m4a"]);
        assert_eq!(codes("application/pdf", ".pdf", None, false, ""), [".pdf", ".png", ".docx"]);
        assert_eq!(codes("application/pdf", ".pdf", None, false, "x2t"), [".pdf", ".png"]);
        assert_eq!(codes("x", ".xlsx", None, false, ""), [".ods", ".csv", ".pdf"]);
        assert!(codes("x", ".docx", None, false, "x2t").is_empty());
        assert_eq!(codes("model/stl", ".stl", None, false, ""), [".glb", ".gltf", ".obj", ".fbx", ".ply", ".dae"]);
        assert!(codes("x", ".blend", None, false, "blender").is_empty());
        assert_eq!(codes("application/zip", ".zip", None, false, ""), ["/"]);
        assert_eq!(codes("text/markdown", ".md", None, true, ""), [".pdf", ".html"]);
        assert_eq!(codes("text/x-python", ".py", None, true, ""), [".pdf"]);
        let writable = ["png", "jpeg", "webp", "gif"].map(String::from);
        assert_eq!(codes("image/gif", ".gif", Some((&writable, true), ), false, ""), ["up2", "up4", ".png", ".jpg", ".webp", ".pdf", ".mp4", ".webm"]);
        assert_eq!(codes("image/png", ".png", Some((&writable, false)), false, "waifu2x"), [".jpg", ".webp", ".pdf"]);
    }

    #[test]
    fn names_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("quickpeek-free-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_path(&dir, "a", ".png"), dir.join("a.png"));
        std::fs::write(dir.join("a.png"), "").unwrap();
        std::fs::write(dir.join("a (2).png"), "").unwrap();
        assert_eq!(free_path(&dir, "a", ".png"), dir.join("a (3).png"));
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(stem(Path::new("/x/Photos.TAR.GZ")), "Photos");
        assert_eq!(stem(Path::new("/x/clip.mp4")), "clip");
        assert_eq!(format_name(".webm"), "WEBM");
    }

    #[test]
    fn ffmpeg_options() {
        assert_eq!(ffmpeg_args(".mkv"), ["-c", "copy"]);
        assert_eq!(ffmpeg_args(".mp3"), ["-vn", "-q:a", "2"]);
        assert!(ffmpeg_args(".flac").is_empty());
        assert_eq!(ffmpeg_args(".wav"), ["-vn"]);
        assert!(ffmpeg_args(".mp4").contains(&"libx264"));
    }
}
