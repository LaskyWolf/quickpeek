// SPDX-License-Identifier: GPL-3.0-or-later
//! Pictures for files Qt's image plugins can't decode: the preview the file carries inside,
//! extracted once to ~/.cache/quickpeek, else the thumbnail Dolphin already made.
//! ImageView then shows them like any picture file, and the cache keeps revisits instant.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const BLEND_HEAD: u64 = 4_000_000; // the .blend thumbnail sits near the start; never decompress a whole scene
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

fn suffix(path: &Path) -> String {
    path.extension().map_or(String::new(), |e| e.to_string_lossy().to_lowercase())
}

/// Files whose own preview we show. Checked before Qt's plugins: kimg_kra claims Krita files
/// without a flattened picture, but can't read them.
pub fn has_preview(path: &Path) -> bool {
    match suffix(path).as_str() {
        // ponytail: layers-only Krita files show their small preview.png; a full-size `krita --export` goes through the conversion queue once it's ported
        "kra" | "krz" => File::open(path).ok().and_then(|f| zip::ZipArchive::new(f).ok())
            .is_none_or(|zip| zip.index_for_name("mergedimage.png").is_none()),
        "ora" | "blend" | "clip" | "rvt" | "af" | "ps" => true,
        _ => false,
    }
}

/// The thumbnail Dolphin already generated, largest first.
pub fn thumbnail(path: &Path) -> Option<PathBuf> {
    crate::browse::thumbnail(path, &["xx-large", "x-large", "large", "normal"])
}

/// The picture to show for `path`: its embedded preview, else Dolphin's thumbnail.
/// Reads the whole file in the worst case: call it off the UI thread.
pub fn picture(path: &Path) -> Option<PathBuf> {
    extracted(path).or_else(|| thumbnail(path))
}

fn extracted(path: &Path) -> Option<PathBuf> {
    if !has_preview(path) {
        return None;
    }
    // Changes whenever the file does, so an edited file never shows a stale preview
    let meta = path.metadata().ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    let key = format!("{:x}", md5::compute(format!("{}:{mtime}:{}", path.display(), meta.len())));
    let cache = crate::browse::cache_home().join("quickpeek");
    let out = cache.join(format!("{key}.png"));
    if !out.exists() {
        let bytes = match suffix(path).as_str() {
            "blend" => blend_thumbnail(path)?,
            "ps" => postscript(path)?,
            "clip" => clip_preview(path)?,
            "rvt" => embedded_png(path, false)?, // Revit's is 128 px
            "af" => embedded_png(path, true)?, // Affinity's is 512 px, the last one in the file
            // Krita: the flattened picture or its small preview; OpenRaster: the same names
            _ => ["mergedimage.png", "preview.png", "Thumbnails/thumbnail.png"].iter().find_map(|name| zip_entry(path, name))?,
        };
        std::fs::create_dir_all(&cache).ok()?;
        let part = out.with_extension("part.png"); // renamed when done, so a killed write never looks finished
        if std::fs::write(&part, bytes).is_ok() {
            let _ = std::fs::rename(&part, &out); // another thread may have renamed the same file first
        }
    }
    out.exists().then_some(out)
}

fn zip_entry(path: &Path, name: &str) -> Option<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(File::open(path).ok()?).ok()?;
    let mut entry = zip.by_name(name).ok()?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

fn map(path: &Path) -> Option<memmap2::Mmap> {
    // SAFETY: mapped read-only; a file truncated while we read it can crash the window, as with any mmap reader
    unsafe { memmap2::Mmap::map(&File::open(path).ok()?) }.ok()
}

/// Clip Studio files embed an SQLite database whose CanvasPreview table holds a PNG.
fn clip_preview(path: &Path) -> Option<Vec<u8>> {
    let data = map(path)?;
    let chunk = memchr::memmem::find(&data, b"CHNKSQLi")?;
    let size = usize::try_from(u64::from_be_bytes(data.get(chunk + 8..chunk + 16)?.try_into().ok()?)).ok()?;
    let db_bytes = data.get(chunk + 16..)?.get(..size)?;
    let mut db = rusqlite::Connection::open_in_memory().ok()?;
    db.deserialize_read_exact(rusqlite::MAIN_DB, db_bytes, size, true).ok()?;
    db.query_row("SELECT ImageData FROM CanvasPreview LIMIT 1", [], |row| row.get(0)).ok()
}

/// The PNG preview that Revit and Affinity files carry inside: the first one in the file, or the `last` one.
/// The documents themselves are closed formats.
fn embedded_png(path: &Path, last: bool) -> Option<Vec<u8>> {
    let data = map(path)?;
    let start = if last { memchr::memmem::rfind(&data, PNG) } else { memchr::memmem::find(&data, PNG) }?;
    let end = start + memchr::memmem::find(&data[start..], b"IEND")?;
    // ponytail: Revit splits streams into sectors; a split preview fails to decode and the view shows the icon
    Some(data.get(start..end + 8)?.to_vec())
}

