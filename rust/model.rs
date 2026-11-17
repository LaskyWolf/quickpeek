// SPDX-License-Identifier: GPL-3.0-or-later
//! 3D models for ModelView.qml: what Qt's RuntimeLoader reads, the counts for the toolbar, and on worker threads
//! the slow parts: Blender turning a .blend into a .glb, and balsam making a QML scene that keeps the animations.

use crate::helpers;
use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QByteArrayBase64Options, QString, QUrl};
use std::io::{BufRead, BufReader, Read};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

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
        #[qproperty(QUrl, model)] // what RuntimeLoader loads: the file itself, or Blender's .glb of a .blend
        #[qproperty(QUrl, scene)] // balsam's QML of the model, when it has animations
        #[qproperty(QString, thumbnail)] // a .blend's own picture as a data: URL, shown until its .glb is ready
        #[qproperty(QString, stats)] // "1.234 vertices  ·  2.000 triangles"
        #[qproperty(QString, note)] // what's happening or what's missing, over the view
        type Model = super::ModelRust;
    }

    impl cxx_qt::Threading for Model {}
    impl cxx_qt::Initialize for Model {}
}

use qobject::Model;

/// Files Qt's assimp importer reads, plus .blend through Blender.
const MODELS: &[&str] = &["glb", "gltf", "obj", "fbx", "stl", "ply", "dae", "3ds"];
const BLEND_HEAD: u64 = 4_000_000; // the .blend thumbnail sits near the start; never decompress a whole scene

#[derive(Default)]
pub struct ModelRust {
    path: QString,
    model: QUrl,
    scene: QUrl,
    thumbnail: QString,
    stats: QString,
    note: QString,
    // Bumped when the file changes: late results are dropped and running converters killed
    generation: Arc<AtomicU64>,
}

impl Drop for ModelRust {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed); // the view is gone: stop Blender too
    }
}

fn suffix(path: &Path) -> String {
    path.extension().map_or(String::new(), |e| e.to_string_lossy().to_lowercase())
}

pub fn is_model(path: &Path) -> bool {
    let suffix = suffix(path);
    suffix == "blend" || MODELS.contains(&suffix.as_str())
}

// ponytail: cache folder, key and external-tool runner belong in the shared rust/convert.rs once it lands
fn cache() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache/quickpeek")
}

/// Same key as the Python app, so both share converted files. Changes whenever the file does.
fn cache_key(path: &Path) -> Option<String> {
    let m = std::fs::metadata(path).ok()?;
    let mtime_ns = m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128;
    Some(helpers::md5_hex(&format!("{}:{mtime_ns}:{}", path.display(), m.size())))
}

/// Marks a cached file recently used, so the cache cleanup keeps it longer.
fn touch(path: &Path) {
    let _ = std::fs::File::options().append(true).open(path).and_then(|f| f.set_modified(SystemTime::now()));
}

/// Runs `command` to the end, or kills it once `alive` turns false. True if it succeeded.
fn run(mut command: Command, alive: &dyn Fn() -> bool) -> bool {
    let Ok(mut child) = command.stdout(Stdio::null()).stderr(Stdio::null()).spawn() else { return false };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if alive() => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

fn blender_command() -> Option<Vec<String>> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    if let Some(blender) = std::env::split_paths(&path).map(|dir| dir.join("blender")).find(|p| p.is_file()) {
        return Some(vec![blender.to_string_lossy().into_owned()]);
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    [PathBuf::from("/var/lib/flatpak"), home.join(".local/share/flatpak")]
        .iter()
        .any(|root| root.join("app/org.blender.Blender").exists())
        .then(|| ["flatpak", "run", "org.blender.Blender"].map(String::from).to_vec())
}

/// Blender's glTF export of a .blend, cached. Err is the note to show instead.
fn blend_to_glb(path: &Path, alive: &dyn Fn() -> bool, say: &dyn Fn(&str)) -> Result<PathBuf, &'static str> {
    let key = cache_key(path).ok_or("Can't read this file")?;
    let out = cache().join(format!("{key}.glb"));
    if out.exists() {
        touch(&out);
        return Ok(out);
    }
    let blender = blender_command().ok_or("Install Blender to see this model in 3D")?;
    let part = cache().join(format!("{key}.part.glb")); // renamed when done, so a killed export never looks finished
    let _ = std::fs::create_dir_all(cache()); // in the home folder: the Blender Flatpak can't see /tmp
    // {:?} quotes the path as a Python string literal would for any normal file name
    let script = format!("import bpy; bpy.ops.export_scene.gltf(filepath={:?}, export_format='GLB', export_apply=True)", part.to_string_lossy());
    say("Opening in Blender…");
    let mut command = Command::new(&blender[0]);
    command.args(&blender[1..]).arg("-b").arg(path).args(["--python-expr", &script]);
    if run(command, alive) && std::fs::rename(&part, &out).is_ok() {
        return Ok(out);
    }
    let _ = std::fs::remove_file(&part);
    Err(if alive() { "Blender couldn't export this file" } else { "" })
}

