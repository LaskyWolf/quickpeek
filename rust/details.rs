// SPDX-License-Identifier: GPL-3.0-or-later
//! Backend of the side panel's details page: everything QuickPeek knows about the file.
//! The type is told on the Qt thread; the slow reading (ffprobe, pdfinfo, archives, EXIF) runs on a worker
//! that makes no Qt calls, and sizes and dates are put into words back on the Qt thread.

use crate::helpers;
use core::pin::Pin;
use cxx_qt::{Threading};
use cxx_qt_lib::QString;
use serde_json::json;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

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
        #[qproperty(QString, comment)] // the file type in words
        #[qproperty(QString, sections)] // JSON [{"t": title, "r": [[label, value, link]]}]; empty while reading
        type Details = super::DetailsRust;
    }

    impl cxx_qt::Threading for Details {}
    impl cxx_qt::Initialize for Details {}
}

use qobject::Details;

const TEXT_LIMIT: usize = 1_000_000; // same cut as the text view
const COUNT_DEADLINE: Duration = Duration::from_secs(3);
const OFFICE: &[&str] = &["docx", "xlsx", "pptx", "odt", "ods", "odp", "docm", "xlsm", "doc", "xls", "ppt", "rtf"];

#[derive(Default)]
pub struct DetailsRust {
    path: QString,
    comment: QString,
    sections: QString,
}

/// A value as the worker found it; the Qt thread turns sizes and times into words.
#[derive(Debug, PartialEq)]
enum Val {
    Text(String),
    Link(String, String),
    Size(u64, bool), // bytes, "at least"
    FileSize(u64),   // with the exact byte count
    Time(i64),
}