/// The first page, rendered by ghostscript. Qt's EPS plugin uses ghostscript too, but
/// gives up on PostScript documents without a BoundingBox comment.
fn postscript(path: &Path) -> Option<Vec<u8>> {
    // ponytail: first page only, like the EPS plugin; all pages need a PDF conversion and the PDF view
    let out = std::process::Command::new("gs")
        .args(["-q", "-dSAFER", "-dBATCH", "-dNOPAUSE", "-sDEVICE=png16m", "-r150", "-dTextAlphaBits=4", "-dGraphicsAlphaBits=4"])
        .args(["-dFirstPage=1", "-dLastPage=1", "-sOutputFile=-"])
        .arg(path)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    (out.status.success() && out.stdout.starts_with(PNG)).then_some(out.stdout)
}

fn blend_thumbnail(path: &Path) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let mut head = [0u8; 4];
    file.read_exact(&mut head).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;
    let reader: Box<dyn Read> = match head {
        [0x28, 0xb5, 0x2f, 0xfd] => Box::new(ruzstd::decoding::StreamingDecoder::new(file).ok()?), // Blender 3.0+ "Compress"
        [0x1f, 0x8b, ..] => Box::new(flate2::read::GzDecoder::new(file)), // older Blender
        _ => Box::new(file),
    };
    let mut data = Vec::new();
    let _ = reader.take(BLEND_HEAD).read_to_end(&mut data); // a cut-off stream still has its first blocks
    let (width, height, rgba) = blend_pixels(&data).filter(|(w, h, _)| *w > 0 && *h > 0)?;
    let top_down: Vec<u8> = rgba.chunks_exact(width * 4).rev().flatten().copied().collect();
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    encoder.set_color(png::ColorType::Rgba);
    encoder.write_header().ok()?.write_image_data(&top_down).ok()?;
    Some(out)
}

/// Blender stores a thumbnail in a "TEST" block: width, height, then bottom-up RGBA pixels.
fn blend_pixels(data: &[u8]) -> Option<(usize, usize, &[u8])> {
    if !data.starts_with(b"BLENDER") {
        return None;
    }
    let v5 = data.get(7..9)? == b"17"; // Blender 5.0+: "BLENDER17-01v0500", 64-bit block headers
    let little = v5 || data[8] == b'v'; // "BLENDER-v405": '-' = 8-byte pointers, 'v' = little endian
    let num = |at: usize, len: usize| -> Option<usize> {
        let bytes = data.get(at..at.checked_add(len)?)?;
        let fold = |v: u64, b: &u8| v << 8 | *b as u64;
        let v = if little { bytes.iter().rev().fold(0, fold) } else { bytes.iter().fold(0, fold) };
        usize::try_from(v).ok()
    };
    // Block header: code, then the block length at `len_at`; the block follows the header
    let (mut pos, header, len_at, len_size) = if v5 { (17, 32, 16, 8) } else { (12, if data[7] == b'-' { 24 } else { 20 }, 4, 4) };
    while let Some(code) = data.get(pos..pos + 4) {
        let body = pos + header;
        match code {
            b"TEST" => {
                let (width, height) = (num(body, 4)?, num(body + 4, 4)?);
                let pixels = data.get(body + 8..)?.get(..width.checked_mul(height)?.checked_mul(4)?)?;
                return Some((width, height, pixels));
            }
            b"ENDB" => return None,
            _ => pos = body.checked_add(num(pos + len_at, len_size)?)?,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_blend_thumbnail() {
        // Blender 4: 8-byte pointers, little endian; one unrelated block, then a 1×2 thumbnail
        let mut old = b"BLENDER-v405".to_vec();
        old.extend(b"REND");
        old.extend(3i32.to_le_bytes());
        old.extend([0; 16]);
        old.extend([9, 9, 9]);
        old.extend(b"TEST");
        old.extend(16i32.to_le_bytes());
        old.extend([0; 16]);
        old.extend(1i32.to_le_bytes());
        old.extend(2i32.to_le_bytes());
        old.extend([1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(blend_pixels(&old), Some((1, 2, &[1, 2, 3, 4, 5, 6, 7, 8][..])));

        // Blender 5: 64-bit block length at offset 16
        let mut new = b"BLENDER17-01v0500".to_vec();
        new.extend(b"TEST");
        new.extend([0; 12]);
        new.extend(12i64.to_le_bytes());
        new.extend([0; 8]);
        new.extend(1i32.to_le_bytes());
        new.extend(1i32.to_le_bytes());
        new.extend([1, 2, 3, 4]);
        assert_eq!(blend_pixels(&new), Some((1, 1, &[1, 2, 3, 4][..])));

        assert_eq!(blend_pixels(b"BLENDER-v405ENDB"), None);
        assert_eq!(blend_pixels(b"not a blend"), None);
    }
}