/// balsam's QML version of a model, which keeps its animations, or None if it has none. Cached in <key>.scene/.
fn animated_scene(path: &Path, alive: &dyn Fn() -> bool) -> Option<PathBuf> {
    let balsam = PathBuf::from(helpers::qt_binaries_dir()).join("balsam");
    let key = cache_key(path)?;
    let out = cache().join(format!("{key}.scene"));
    if !out.exists() && balsam.exists() {
        let part = cache().join(format!("{key}.part.scene"));
        let _ = std::fs::remove_dir_all(&part);
        let mut command = Command::new(balsam);
        command.arg("-o").arg(&part).arg(path);
        if !run(command, alive) || std::fs::rename(&part, &out).is_err() {
            let _ = std::fs::remove_dir_all(&part);
            return None;
        }
    }
    touch(&out);
    let qml = std::fs::read_dir(&out).ok()?.flatten().map(|e| e.path()).find(|p| suffix(p) == "qml")?;
    std::fs::read_to_string(&qml).ok()?.contains("Timeline {").then_some(qml)
}

/// Blender stores a thumbnail in a 'TEST' block: width, height, then bottom-up RGBA rows. `data` is the file's start.
fn blend_thumbnail(data: &[u8]) -> Option<(i32, i32, &[u8])> {
    if !data.starts_with(b"BLENDER") {
        return None;
    }
    let int = |at: usize, little: bool| -> Option<i64> {
        let b: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
        Some(if little { i32::from_le_bytes(b) } else { i32::from_be_bytes(b) } as i64)
    };
    let long = |at: usize| Some(i64::from_le_bytes(data.get(at..at + 8)?.try_into().ok()?));
    // Blender 5.0+: "BLENDER17-01v0500", 32-byte block headers (code, sdna, old pointer, length, count).
    // Older: "BLENDER-v405": '-' = 8-byte pointers, 'v' = little endian; code, length, old pointer, sdna, count.
    let (mut pos, head, little, new) = if data.get(7..9) == Some(b"17") {
        (17, 32, true, true)
    } else {
        let ptr = if data.get(7) == Some(&b'-') { 8 } else { 4 };
        (12, 16 + ptr, data.get(8) == Some(&b'v'), false)
    };
    while pos + head <= data.len() {
        let code = &data[pos..pos + 4];
        let size = if new { long(pos + 16)? } else { int(pos + 4, little)? };
        pos += head;
        match code {
            b"TEST" => {
                let (w, h) = (int(pos, little)?, int(pos + 4, little)?);
                let pixels = data.get(pos + 8..pos + 8 + usize::try_from(w * h * 4).ok()?)?;
                return Some((w as i32, h as i32, pixels));
            }
            b"ENDB" => return None,
            _ => pos += usize::try_from(size).ok()?,
        }
    }
    None
}

/// The first BLEND_HEAD bytes of a .blend, unpacked if Blender compressed it.
fn blend_head(path: &Path) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut magic = [0; 4];
    file.read_exact(&mut magic).ok()?;
    let file = std::io::Cursor::new(magic).chain(file);
    let reader: Box<dyn Read> = match magic {
        [0x28, 0xb5, 0x2f, 0xfd, ..] => Box::new(ruzstd::decoding::StreamingDecoder::new(file).ok()?), // zstd, Blender 3.0+
        [0x1f, 0x8b, ..] => Box::new(flate2::read::GzDecoder::new(file)), // gzip, older Blender
        _ => Box::new(file),
    };
    let mut data = Vec::new();
    reader.take(BLEND_HEAD).read_to_end(&mut data).ok()?;
    Some(data)
}