type Rows = Vec<(&'static str, Val)>;

fn text(s: impl Into<String>) -> Val {
    Val::Text(s.into())
}

/// The reader for the file's type, picked on the Qt thread (it needs the mime database).
enum Reader {
    Folder,
    Media,
    Pdf,
    Office,
    Font,
    Archive,
    Image(helpers::ImageHeader),
    Text,
    None,
}

fn reader_for(path: &Path, mime: &str) -> Reader {
    let p = path.to_string_lossy();
    let ext = path.extension().map_or(String::new(), |e| e.to_string_lossy().to_lowercase());
    if path.is_dir() {
        Reader::Folder
    } else if mime.starts_with("audio/") || mime.starts_with("video/") {
        Reader::Media
    } else if mime == "application/pdf" {
        Reader::Pdf
    } else if OFFICE.contains(&ext.as_str()) && read_head(path, 4) == b"PK\x03\x04" {
        Reader::Office
    } else if ["ttf", "otf", "ttc", "otc", "woff", "woff2"].contains(&ext.as_str()) || mime.starts_with("font/") {
        Reader::Font
    } else if crate::app::ARCHIVES.contains(&mime) {
        Reader::Archive
    } else if helpers::can_read_image(&p) {
        Reader::Image(helpers::image_header(&p))
    } else if helpers::mime_inherits(&p, "text/plain") {
        Reader::Text
    } else {
        Reader::None
    }
}

fn file_details(path: &Path, reader: Reader) -> Vec<(&'static str, Rows)> {
    let Ok(st) = std::fs::metadata(path) else { return Vec::new() };
    let mut general: Rows = Vec::new();
    if st.is_file() {
        general.push(("Size", Val::FileSize(st.len())));
    }
    general.push(("Folder", text(path.parent().map_or(String::new(), |p| p.display().to_string()))));
    general.push(("Modified", Val::Time(st.mtime())));
    if let Ok(born) = st.created().and_then(|t| t.duration_since(UNIX_EPOCH).map_err(std::io::Error::other)) {
        general.push(("Created", Val::Time(born.as_secs() as i64)));
    }
    general.push(("Opened", Val::Time(st.atime())));
    general.push(("Owner", text(format!("{} : {}", user_name("/etc/passwd", st.uid()), user_name("/etc/group", st.gid())))));
    general.push(("Permissions", text(file_mode(st.permissions().mode()))));
    let mut sections = vec![("General", general)];

    let (title, rows) = match reader {
        Reader::Folder => {
            let (files, folders, size, complete) = crate::browse::folder_summary(path, COUNT_DEADLINE);
            ("Contents", vec![("Files", text(files.to_string())), ("Subfolders", text(folders.to_string())), ("Total size", Val::Size(size, !complete))])
        }
        Reader::Media => ("Media", media_rows(path)),
        Reader::Pdf => ("Document", pdf_rows(&run("pdfinfo", &[path.as_os_str()]))),
        Reader::Office => ("Document", office_rows(path).unwrap_or_default()),
        Reader::Font => ("Font", std::fs::read(path).map(|d| font_rows(&d)).unwrap_or_default()),
        Reader::Archive => ("Archive", archive_rows(path, st.len())),
        Reader::Image(header) => ("Image", image_rows(path, header)),
        Reader::Text => ("Text", text_rows(&read_head(path, TEXT_LIMIT))),
        Reader::None => ("", Vec::new()),
    };
    if !rows.is_empty() {
        sections.push((title, rows));
    }
    sections
}

/// Name for a uid/gid from /etc/passwd or /etc/group; the number when it isn't there.
// ponytail: local files only, users from LDAP/systemd-homed show as numbers; getpwuid_r if that matters
fn user_name(file: &str, id: u32) -> String {
    let table = std::fs::read_to_string(file).unwrap_or_default();
    table
        .lines()
        .map(|l| l.split(':').collect::<Vec<_>>())
        .find(|f| f.len() > 2 && f[2] == id.to_string())
        .map_or(id.to_string(), |f| f[0].to_string())
}

/// "-rw-r--r--", like `ls -l`.
fn file_mode(mode: u32) -> String {
    let kind = match mode & 0o170000 {
        0o040000 => 'd',
        0o120000 => 'l',
        0o020000 => 'c',
        0o060000 => 'b',
        0o010000 => 'p',
        0o140000 => 's',
        _ => '-',
    };
    let mut out = String::from(kind);
    for (shift, special, mark) in [(6, 0o4000, 's'), (3, 0o2000, 's'), (0, 0o1000, 't')] {
        let bits = (mode >> shift) & 7;
        out.push(if bits & 4 != 0 { 'r' } else { '-' });
        out.push(if bits & 2 != 0 { 'w' } else { '-' });
        out.push(match (bits & 1 != 0, mode & special != 0) {
            (true, true) => mark,
            (false, true) => mark.to_ascii_uppercase(),
            (true, false) => 'x',
            (false, false) => '-',
        });
    }
    out
}

fn run(program: &str, args: &[&OsStr]) -> String {
    // ponytail: no timeout; a hung tool only keeps this worker busy
    Command::new(program).args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

fn read_head(path: &Path, limit: usize) -> Vec<u8> {
    let mut data = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(limit as u64).read_to_end(&mut data);
    }
    data
}

/// 3 significant digits without trailing zeros, like printf "%.3g", for frame rates.
fn g3(value: f64) -> String {
    let digits = if value.abs() < 1.0 { 1 } else { value.abs().log10().floor() as i32 + 1 };
    let s = format!("{:.*}", (3 - digits).max(0) as usize, value);
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

fn duration(seconds: f64) -> String {
    if seconds >= 3600.0 {
        format!("{}:{:02}:{:04.1}", (seconds / 3600.0) as u64, (seconds % 3600.0 / 60.0) as u64, seconds % 60.0)
    } else {
        format!("{}:{:04.1}", (seconds / 60.0) as u64, seconds % 60.0)
    }
}

fn media_rows(path: &Path) -> Rows {
    // the path is absolute, so it can't pass for an option
    let args: [&OsStr; 7] = ["-v".as_ref(), "error".as_ref(), "-print_format".as_ref(), "json".as_ref(), "-show_format".as_ref(), "-show_streams".as_ref(), path.as_os_str()];
    ffprobe_rows(&run("ffprobe", &args))
}

/// Codec, resolution, bitrate and tags from ffprobe's JSON.
fn ffprobe_rows(out: &str) -> Rows {
    let info: serde_json::Value = serde_json::from_str(out).unwrap_or_default();
    let fmt = &info["format"];
    let s = |v: &serde_json::Value| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
    let num = |v: &serde_json::Value| v.as_str().and_then(|t| t.parse::<f64>().ok()).or(v.as_f64()).unwrap_or(0.0);
    let mut rows = Rows::new();
    let seconds = num(&fmt["duration"]);
    if seconds > 0.0 {
        rows.push(("Duration", text(duration(seconds))));
    }
    if let Some(name) = fmt["format_long_name"].as_str() {
        rows.push(("Container", text(name)));
    }
    let bitrate = num(&fmt["bit_rate"]);
    if bitrate > 0.0 {
        rows.push(("Bitrate", text(format!("{} kb/s", bitrate as u64 / 1000))));
    }
    for st in info["streams"].as_array().into_iter().flatten() {
        let codec = st["codec_long_name"].as_str().or(st["codec_name"].as_str()).unwrap_or("?");
        match st["codec_type"].as_str() {
            Some("video") if st["disposition"]["attached_pic"].as_i64() != Some(1) => {
                let rate = st["avg_frame_rate"].as_str().unwrap_or("0/1");
                let (n, d) = rate.split_once('/').unwrap_or((rate, "1"));
                let (n, d) = (n.parse::<f64>().unwrap_or(0.0), d.parse::<f64>().unwrap_or(0.0));
                let fps = if d != 0.0 { format!(", {} fps", g3(n / d)) } else { String::new() };
                let fps = if n == 0.0 { String::new() } else { fps };
                rows.push(("Video", text(format!("{codec}, {} × {}{fps}", s(&st["width"]), s(&st["height"])))));
            }
            Some("audio") => {
                let layout = st["channel_layout"].as_str().map_or_else(|| format!("{} channels", s(&st["channels"])), str::to_string);
                rows.push(("Audio", text(format!("{codec}, {} kHz, {layout}", num(&st["sample_rate"]) / 1000.0))));
            }
            _ => {}
        }
    }
    let tags: HashMap<String, String> =
        fmt["tags"].as_object().into_iter().flatten().map(|(k, v)| (k.to_lowercase(), s(v))).collect();
    for (key, label) in [("title", "Title"), ("artist", "Artist"), ("album", "Album"), ("date", "Date"), ("genre", "Genre"), ("encoder", "Encoder")] {
        if let Some(v) = tags.get(key).filter(|v| !v.is_empty()) {
            rows.push((label, text(v.as_str())));
        }
    }
    rows
}

fn pdf_rows(out: &str) -> Rows {
    let fields: HashMap<&str, &str> = out.lines().filter_map(|l| l.split_once(':')).map(|(k, v)| (k, v.trim())).collect();
    [("Title", "Title"), ("Author", "Author"), ("Creator", "Made with"), ("Producer", "Producer"), ("CreationDate", "Created"),
     ("Pages", "Pages"), ("Page size", "Page size"), ("PDF version", "PDF version"), ("Encrypted", "Encrypted")]
        .into_iter()
        .filter_map(|(key, label)| fields.get(key).filter(|v| !v.is_empty()).map(|v| (label, text(*v))))
        .collect()
}

/// Author, dates and counts that Office (docProps) and OpenDocument (meta.xml) files carry.
fn office_rows(path: &Path) -> Option<Rows> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let mut found: HashMap<String, String> = HashMap::new();
    for part in ["docProps/core.xml", "docProps/app.xml", "meta.xml"] {
        let mut xml = String::new();
        if let Ok(mut file) = zip.by_name(part) {
            let _ = file.read_to_string(&mut xml);
            xml_fields(&xml, &mut found);
        }
    }
    Some(office_fields(&found))
}

/// First non-empty text of each element by local name, plus OpenDocument's document-statistic attributes.
fn xml_fields(xml: &str, found: &mut HashMap<String, String>) {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return };
    for node in doc.descendants().filter(|n| n.is_element()) {
        let name = node.tag_name().name();
        if name == "document-statistic" {
            for a in node.attributes() {
                found.entry(a.name().to_string()).or_insert_with(|| a.value().to_string());
            }
        }
        if let Some(text) = node.text().map(str::trim).filter(|t| !t.is_empty()) {
            found.entry(name.to_string()).or_insert_with(|| text.to_string());
        }
    }
}

