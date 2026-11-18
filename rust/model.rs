// SPDX-License-Identifier: GPL-3.0-or-later
//! 3D models for ModelView.qml: what Qt's RuntimeLoader reads, the counts for the toolbar, and on worker threads
//! the slow parts: Blender turning a .blend into a .glb, and balsam making a QML scene that keeps the animations.

use crate::{convert, embedded, helpers};
use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QByteArrayBase64Options, QString, QUrl};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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
pub(crate) const MODELS: &[&str] = &["glb", "gltf", "obj", "fbx", "stl", "ply", "dae", "3ds"];

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

pub(crate) fn blender_command() -> Option<Vec<String>> {
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
pub(crate) fn blend_to_glb(path: &Path, alive: &dyn Fn() -> bool, say: &dyn Fn(&str)) -> Result<PathBuf, &'static str> {
    let key = convert::cache_key(path).ok_or("Can't read this file")?;
    let out = convert::cache_dir().join(format!("{key}.glb"));
    if out.exists() {
        convert::touch(&out);
        return Ok(out);
    }
    let blender = blender_command().ok_or("Install Blender to see this model in 3D")?;
    let part = convert::cache_dir().join(format!("{key}.part.glb")); // renamed when done, so a killed export never looks finished
    let _ = std::fs::create_dir_all(convert::cache_dir()); // in the home folder: the Blender Flatpak can't see /tmp
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
    let key = convert::cache_key(path)?;
    let out = convert::cache_dir().join(format!("{key}.scene"));
    if !out.exists() && balsam.exists() {
        let part = convert::cache_dir().join(format!("{key}.part.scene"));
        let _ = std::fs::remove_dir_all(&part);
        let mut command = Command::new(balsam);
        command.arg("-o").arg(&part).arg(path);
        if !run(command, alive) || std::fs::rename(&part, &out).is_err() {
            let _ = std::fs::remove_dir_all(&part);
            return None;
        }
    }
    convert::touch(&out);
    let qml = std::fs::read_dir(&out).ok()?.flatten().map(|e| e.path()).find(|p| suffix(p) == "qml")?;
    std::fs::read_to_string(&qml).ok()?.contains("Timeline {").then_some(qml)
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
        if let Some(png) = embedded::blend_thumbnail(&path) {
            send(Box::new(move |model| model.set_thumbnail(data_url(&png))));
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
}