/// (count, what) for formats whose headers say so: vertices, triangles, materials...
fn counts(path: &Path) -> Vec<(u64, &'static str)> {
    let size = std::fs::metadata(path).map_or(0, |m| m.len());
    match suffix(path).as_str() {
        "glb" | "gltf" => gltf_counts(path).unwrap_or_default(),
        "obj" if size < 200_000_000 => {
            let (mut vertices, mut triangles) = (0, 0);
            let Ok(file) = std::fs::File::open(path) else { return vec![] };
            for line in BufReader::new(file).split(b'\n').map_while(Result::ok) {
                if line.starts_with(b"v ") {
                    vertices += 1;
                } else if line.starts_with(b"f ") {
                    // an n-sided face is n - 2 triangles
                    triangles += (line.split(|b| b.is_ascii_whitespace()).filter(|w| !w.is_empty()).count() as u64).saturating_sub(3);
                }
            }
            vec![(vertices, "vertices"), (triangles, "triangles")]
        }
        "stl" => {
            let Ok(data) = std::fs::read(path) else { return vec![] };
            let count = data.get(80..84).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()) as u64);
            if size == 84 + 50 * count {
                vec![(count, "triangles")] // binary STL: 50 bytes per triangle
            } else {
                vec![(data.windows(12).filter(|w| w == b"facet normal").count() as u64, "triangles")]
            }
        }
        "ply" => {
            let Ok(file) = std::fs::File::open(path) else { return vec![] };
            let mut found = vec![];
            for line in BufReader::new(file).split(b'\n').map_while(Result::ok) {
                let words: Vec<&[u8]> = line.split(|b| b.is_ascii_whitespace()).filter(|w| !w.is_empty()).collect();
                let count = || words.get(2).and_then(|n| std::str::from_utf8(n).ok()?.parse().ok());
                match words.get(..2) {
                    Some([b"element", b"vertex"]) => found.extend(count().map(|n| (n, "vertices"))),
                    Some([b"element", b"face"]) => found.extend(count().map(|n| (n, "faces"))),
                    _ if line.trim_ascii() == b"end_header" => break,
                    _ => {}
                }
            }
            found
        }
        _ => vec![],
    }
}

fn gltf_counts(path: &Path) -> Option<Vec<(u64, &'static str)>> {
    let data = std::fs::read(path).ok()?;
    let json = if data.starts_with(b"glTF") {
        // 12-byte header, then the JSON chunk: length, type, data
        let length = u32::from_le_bytes(data.get(12..16)?.try_into().ok()?) as usize;
        data.get(20..20 + length)?
    } else {
        &data
    };
    let gltf: serde_json::Value = serde_json::from_slice(json).ok()?;
    let accessors = &gltf["accessors"];
    let (mut vertices, mut triangles) = (0, 0);
    for prim in gltf["meshes"].as_array().into_iter().flatten().flat_map(|m| m["primitives"].as_array().into_iter().flatten()) {
        let count = |accessor: &serde_json::Value| accessors[accessor.as_u64().unwrap_or(u64::MAX) as usize]["count"].as_u64().unwrap_or(0);
        let positions = count(&prim["attributes"]["POSITION"]);
        vertices += positions;
        if prim["mode"].as_u64().unwrap_or(4) == 4 {
            // 4 = triangles
            triangles += if prim["indices"].is_null() { positions } else { count(&prim["indices"]) } / 3;
        }
    }
    let len = |key: &str| gltf[key].as_array().map_or(0, |a| a.len() as u64);
    let mut found = vec![(vertices, "vertices"), (triangles, "triangles"), (len("materials"), "materials"), (len("images"), "textures")];
    match len("animations") {
        0 => {}
        1 => found.push((1, "animation")),
        n => found.push((n, "animations")),
    }
    Some(found)
}

fn data_url(png: &[u8]) -> QString {
    let base64 = QByteArray::from(png).to_base64(QByteArrayBase64Options::default());
    QString::from(format!("data:image/png;base64,{}", String::from_utf8_lossy(base64.as_slice())).as_str())
}

fn url(path: &Path) -> QUrl {
    QUrl::from_local_file(&QString::from(path.to_string_lossy().as_ref()))
}

fn load(thread: cxx_qt::CxxQtThread<Model>, path: PathBuf, generation: Arc<AtomicU64>) {
    let mine = generation.load(Ordering::Relaxed);
    let alive = || generation.load(Ordering::Relaxed) == mine;
    // Runs `set` on the Qt thread, unless another file was opened meanwhile
    let send = |set: Box<dyn FnOnce(Pin<&mut Model>) + Send>| {
        let _ = thread.queue(move |model| {
            if model.rust().generation.load(Ordering::Relaxed) == mine {
                set(model)
            }
        });
    };
    let say = |note: &str| {
        let note = QString::from(note);
        send(Box::new(move |model| model.set_note(note)));
    };

    let model = if suffix(&path) == "blend" {
        if let Some((w, h, pixels)) = blend_head(&path).as_deref().and_then(blend_thumbnail) {
            let png = helpers::rgba_bottom_up_png(pixels, w, h);
            if !png.is_empty() {
                send(Box::new(move |model| model.set_thumbnail(data_url(&png))));
            }
        }
        match blend_to_glb(&path, &alive, &say) {
            Ok(glb) => glb,
            Err(note) => return say(note),
        }
    } else {
        path
    };
    let (source, stats_of) = (url(&model), model.clone());
    send(Box::new(move |mut model| {
        model.as_mut().set_note(QString::default());
        model.set_model(source);
    }));

    let counts = counts(&stats_of);
    let stats = counts.iter().map(|(n, what)| format!("{} {what}", helpers::locale_number(*n as f64))).collect::<Vec<_>>().join("  ·  ");
    send(Box::new(move |model| model.set_stats(QString::from(stats.as_str()))));

    // Only formats that can carry animations; glTF says in its header whether it does
    let animated = match suffix(&model).as_str() {
        "glb" | "gltf" => counts.iter().any(|(_, what)| what.starts_with("animation")),
        "fbx" | "dae" => true,
        _ => false,
    };
    if let Some(qml) = animated.then(|| animated_scene(&model, &alive)).flatten() {
        let scene = url(&qml);
        send(Box::new(move |model| model.set_scene(scene)));
    }
}