fn office_fields(found: &HashMap<String, String>) -> Rows {
    let wanted = [("title", "Title"), ("creator", "Last author"), ("initial-creator", "Author"), ("lastModifiedBy", "Last edited by"),
        ("created", "Created"), ("creation-date", "Created"), ("modified", "Modified"), ("date", "Modified"), ("Pages", "Pages"),
        ("page-count", "Pages"), ("Words", "Words"), ("word-count", "Words"), ("Slides", "Slides"), ("Application", "Made with"),
        ("generator", "Made with")];
    let mut rows = Rows::new();
    for (key, label) in wanted {
        if let Some(v) = found.get(key).filter(|_| !rows.iter().any(|(l, _)| *l == label)) {
            let v = if matches!(label, "Created" | "Modified") { v.replace('T', " ").trim_end_matches('Z').to_string() } else { v.clone() };
            rows.push((label, text(v)));
        }
    }
    rows
}

fn archive_rows(path: &Path, packed: u64) -> Rows {
    let Some((files, unpacked)) = crate::browse::archive_totals(path) else { return Vec::new() };
    let mut rows = vec![("Files", text(files.to_string())), ("Unpacked", Val::Size(unpacked, false))];
    if unpacked > 0 {
        rows.push(("Compression", text(format!("{:.0}% smaller", 100.0 - packed as f64 * 100.0 / unpacked as f64))));
    }
    if let Ok(mut zip) = std::fs::File::open(path).map_err(zip::result::ZipError::Io).and_then(zip::ZipArchive::new) {
        let locked = (0..zip.len()).any(|i| zip.by_index_raw(i).is_ok_and(|f| f.encrypted()));
        rows.push(("Password", text(if locked { "yes" } else { "no" })));
    }
    rows
}

fn text_rows(data: &[u8]) -> Rows {
    let encoding = match std::str::from_utf8(data) {
        Ok(_) if data.starts_with(b"\xef\xbb\xbf") => "UTF-8 with BOM",
        Ok(_) => "UTF-8",
        Err(_) => "not UTF-8 (probably Windows-1252 / Latin-1)",
    };
    let newlines = data.iter().filter(|&&b| b == b'\n').count();
    let crlf = data.windows(2).filter(|w| w == b"\r\n").count();
    let endings = if crlf > 0 && newlines == crlf { "Windows (CRLF)" } else if crlf > 0 { "mixed" } else { "Linux (LF)" };
    let lines = newlines + usize::from(!data.is_empty() && !data.ends_with(b"\n"));
    let cut = if data.len() == TEXT_LIMIT { "+" } else { "" };
    vec![("Lines", text(format!("{lines}{cut}"))), ("Encoding", text(encoding)), ("Line endings", text(endings))]
}

/// Version, designer and license from the font's 'name' table (TrueType / OpenType only).
fn font_rows(data: &[u8]) -> Rows {
    let u16_at = |at: usize| data.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]) as usize);
    let u32_at = |at: usize| data.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    if !matches!(data.get(..4), Some(b"\x00\x01\x00\x00" | b"OTTO" | b"true")) {
        return Vec::new();
    }
    let tables: HashMap<&[u8], usize> = (0..u16_at(4).unwrap_or(0))
        .filter_map(|i| Some((data.get(12 + 16 * i..16 + 16 * i)?, u32_at(20 + 16 * i)?)))
        .collect(); // tag -> offset
    let mut rows = Rows::new();
    if let Some(glyphs) = tables.get(&b"maxp"[..]).and_then(|&at| u16_at(at + 4)) {
        rows.push(("Glyphs", text(glyphs.to_string())));
    }
    let Some(&base) = tables.get(&b"name"[..]) else { return rows };
    let (count, strings) = (u16_at(base + 2).unwrap_or(0), u16_at(base + 4).unwrap_or(0));
    let mut names: HashMap<usize, String> = HashMap::new();
    for i in 0..count {
        let field = |n: usize| u16_at(base + 6 + 12 * i + 2 * n);
        let (Some(platform), Some(id), Some(length), Some(offset)) = (field(0), field(3), field(4), field(5)) else { break };
        let Some(raw) = data.get(base + strings + offset..base + strings + offset + length) else { continue };
        let value = match platform {
            0 | 3 => String::from_utf16_lossy(&raw.chunks_exact(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect::<Vec<_>>()),
            1 => raw.iter().map(|&b| b as char).collect(), // old Macintosh names, read as Latin-1
            _ => continue,
        };
        names.entry(id).or_insert(value);
    }
    for (id, label) in [(5, "Version"), (9, "Designer"), (8, "Foundry"), (0, "Copyright"), (13, "License"), (14, "License URL")] {
        if let Some(v) = names.get(&id).map(|v| v.trim()).filter(|v| !v.is_empty()) {
            let v = if v.chars().count() < 300 { v.to_string() } else { v.chars().take(300).collect::<String>() + "…" };
            rows.push((label, text(v)));
        }
    }
    rows
}