impl cxx_qt::Initialize for Model {
    fn initialize(self: Pin<&mut Self>) {
        self.on_path_changed(|mut model| {
            model.rust().generation.fetch_add(1, Ordering::Relaxed);
            model.as_mut().set_model(QUrl::default());
            model.as_mut().set_scene(QUrl::default());
            model.as_mut().set_thumbnail(QString::default());
            model.as_mut().set_stats(QString::default());
            model.as_mut().set_note(QString::default());
            let (thread, path, generation) = (model.qt_thread(), PathBuf::from(model.path().to_string()), model.rust().generation.clone());
            if !path.as_os_str().is_empty() {
                std::thread::spawn(move || load(thread, path, generation));
            }
        })
        .release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str, data: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("quickpeek-model-test-{}-{name}", std::process::id()));
        std::fs::write(&path, data).unwrap();
        path
    }

    #[test]
    fn counts_obj_stl_ply_gltf() {
        let obj = scratch("a.obj", b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\nf 1 2 3\n");
        assert_eq!(counts(&obj), vec![(4, "vertices"), (3, "triangles")]);

        let mut stl = vec![0; 80];
        stl.extend(2u32.to_le_bytes());
        stl.extend([0; 100]);
        assert_eq!(counts(&scratch("b.stl", &stl)), vec![(2, "triangles")]);
        let ascii = b"solid x\nfacet normal 0 0 1\nendfacet\nfacet normal 0 0 1\nendfacet\nendsolid\n";
        assert_eq!(counts(&scratch("c.stl", ascii)), vec![(2, "triangles")]);

        let ply = b"ply\nformat ascii 1.0\nelement vertex 8\nproperty float x\nelement face 6\nend_header\nelement face 99\n";
        assert_eq!(counts(&scratch("d.ply", ply)), vec![(8, "vertices"), (6, "faces")]);

        let gltf = br#"{"accessors":[{"count":4},{"count":6}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1},{"attributes":{"POSITION":0},"mode":1}]}],"materials":[{}],"animations":[{},{}]}"#;
        let want = vec![(8, "vertices"), (2, "triangles"), (1, "materials"), (0, "textures"), (2, "animations")];
        assert_eq!(counts(&scratch("e.gltf", gltf)), want);
        let mut glb = b"glTF\x02\0\0\0\0\0\0\0".to_vec();
        glb.extend((gltf.len() as u32).to_le_bytes());
        glb.extend(b"JSON");
        glb.extend(gltf);
        assert_eq!(counts(&scratch("f.glb", &glb)), want);
        for name in ["a.obj", "b.stl", "c.stl", "d.ply", "e.gltf", "f.glb"] {
            let _ = std::fs::remove_file(scratch(name, b""));
        }
    }

    #[test]
    fn finds_blend_thumbnail() {
        let pixels = [7u8; 2 * 1 * 4];
        // Pre-5.0 file, 8-byte pointers, little endian: a skipped block, then TEST
        let mut old = b"BLENDER-v405".to_vec();
        old.extend(b"REND"); old.extend(4i32.to_le_bytes()); old.extend([0; 8 + 4 + 4]); old.extend([1; 4]);
        old.extend(b"TEST"); old.extend(16i32.to_le_bytes()); old.extend([0; 8 + 4 + 4]);
        old.extend(2i32.to_le_bytes()); old.extend(1i32.to_le_bytes()); old.extend(pixels);
        assert_eq!(blend_thumbnail(&old), Some((2, 1, &pixels[..])));
        // 5.0+: 64-bit lengths at offset 16 of a 32-byte header
        let mut new = b"BLENDER17-01v0500".to_vec();
        new.extend(b"TEST"); new.extend([0; 12]); new.extend(16i64.to_le_bytes()); new.extend([0; 8]);
        new.extend(2i32.to_le_bytes()); new.extend(1i32.to_le_bytes()); new.extend(pixels);
        assert_eq!(blend_thumbnail(&new), Some((2, 1, &pixels[..])));
        assert_eq!(blend_thumbnail(b"BLENDER-v405ENDB"), None);
        assert_eq!(blend_thumbnail(b"not a blend"), None);
    }
}