/// Size and format from the header (read on the Qt thread), the color profile, and camera details from EXIF.
fn image_rows(path: &Path, header: helpers::ImageHeader) -> Rows {
    if header.width < 0 {
        return Vec::new();
    }
    let (w, h) = (header.width, header.height);
    let format = if header.format.is_empty() { "?".to_string() } else { header.format };
    let mut rows = vec![
        ("Dimensions", text(format!("{w} × {h} px"))),
        ("Megapixels", text(format!("{:.1}", w as f64 * h as f64 / 1e6))),
        ("Format", text(format)),
    ];
    if let Some(name) = icc_name(&read_head(path, 1 << 20)) {
        rows.push(("Color profile", text(name)));
    }
    if let Ok(exif) = std::fs::File::open(path).map(std::io::BufReader::new).map_err(exif::Error::Io).and_then(|mut f| exif::Reader::new().read_from_container(&mut f)) {
        rows.extend(exif_rows(&exif));
    }
    rows
}

/// Description of an ICC profile stored as it is (JPEG, WebP, TIFF): "sRGB", "Display P3".
// ponytail: PNG compresses its profile (iCCP), so PNGs show none; inflate it with flate2 if wanted
fn icc_name(data: &[u8]) -> Option<String> {
    let magic = data.windows(4).position(|w| w == b"acsp").filter(|&p| p >= 36)?; // signature at byte 36 of a profile
    let start = magic - 36;
    let size = u32::from_be_bytes(data.get(start..start + 4)?.try_into().ok()?) as usize;
    let profile = data.get(start..(start + size).min(data.len()))?;
    let u32_at = |at: usize| profile.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    // the tag table after the 128-byte header: (signature, offset, size) for each tag
    let desc = (0..u32_at(128)?).find_map(|i| (profile.get(132 + 12 * i..136 + 12 * i)? == b"desc").then(|| u32_at(136 + 12 * i))?)?;
    let name = match profile.get(desc..desc + 4)? {
        b"desc" => String::from_utf8_lossy(profile.get(desc + 12..desc + 12 + u32_at(desc + 8)?)?).into_owned(), // ICC v2: ASCII
        b"mluc" => {
            // ICC v4: UTF-16 strings by language; the first one will do
            let (length, offset) = (u32_at(desc + 20)?, u32_at(desc + 24)?);
            let raw = profile.get(desc + offset..desc + offset + length)?;
            String::from_utf16_lossy(&raw.chunks_exact(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect::<Vec<_>>())
        }
        _ => return None,
    };
    let name = name.trim_matches(['\0', ' ']).to_string();
    (!name.is_empty()).then_some(name)
}

fn exif_rows(exif: &exif::Exif) -> Rows {
    use exif::{In, Tag, Value};
    let field = |tag| exif.get_field(tag, In::PRIMARY);
    let ascii = |tag| match field(tag).map(|f| &f.value) {
        Some(Value::Ascii(v)) => v.first().map(|b| String::from_utf8_lossy(b).trim_matches(['\0', ' ']).to_string()).filter(|s| !s.is_empty()),
        _ => None,
    };
    let real = |tag| match field(tag).map(|f| &f.value) {
        Some(Value::Rational(v)) => v.first().map(|r| r.to_f64()),
        Some(Value::SRational(v)) => v.first().map(|r| r.to_f64()),
        _ => None,
    };
    let mut rows = Rows::new();
    let camera: Vec<String> = [Tag::Make, Tag::Model].into_iter().filter_map(|t| ascii(t)).collect();
    if !camera.is_empty() {
        rows.push(("Camera", text(camera.join(" "))));
    }
    if let Some(lens) = ascii(Tag::LensModel) {
        rows.push(("Lens", text(lens)));
    }
    if let Some(taken) = ascii(Tag::DateTimeOriginal).or_else(|| ascii(Tag::DateTime)) {
        rows.push(("Taken", text(taken)));
    }
    if let Some(t) = real(Tag::ExposureTime).filter(|t| *t > 0.0) {
        rows.push(("Exposure", text(if t < 1.0 { format!("1/{} s", (1.0 / t).round()) } else { format!("{t} s") })));
    }
    if let Some(f) = real(Tag::FNumber) {
        rows.push(("Aperture", text(format!("f/{f}"))));
    }
    if let Some(iso) = field(Tag::PhotographicSensitivity).and_then(|f| f.value.get_uint(0)) {
        rows.push(("ISO", text(iso.to_string())));
    }
    if let Some(mm) = real(Tag::FocalLength) {
        rows.push(("Focal length", text(format!("{mm} mm"))));
    }
    if let Some(software) = ascii(Tag::Software) {
        rows.push(("Software", text(software)));
    }
    let degrees = |tag, reference| match field(tag).map(|f| &f.value) {
        Some(Value::Rational(v)) if v.len() >= 3 => {
            let value = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
            Some(if matches!(ascii(reference).as_deref(), Some("S" | "W")) { -value } else { value })
        }
        _ => None,
    };
    if let (Some(lat), Some(lon)) = (degrees(Tag::GPSLatitude, Tag::GPSLatitudeRef), degrees(Tag::GPSLongitude, Tag::GPSLongitudeRef)) {
        let url = format!("https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=15/{lat}/{lon}");
        rows.push(("Location", Val::Link(format!("{lat:.5}, {lon:.5}"), url)));
    }
    rows
}

/// The sections as JSON for SidePanel.qml. Qt thread: sizes and dates use the locale.
fn to_json(sections: &[(&str, Rows)]) -> String {
    let rows = |rows: &Rows| -> Vec<serde_json::Value> {
        rows.iter()
            .map(|(label, value)| {
                let (shown, link) = match value {
                    Val::Text(t) => (t.clone(), String::new()),
                    Val::Link(t, url) => (t.clone(), url.clone()),
                    Val::Size(n, at_least) => (format!("{}{}", if *at_least { "at least " } else { "" }, helpers::formatted_size(*n)), String::new()),
                    Val::FileSize(n) => (format!("{} ({} bytes)", helpers::formatted_size(*n), helpers::locale_integer(*n as i64)), String::new()),
                    Val::Time(secs) => (helpers::local_time(*secs), String::new()),
                };
                json!([label, shown, link])
            })
            .collect()
    };
    serde_json::Value::Array(sections.iter().map(|(title, r)| json!({"t": title, "r": rows(r)})).collect()).to_string()
}

impl cxx_qt::Initialize for Details {
    fn initialize(self: Pin<&mut Self>) {
        self.on_path_changed(|mut details| details.as_mut().load()).release();
    }
}

impl qobject::Details {
    fn load(mut self: Pin<&mut Self>) {
        let path = PathBuf::from(self.path().to_string());
        self.as_mut().set_sections(QString::default());
        if path.as_os_str().is_empty() {
            self.as_mut().set_comment(QString::default());
            return;
        }
        let mime = helpers::mime_info(&path.to_string_lossy());
        self.as_mut().set_comment(QString::from(mime.comment.as_str()));
        let reader = reader_for(&path, &mime.name);
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let sections = file_details(&path, reader);
            let _ = thread.queue(move |mut details| {
                if PathBuf::from(details.path().to_string()) == path {
                    details.as_mut().set_sections(QString::from(to_json(&sections).as_str()));
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(rows: &Rows) -> Vec<(&str, String)> {
        rows.iter().map(|(l, v)| (*l, match v { Val::Text(t) => t.clone(), other => format!("{other:?}") })).collect()
    }

    #[test]
    fn formats_modes_durations_and_rates() {
        assert_eq!(file_mode(0o100644), "-rw-r--r--");
        assert_eq!(file_mode(0o041777), "drwxrwxrwt");
        assert_eq!(file_mode(0o104754), "-rwsr-xr--");
        assert_eq!(duration(65.25), "1:05.2");
        assert_eq!(duration(3725.0), "1:02:05.0");
        assert_eq!((g3(29.97), g3(23.976), g3(25.0), g3(59.94), g3(0.5)), ("30".into(), "24".into(), "25".into(), "59.9".into(), "0.5".into()));
    }

    #[test]
    fn reads_text_facts() {
        assert_eq!(shown(&text_rows(b"a\r\nb\r\n")), [("Lines", "2".into()), ("Encoding", "UTF-8".into()), ("Line endings", "Windows (CRLF)".into())]);
        assert_eq!(shown(&text_rows(b"a\nb\r\nc"))[0].1, "3");
        assert_eq!(shown(&text_rows(b"a\nb\r\nc"))[2].1, "mixed");
        assert_eq!(shown(&text_rows(b"\xff"))[1].1, "not UTF-8 (probably Windows-1252 / Latin-1)");
    }

    #[test]
    fn reads_ffprobe_and_pdfinfo() {
        let probe = r#"{"streams":[{"codec_type":"video","codec_long_name":"H.264","width":1920,"height":1080,"avg_frame_rate":"30000/1001"},
            {"codec_type":"audio","codec_name":"aac","sample_rate":"44100","channel_layout":"stereo"},
            {"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}}],
            "format":{"duration":"65.25","format_long_name":"QuickTime / MOV","bit_rate":"1234567","tags":{"TITLE":"Hi"}}}"#;
        assert_eq!(shown(&ffprobe_rows(probe)), [
            ("Duration", "1:05.2".into()), ("Container", "QuickTime / MOV".into()), ("Bitrate", "1234 kb/s".into()),
            ("Video", "H.264, 1920 × 1080, 30 fps".into()), ("Audio", "aac, 44.1 kHz, stereo".into()), ("Title", "Hi".into())]);
        assert!(ffprobe_rows("").is_empty());
        let info = "Title:          \nProducer:       Typst\nPages:          3\nPage size:      595 x 842 pts (A4)\n";
        assert_eq!(shown(&pdf_rows(info)), [("Producer", "Typst".into()), ("Pages", "3".into()), ("Page size", "595 x 842 pts (A4)".into())]);
    }

    #[test]
    fn reads_office_metadata() {
        let mut found = HashMap::new();
        xml_fields(r#"<cp:coreProperties xmlns:cp="x" xmlns:dc="d"><dc:title>A &amp; B</dc:title><dc:creator>Ann</dc:creator>
            <dcterms:created xmlns:dcterms="t">2024-01-02T03:04:05Z</dcterms:created></cp:coreProperties>"#, &mut found);
        xml_fields(r#"<office:meta xmlns:office="o" xmlns:meta="m"><meta:document-statistic meta:page-count="4" meta:word-count="99"/></office:meta>"#, &mut found);
        assert_eq!(shown(&office_fields(&found)), [
            ("Title", "A & B".into()), ("Last author", "Ann".into()), ("Created", "2024-01-02 03:04:05".into()),
            ("Pages", "4".into()), ("Words", "99".into())]);
    }

    #[test]
    fn reads_font_names() {
        // one table, 'name', holding a Windows (UTF-16) version string
        let mut font = b"\x00\x01\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00name\x00\x00\x00\x00\x00\x00\x00\x1c\x00\x00\x00\x00".to_vec();
        font.extend([0, 0, 0, 1, 0, 18, 0, 3, 0, 1, 4, 9, 0, 5, 0, 6, 0, 0]);
        font.extend([0, b'1', 0, b'.', 0, b'0']);
        assert_eq!(shown(&font_rows(&font)), [("Version", "1.0".into())]);
        assert!(font_rows(b"wOFF").is_empty());
    }

    #[test]
    fn finds_icc_profile_names() {
        let mut data = b"JFIF....".to_vec();
        let mut profile = vec![0u8; 128];
        profile[36..40].copy_from_slice(b"acsp");
        // two tags: 'cprt' first (a naive scan would read its neighbour's name), then 'desc' at 156
        profile.extend([0, 0, 0, 2]);
        profile.extend(b"cprt\0\0\0\0\0\0\0\0desc\0\0\0\x9c\0\0\0\x20");
        profile.extend(b"desc\0\0\0\0\0\0\0\x0esRGB IEC61966\0");
        let size = profile.len() as u32;
        profile[..4].copy_from_slice(&size.to_be_bytes());
        data.extend(profile);
        assert_eq!(icc_name(&data).as_deref(), Some("sRGB IEC61966"));
        assert_eq!(icc_name(b"no profile here"), None);
    }
}
