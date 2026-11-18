// SPDX-License-Identifier: GPL-3.0-or-later
//! AI tools on pictures: pick a subject (Segment Anything 2.1, outlines sharpened by BiRefNet / ToonOut)
//! and find text (PaddleOCR). ONNX Runtime is loaded from the system at run time (onnxruntime-cpu);
//! the models are downloaded once into ~/.local/share/quickpeek/models.

use crate::helpers;
use core::pin::Pin;
use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::QString;
use ort::environment::Environment;
use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;
use std::borrow::Cow;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        /// `lines`: found text as JSON `[{text, x, y, w, h, block}]` in picture pixels, empty until read.
        /// `overlay`: revision of `image://aioverlay` (dimming and outline of the picked subject), 0 = none.
        /// `busy`: a model is reading the picture.
        #[qobject]
        #[qml_element]
        #[qproperty(QString, status)]
        #[qproperty(bool, busy)]
        #[qproperty(QString, lines)]
        #[qproperty(i32, overlay)]
        type ImageTools = super::ImageToolsRust;

        /// ONNX Runtime is installed.
        #[qinvokable]
        fn available(self: &ImageTools) -> bool;
        /// Forgets everything about the previous picture.
        #[qinvokable]
        fn reset(self: Pin<&mut ImageTools>);
        /// `picture` is what's shown; `path` the file, which keys the cached result.
        #[qinvokable]
        #[cxx_name = "readText"]
        fn read_text(self: Pin<&mut ImageTools>, picture: &QString, path: &QString);
        #[qinvokable]
        #[cxx_name = "startSubject"]
        fn start_subject(self: Pin<&mut ImageTools>, picture: &QString);
        #[qinvokable]
        #[cxx_name = "stopSubject"]
        fn stop_subject(self: Pin<&mut ImageTools>);
        /// `mode`: "new", "add", "remove" (a click at x, y in picture pixels), "reach" (slider moved) or "undo".
        /// `reach` -8..8: select less or more.
        #[qinvokable]
        fn pick(self: Pin<&mut ImageTools>, x: f64, y: f64, mode: &QString, reach: i32);
        #[qinvokable]
        #[cxx_name = "copySubject"]
        fn copy_subject(self: Pin<&mut ImageTools>);
        /// Saves the cut-out next to `path`.
        #[qinvokable]
        #[cxx_name = "saveSubject"]
        fn save_subject(self: Pin<&mut ImageTools>, path: &QString);
        #[qinvokable]
        #[cxx_name = "copyText"]
        fn copy_text(self: &ImageTools, text: &QString);
    }

    impl cxx_qt::Threading for ImageTools {}
}

type Res<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---------------------------------------------------------------- models

/// name, file, url, size in bytes. Downloaded the first time a tool is used.
const MODELS: &[(&str, &str, &str, u64)] = &[
    // Segment Anything 2.1 (Hiera Base+), zipped: the encoder reads a picture once, the decoder turns each click into a mask
    ("sam2", "sam2.1_hiera_base_plus_20260221.zip", "https://huggingface.co/vietanhdev/segment-anything-2.1-onnx-models/resolve/main/sam2.1_hiera_base_plus_20260221.zip", 271998747),
    // BiRefNet retrained on anime and illustrations (ToonOut): outlines of drawn subjects
    ("toonout", "birefnet-toonout.onnx", "https://huggingface.co/sprited/birefnet-toonout-onnx/resolve/main/birefnet-toonout.onnx", 974344828),
    ("birefnet", "BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx", "https://github.com/danielgatis/rembg/releases/download/v0.0.0/BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx", 224005088),
    // PaddleOCR PP-OCRv5 (Apache-2.0): the detector finds lines of text, the recognizer reads Latin-script lines
    ("ocr_det", "ppocrv5-det.onnx", "https://huggingface.co/monkt/paddleocr-onnx/resolve/main/detection/v5/det.onnx", 88030804),
    ("ocr_rec", "ppocrv5-latin-rec.onnx", "https://huggingface.co/monkt/paddleocr-onnx/resolve/main/languages/latin/rec.onnx", 7862832),
    ("ocr_dict", "ppocrv5-latin-dict.txt", "https://huggingface.co/monkt/paddleocr-onnx/resolve/main/languages/latin/dict.txt", 1634),
];
const SAM2_FILES: [&str; 2] = ["sam2.1_hiera_base_plus.encoder.onnx", "sam2.1_hiera_base_plus.decoder.onnx"];
const OCR_SIDE: f64 = 1280.0; // longest side the text detector sees, in pixels
const HINT: &str = "Click what to keep · Shift+click adds · Ctrl+click cuts away · Ctrl+Z undoes";

/// Not in the cache: cleaning the cache must never delete these.
fn models_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share/quickpeek/models")
}

/// Path to a model file, downloading it first if needed. Worker thread.
fn ensure_model(name: &str, report: &dyn Fn(String)) -> Res<PathBuf> {
    // ponytail: one lock for every model, so a second tool waits for the first one's download. Per-model locks if that bites.
    static LOCK: Mutex<()> = Mutex::new(());
    let &(_, file, url, size) = MODELS.iter().find(|m| m.0 == name).expect("known model");
    let path = models_dir().join(file);
    let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if std::fs::metadata(&path).is_ok_and(|m| m.len() == size) {
        return Ok(path);
    }
    std::fs::create_dir_all(models_dir()).map_err(err)?;
    let part = path.with_file_name(format!("{file}.part")); // renamed when complete: a cut download is never used
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(std::time::Duration::from_secs(30)))
        .timeout_recv_response(Some(std::time::Duration::from_secs(30)))
        .build()
        .into();
    let mut body = agent.get(url).call().map_err(err)?.into_body().into_reader();
    let mut out = std::fs::File::create(&part).map_err(err)?;
    let (mut buffer, mut done, mut shown) = (vec![0; 1 << 20], 0u64, u64::MAX);
    loop {
        let n = body.read(&mut buffer).map_err(err)?;
        if n == 0 || done > size {
            break;
        }
        out.write_all(&buffer[..n]).map_err(err)?;
        done += n as u64;
        if done * 100 / size != shown {
            shown = done * 100 / size;
            report(format!("Downloading the AI model, once: {shown}% of {} MB", size >> 20));
        }
    }
    drop(out);
    if done != size {
        let _ = std::fs::remove_file(&part);
        return Err("the model download was cut off; try again".into());
    }
    std::fs::rename(&part, &path).map_err(err)?;
    Ok(path)
}

/// The Segment Anything 2.1 encoder and decoder, downloaded and unpacked once. Worker thread.
fn sam2_files(report: &dyn Fn(String)) -> Res<[PathBuf; 2]> {
    static LOCK: Mutex<()> = Mutex::new(());
    let paths = SAM2_FILES.map(|name| models_dir().join(name));
    let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if paths.iter().all(|p| p.exists()) {
        return Ok(paths);
    }
    let archive = ensure_model("sam2", report)?;
    report("Unpacking the AI model, once…".into());
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&archive).map_err(err)?).map_err(err)?;
    for (path, name) in paths.iter().zip(SAM2_FILES) {
        let part = path.with_file_name(format!("{name}.part"));
        std::io::copy(&mut zip.by_name(name).map_err(err)?, &mut std::fs::File::create(&part).map_err(err)?).map_err(err)?;
        std::fs::rename(&part, path).map_err(err)?;
    }
    let _ = std::fs::remove_file(&archive); // unpacked: the archive is not needed any more
    Ok(paths)
}

fn runtime() -> Res<&'static Environment> {
    static RUNTIME: OnceLock<Res<Environment>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            // Qt switches C number formatting to the system locale; ONNX Runtime under a decimal-comma locale
            // (it_IT, de_DE...) misreads some models and the text detector then finds nothing.
            unsafe extern "C" {
                fn setlocale(category: i32, locale: *const std::ffi::c_char) -> *mut std::ffi::c_char;
            }
            const LC_NUMERIC: i32 = 1; // glibc
            unsafe { setlocale(LC_NUMERIC, c"C".as_ptr()) };
            ort::init_from("libonnxruntime.so.1").map_err(err)?.with_name("quickpeek").build().map_err(err)
        })
        .as_ref()
        .map_err(Clone::clone)
}

type Model = Arc<Mutex<Session>>;
static SESSIONS: Mutex<Vec<(PathBuf, Model)>> = Mutex::new(Vec::new());

/// An ONNX Runtime session for a model file. `keep`: cached for the next picture.
fn session(path: &Path, keep: bool) -> Res<Model> {
    if let Some((_, model)) = SESSIONS.lock().unwrap().iter().find(|(p, _)| p == path) {
        return Ok(model.clone());
    }
    let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
    let model = Session::builder(runtime()?)
        .map_err(err)?
        .with_execution_providers([ort::ep::CPU::default().build()]) // no memory arena: memory goes back after each run
        .map_err(err)?
        .with_intra_op_spinning(false) // no busy-waiting when idle
        .map_err(err)?
        .with_intra_threads((cores / 2).max(1)) // half the cores: the PC stays responsive
        .map_err(err)?
        .commit_from_file(path)
        .map_err(err)?;
    let model = Arc::new(Mutex::new(model));
    if keep {
        SESSIONS.lock().unwrap().push((path.to_owned(), model.clone()));
    }
    Ok(model)
}

/// Unloads the models and hands freed memory back to the system: the window stays running hidden.
pub fn free_memory() {
    SESSIONS.lock().unwrap().clear();
    trim();
}

/// Hands freed memory back to the system: glibc keeps freed big blocks for itself otherwise.
fn trim() {
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> i32;
    }
    unsafe { malloc_trim(0) };
}

/// One output of a run, as (shape, values).
fn run(model: &Model, inputs: Vec<(Cow<'static, str>, SessionInputValue<'static>)>) -> Res<Vec<(Vec<i64>, Vec<f32>)>> {
    let mut model = model.lock().unwrap();
    let outputs = model.run(inputs).map_err(err)?;
    outputs
        .values()
        .map(|value| value.try_extract_tensor::<f32>().map(|(shape, data)| (shape.to_vec(), data.to_vec())).map_err(err))
        .collect()
}

fn tensor(name: &str, shape: Vec<i64>, data: Vec<f32>) -> Res<(Cow<'static, str>, SessionInputValue<'static>)> {
    Ok((Cow::Owned(name.to_owned()), Tensor::from_array((shape, data)).map_err(err)?.into()))
}

// ---------------------------------------------------------------- pictures

/// 0xAARRGGBB pixels, row by row, not premultiplied.
struct Image {
    w: usize,
    h: usize,
    px: Vec<u32>,
}

impl Image {
    fn read(path: &str) -> Res<Image> {
        Image::from(helpers::read_image(path)).ok_or_else(|| "can't read the picture".into())
    }

    fn from(p: helpers::Pixels) -> Option<Image> {
        (p.width > 0 && p.height > 0).then(|| Image { w: p.width as usize, h: p.height as usize, px: p.data })
    }

    fn scaled(&self, w: usize, h: usize) -> Image {
        Image::from(helpers::scale_pixels(self.w as i32, self.h as i32, &self.px, w as i32, h as i32)).expect("scaled")
    }

    fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Image {
        let px = (y..y + h).flat_map(|row| self.px[row * self.w + x..row * self.w + x + w].iter().copied()).collect();
        Image { w, h, px }
    }

    /// Turned a quarter to the left.
    fn rotated_left(&self) -> Image {
        let mut px = vec![0; self.px.len()];
        for y in 0..self.h {
            for x in 0..self.w {
                px[(self.w - 1 - x) * self.h + y] = self.px[y * self.w + x];
            }
        }
        Image { w: self.h, h: self.w, px }
    }

    /// Brightness 0..1, like Qt's grayscale conversion.
    fn gray(&self) -> Vec<f32> {
        self.px.iter().map(|p| ((p >> 16 & 255) * 11 + (p >> 8 & 255) * 16 + (p & 255) * 5) as f32 / 32.0 / 255.0).collect()
    }
}

const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

fn imagenet(channel: usize, v: f32) -> f32 {
    (v / 255.0 - MEAN[channel]) / STD[channel]
}

/// The 3×H×W planes models read, R G B (or B G R): each 0..255 value mapped by `f(channel in RGB order, value)`.
fn planes(image: &Image, bgr: bool, f: impl Fn(usize, f32) -> f32) -> Vec<f32> {
    let n = image.px.len();
    let mut out = vec![0.0; 3 * n];
    for (i, p) in image.px.iter().enumerate() {
        for c in 0..3 {
            out[if bgr { 2 - c } else { c } * n + i] = f(c, (p >> (16 - 8 * c) & 255) as f32);
        }
    }
    out
}

/// Bilinear resize of a w×h map to `to_w`×`to_h`.
fn upscale(a: &[f32], w: usize, h: usize, to_w: usize, to_h: usize) -> Vec<f32> {
    let taps = |n: usize, to: usize| -> Vec<(usize, usize, f32)> {
        (0..to)
            .map(|i| {
                let s = ((i as f32 + 0.5) * n as f32 / to as f32 - 0.5).clamp(0.0, (n - 1) as f32);
                let i0 = s as usize;
                (i0, (i0 + 1).min(n - 1), s - i0 as f32)
            })
            .collect()
    };
    let (xs, ys) = (taps(w, to_w), taps(h, to_h));
    let mut out = Vec::with_capacity(to_w * to_h);
    for &(y0, y1, fy) in &ys {
        for &(x0, x1, fx) in &xs {
            let top = a[y0 * w + x0] * (1.0 - fx) + a[y0 * w + x1] * fx;
            let bottom = a[y1 * w + x0] * (1.0 - fx) + a[y1 * w + x1] * fx;
            out.push(top * (1.0 - fy) + bottom * fy);
        }
    }
    out
}

/// Each value replaced by the average of the (2r+1)² square around it, edges repeated (running sums: fast).
fn box_blur(a: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let size = (2 * r + 1) as f64;
    let clamp = |i: isize, n: usize| i.clamp(0, n as isize - 1) as usize;
    let mut rows = vec![0f32; w * h];
    for y in 0..h {
        let row = &a[y * w..(y + 1) * w];
        let at = |i: isize| row[clamp(i, w)] as f64;
        let mut sum: f64 = (-(r as isize)..=r as isize).map(at).sum();
        for x in 0..w {
            rows[y * w + x] = (sum / size) as f32;
            sum += at(x as isize + r as isize + 1) - at(x as isize - r as isize);
        }
    }
    let mut sums = vec![0f64; w];
    for dy in -(r as isize)..=r as isize {
        let row = clamp(dy, h) * w;
        sums.iter_mut().zip(&rows[row..row + w]).for_each(|(s, v)| *s += *v as f64);
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        let (add, sub) = (clamp(y as isize + r as isize + 1, h) * w, clamp(y as isize - r as isize, h) * w);
        for x in 0..w {
            out[y * w + x] = (sums[x] / size) as f32;
            sums[x] += rows[add + x] as f64 - rows[sub + x] as f64;
        }
    }
    out
}

fn to_f32(inside: &[bool]) -> Vec<f32> {
    inside.iter().map(|&b| b as u8 as f32).collect()
}

/// A mask grown (or shrunk) by `steps` pixels.
fn grow(inside: &[bool], w: usize, h: usize, steps: usize, shrink: bool) -> Vec<bool> {
    box_blur(&to_f32(inside), w, h, steps).iter().map(|&v| if shrink { v >= 1.0 } else { v > 0.0 }).collect()
}

/// Connected areas (4 neighbours), numbered from 1 in reading order; 0 = outside. And how many.
fn components(mask: &[bool], w: usize, h: usize) -> (Vec<u32>, u32) {
    let mut labels = vec![0u32; mask.len()];
    let (mut count, mut stack) = (0, Vec::new());
    for start in 0..mask.len() {
        if !mask[start] || labels[start] != 0 {
            continue;
        }
        count += 1;
        labels[start] = count;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            for (ok, j) in [(x > 0, i.wrapping_sub(1)), (x + 1 < w, i + 1), (y > 0, i.wrapping_sub(w)), (y + 1 < h, i + w)] {
                if ok && mask[j] && labels[j] == 0 {
                    labels[j] = count;
                    stack.push(j);
                }
            }
        }
    }
    (labels, count)
}

/// Smallest (x0, y0, x1, y1) holding every value above 0.5, both ends included.
fn bounds(mask: &[f32], w: usize) -> Option<(usize, usize, usize, usize)> {
    mask.iter().enumerate().filter(|(_, v)| **v > 0.5).fold(None, |b, (i, _)| {
        let (x, y) = (i % w, i / w);
        Some(b.map_or((x, y, x, y), |(x0, y0, x1, y1)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y))))
    })
}

// ---------------------------------------------------------------- text

/// A found line: text and its box in picture pixels.
type Line = (String, [i32; 4]);

/// Areas of the detector's "text here" map: (mean score, pixel count, x0, y0, x1, y1 with the ends excluded).
fn blobs(map: &[f32], w: usize, h: usize) -> Vec<(f32, usize, usize, usize, usize, usize)> {
    let (labels, count) = components(&map.iter().map(|&v| v > 0.3).collect::<Vec<_>>(), w, h);
    let mut out = vec![(0f32, 0usize, usize::MAX, usize::MAX, 0usize, 0usize); count as usize];
    for (i, &label) in labels.iter().enumerate().filter(|(_, l)| **l > 0) {
        let b = &mut out[label as usize - 1];
        let (x, y) = (i % w, i / w);
        *b = (b.0 + map[i], b.1 + 1, b.2.min(x), b.3.min(y), b.4.max(x + 1), b.5.max(y + 1));
    }
    out.iter_mut().for_each(|b| b.0 /= b.1 as f32);
    out
}

/// Greedy CTC decoding of steps × classes odds: the best class per step, blanks (0) and repeats dropped.
/// The kept classes, and how sure the model was of them on average.
fn ctc(odds: &[f32], classes: usize) -> (Vec<usize>, f32) {
    let best: Vec<(usize, f32)> = odds
        .chunks(classes)
        .map(|step| step.iter().copied().enumerate().fold((0, f32::MIN), |b, (i, v)| if v > b.1 { (i, v) } else { b }))
        .collect();
    let kept: Vec<(usize, f32)> = best.iter().enumerate().filter(|(t, b)| b.0 != 0 && (*t == 0 || best[t - 1].0 != b.0)).map(|(_, b)| *b).collect();
    let sure = kept.iter().map(|k| k.1).sum::<f32>() / kept.len().max(1) as f32;
    (kept.into_iter().map(|k| k.0).collect(), sure)
}

/// `rect` of `image` scaled to 48 px tall (turned on its side first when `vertical`), as the BGR planes,
/// values -1..1, that PaddleOCR's recognizer reads. Returns (width, planes).
fn rec_input(image: &Image, [x, y, w, h]: [usize; 4], vertical: bool) -> (usize, Vec<f32>) {
    let mut crop = image.crop(x, y, w, h);
    if vertical {
        crop = crop.rotated_left();
    }
    let width = ((crop.w as f64 * 48.0 / crop.h.max(1) as f64).round() as usize).clamp(8, 3200);
    (width, planes(&crop.scaled(width, 48), true, |_, v| v / 127.5 - 1.0))
}

/// Lines of text in a picture, top to bottom. Worker thread. PaddleOCR: the detector marks where text is,
/// the recognizer reads each found line.
fn ocr_lines(image: &Image, report: &dyn Fn(String)) -> Res<Vec<Line>> {
    let detector = session(&ensure_model("ocr_det", report)?, true)?;
    let reader = session(&ensure_model("ocr_rec", report)?, true)?;
    let dict = std::fs::read_to_string(ensure_model("ocr_dict", report)?).map_err(err)?;
    let mut chars: Vec<&str> = std::iter::once("").chain(dict.split('\n')).collect();
    chars.pop();
    chars.push(" ");
    report("Finding text…".into());

    // Detector: the picture at most 1280 px long, sides a multiple of 32; out comes a 0..1 "text here" map
    let (width, height) = (image.w as f64, image.h as f64);
    let scale = (OCR_SIDE / width.max(height)).min(1.0);
    let side = |s: f64| ((s * scale / 32.0).round() as usize * 32).max(32);
    let (dw, dh) = (side(width), side(height));
    let pixels = planes(&image.scaled(dw, dh), true, imagenet); // PaddleOCR models were trained on BGR pictures
    let map = run(&detector, vec![tensor("x", vec![1, 3, dh as i64, dw as i64], pixels)?])?.swap_remove(0).1;
    let (sx, sy) = (dw as f64 / width, dh as f64 / height);
    let mut lines = Vec::new();
    // ponytail: straight boxes around each found line; strongly slanted text gets a loose box.
    // Upgrade path: rotated rectangles (minimum-area box) and a straightened crop.
    for (score, size, x0, y0, x1, y1) in blobs(&map, dw, dh) {
        if score < 0.6 || size < 12 {
            continue; // faint or tiny: texture, not text
        }
        // the detector marks a line's shrunken core: grow it back by area × 1.5 / perimeter, as PaddleOCR does
        let grow = size as f64 * 1.5 / (2.0 * ((y1 - y0) + (x1 - x0)) as f64);
        let left = ((x0 as f64 - grow) / sx).max(0.0).round();
        let top = ((y0 as f64 - grow) / sy).max(0.0).round();
        let right = ((x1 as f64 + grow) / sx).min(width).round();
        let bottom = ((y1 as f64 + grow) / sy).min(height).round();
        let rect = [left as usize, top as usize, ((right - left) as usize).max(1), ((bottom - top) as usize).max(1)];
        let rect = [rect[0].min(image.w - 1), rect[1].min(image.h - 1), rect[2].min(image.w - rect[0].min(image.w - 1)), rect[3].min(image.h - rect[1].min(image.h - 1))];
        let vertical = rect[3] as f64 > 1.5 * rect[2] as f64; // a vertical line of text: read it turned on its side
        let (cw, crop) = rec_input(image, rect, vertical);
        let (shape, odds) = run(&reader, vec![tensor("x", vec![1, 3, 48, cw as i64], crop)?])?.swap_remove(0);
        let (kept, sure) = ctc(&odds, *shape.last().unwrap_or(&1) as usize);
        if kept.is_empty() || sure < 0.5 {
            continue;
        }
        let text: String = kept.iter().filter_map(|&i| chars.get(i).copied()).collect();
        if !text.trim().is_empty() {
            lines.push((text.trim().to_owned(), rect.map(|v| v as i32)));
        }
    }
    lines.sort_by_key(|l| l.1[0]);
    let mut lines = join_words(lines);
    let row = |b: &[i32; 4]| ((b[1] as f64 + b[3] as f64 / 2.0) / 10.0).floor() * 10.0 + b[0] as f64 / 1e5;
    lines.sort_by(|a, b| row(&a.1).total_cmp(&row(&b.1)));
    Ok(lines)
}

/// Pieces on the same row, close together and about as tall, as one line: the detector cuts big
/// headings at wide gaps. `lines` sorted left to right.
fn join_words(lines: Vec<Line>) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    for (text, [x, y, w, h]) in lines {
        let near = out.iter_mut().find(|(_, [ox, oy, ow, oh])| {
            let overlap = (y + h).min(oy + oh) - y.max(*oy);
            let ratio = h as f64 / *oh as f64;
            let gap = (x - (ox + ow)) as f64;
            let small = h.min(*oh) as f64;
            ratio > 0.6 && ratio < 1.6 && overlap as f64 > 0.5 * small && gap > -0.2 * h as f64 && gap < 1.2 * small
        });
        match near {
            Some((other, b)) => {
                let (left, top) = (x.min(b[0]), y.min(b[1]));
                *b = [left, top, x + w - left, (y + h).max(b[1] + b[3]) - top];
                *other = format!("{other} {text}");
            }
            None => out.push((text, [x, y, w, h])),
        }
    }
    out
}

/// Lines grouped into paragraphs, as each line's paragraph number. A line joins the paragraph above it when
/// it is about as tall, starts below it within a line's height, and overlaps it sideways.
fn text_blocks(lines: &[Line]) -> Vec<usize> {
    let mut blocks: Vec<Vec<usize>> = Vec::new();
    for (i, (_, [x, y, w, h])) in lines.iter().enumerate() {
        let (x, y, w, h) = (*x as f64, *y as f64, *w as f64, *h as f64);
        let found = blocks.iter_mut().find(|block| {
            let [bx, by, bw, bh] = lines[*block.last().unwrap()].1.map(|v| v as f64);
            let (ratio, below) = (h / bh, y - (by + bh));
            ratio > 0.6 && ratio < 1.6 && below > -0.3 * h && below < h && x < bx + bw && x + w > bx
        });
        match found {
            Some(block) => block.push(i),
            None => blocks.push(vec![i]),
        }
    }
    let mut of = vec![0; lines.len()];
    for (n, block) in blocks.iter().enumerate() {
        block.iter().for_each(|&i| of[i] = n);
    }
    of
}

/// Cached by file key, so each picture is read once.
fn read_lines(picture: &str, path: &str, report: &dyn Fn(String)) -> Res<String> {
    let cached = crate::convert::cache_key(Path::new(path)).map(|key| crate::convert::cache_dir().join(format!("{key}.ocr5.json")));
    let from_cache = cached.as_ref().and_then(|c| std::fs::read_to_string(c).ok()).and_then(|text| {
        let rows: Vec<(String, [i32; 4])> = serde_json::from_str::<serde_json::Value>(&text)
            .ok()?
            .as_array()?
            .iter()
            .map(|row| {
                let b = row.get(1)?.as_array()?;
                Some((row.get(0)?.as_str()?.to_owned(), [0, 1, 2, 3].map(|i| b.get(i).and_then(|v| v.as_i64()).unwrap_or(0) as i32)))
            })
            .collect::<Option<_>>()?;
        Some(rows)
    });
    let lines = match from_cache {
        Some(lines) => {
            crate::convert::touch(cached.as_ref().unwrap());
            lines
        }
        None => {
            let lines = ocr_lines(&Image::read(picture)?, report)?;
            if let Some(cached) = &cached {
                let _ = std::fs::create_dir_all(crate::convert::cache_dir());
                let _ = std::fs::write(cached, serde_json::json!(lines).to_string());
            }
            lines
        }
    };
    let blocks = text_blocks(&lines);
    let rows: Vec<_> = lines
        .iter()
        .zip(blocks)
        .map(|((text, [x, y, w, h]), block)| serde_json::json!({"text": text, "x": x, "y": y, "w": w, "h": h, "block": block}))
        .collect();
    Ok(serde_json::Value::Array(rows).to_string())
}

// ---------------------------------------------------------------- subject

/// True for drawings and illustrations: large areas of flat or smoothly shaded colour. Photos have grain,
/// so neighbouring pixels almost always differ.
// ponytail: one brightness statistic; a very clean photo (studio, heavy denoise) passes as a drawing.
// Upgrade path: a small classifier model if that ever picks the wrong edge model.
fn looks_drawn(image: &Image) -> bool {
    let s = 512.0 / image.w.max(image.h) as f64;
    let (w, h) = (((image.w as f64 * s).round() as usize).max(2), ((image.h as f64 * s).round() as usize).max(2));
    let gray: Vec<i32> = (0..w * h)
        .map(|i| {
            let p = image.px[(i / w) * image.h / h * image.w + (i % w) * image.w / w]; // nearest pixel: keeps the grain
            ((p >> 16 & 255) * 11 + (p >> 8 & 255) * 16 + (p & 255) * 5) as i32 / 32
        })
        .collect();
    let flat = (0..h - 1)
        .flat_map(|y| (0..w - 1).map(move |x| y * w + x))
        .filter(|&i| (gray[i + 1] - gray[i]).abs() <= 1 && (gray[i + w] - gray[i]).abs() <= 1)
        .count();
    flat as f64 / ((w - 1) * (h - 1)) as f64 > 0.45
}

/// How sure the edge model is that each pixel belongs to the main subject, 0..1, at the picture's size.
/// ToonOut for drawings, BiRefNet for photos. Worker thread.
fn matte(image: &Image, report: &dyn Fn(String)) -> Res<Vec<f32>> {
    let model = session(&ensure_model(if looks_drawn(image) { "toonout" } else { "birefnet" }, report)?, false)?; // up to 1 GB: loaded per picture, then freed
    report("Clicks work now · sharpening the edges…".into());
    let input = model.lock().unwrap().inputs()[0].name().to_owned();
    let mut out = run(&model, vec![tensor(&input, vec![1, 3, 1024, 1024], planes(&image.scaled(1024, 1024), false, imagenet))?])?.swap_remove(0).1;
    let (min, max) = out.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    if min < 0.0 || max > 1.0 {
        out.iter_mut().for_each(|v| *v = 1.0 / (1.0 + (-*v).exp())); // log-odds (BiRefNet) to 0..1; ToonOut answers in 0..1 already
    }
    let (min, max) = out.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    out.iter_mut().for_each(|v| *v = (*v - min) / (max - min + 1e-8)); // 0 = background, 1 = subject
    Ok(upscale(&out, 1024, 1024, image.w, image.h))
}

/// Segment Anything 2.1: reads a picture once (a few seconds), then turns a click into the mask of the
/// object under it (a few hundredths of a second). Worker threads.
struct Segmenter {
    decoder: Model,
    features: Vec<(String, Vec<i64>, Vec<f32>)>,
    image: Image,
    guide: Vec<f32>, // full-size brightness, to snap outlines to
}

/// One picked object: its clicks (label 1 = keep here, 0 = not here) and how sure the model is that each pixel
/// belongs to it, as log-odds over the picture squeezed to 256×256. `low` is the model's own answer, `logits`
/// the same with other objects cut out of it.
struct Part {
    points: Vec<(f32, f32)>,
    labels: Vec<f32>,
    low: Vec<f32>,
    logits: Vec<f32>,
}

const LOW: usize = 256;

impl Segmenter {
    fn new(image: Image, report: &dyn Fn(String)) -> Res<Segmenter> {
        let [encoder, decoder] = sam2_files(report)?;
        let (encoder, decoder) = (session(&encoder, true)?, session(&decoder, true)?);
        report("Looking at the picture…".into());
        let pixels = planes(&image.scaled(1024, 1024), false, imagenet);
        let names: Vec<String> = encoder.lock().unwrap().outputs().iter().map(|o| o.name().to_owned()).collect();
        let outputs = run(&encoder, vec![tensor("image", vec![1, 3, 1024, 1024], pixels)?])?;
        let features = names.into_iter().zip(outputs).map(|(name, (shape, data))| (name, shape, data)).collect();
        Ok(Segmenter { decoder, features, guide: image.gray(), image })
    }

    fn part(&self, points: Vec<(f32, f32)>, labels: Vec<f32>, previous: Option<&Part>) -> Res<Part> {
        let (sx, sy) = (1024.0 / self.image.w as f32, 1024.0 / self.image.h as f32);
        let mut inputs = self.features.iter().map(|(name, shape, data)| tensor(name, shape.clone(), data.clone())).collect::<Res<Vec<_>>>()?;
        let n = points.len() as i64;
        inputs.push(tensor("point_coords", vec![1, n, 2], points.iter().flat_map(|&(x, y)| [x * sx, y * sy]).collect())?);
        inputs.push(tensor("point_labels", vec![1, n], labels.clone())?);
        inputs.push(tensor("mask_input", vec![1, 1, LOW as i64, LOW as i64], previous.map_or(vec![0.0; LOW * LOW], |p| p.low.clone()))?);
        inputs.push(tensor("has_mask_input", vec![1], vec![previous.map_or(0.0, |_| 1.0)])?);
        let mut outputs = run(&self.decoder, inputs)?;
        let (scores, masks) = (outputs.swap_remove(1).1, outputs.swap_remove(0).1);
        let mask = |i: usize| &masks[i * LOW * LOW..(i + 1) * LOW * LOW];
        let top = scores.iter().copied().fold(f32::MIN, f32::max);
        let best = if points.len() == 1 && previous.is_none() {
            // One plain click: the model offers a small part, a part and the whole object. Take the biggest
            // one it is still fairly sure of: a click on a body usually means the whole figure.
            (0..scores.len()).filter(|&i| scores[i] >= top - 0.2).max_by_key(|&i| mask(i).iter().filter(|v| **v > 0.0).count())
        } else {
            (0..scores.len()).find(|&i| scores[i] == top)
        };
        let low = mask(best.unwrap_or(0)).to_vec();
        Ok(Part { points, labels, logits: low.clone(), low })
    }

    /// A part's log-odds at a picture pixel.
    fn at(&self, logits: &[f32], (x, y): (f32, f32)) -> f32 {
        let i = |v: f32, n: usize| (((v + 0.5) * LOW as f32 / n as f32) as usize).min(LOW - 1);
        logits[i(y, self.image.h) * LOW + i(x, self.image.w)]
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    New,
    Add,
    Remove,
    Reach,
    Undo,
}

struct Pick {
    x: f32,
    y: f32,
    mode: Mode,
    bias: f32,
}

/// The picked objects after `pick`, and the mask (0..1 at the picture's size) with its overlay pixels; no mask
/// when nothing is left picked. Worker thread.
fn work(seg: &Segmenter, mut parts: Vec<Arc<Part>>, pick: &Pick, edges: Option<&[f32]>) -> Res<(Vec<Arc<Part>>, Option<(Vec<f32>, Vec<u32>)>)> {
    let p = (pick.x, pick.y);
    match pick.mode {
        Mode::New => parts = vec![Arc::new(seg.part(vec![p], vec![1.0], None)?)],
        Mode::Add => parts.push(Arc::new(seg.part(vec![p], vec![1.0], None)?)),
        Mode::Remove => {
            // "not here" for each picked object under the click: the model reshapes that object
            let mut cut = None;
            for part in parts.iter_mut().filter(|part| seg.at(&part.logits, p) > 0.0) {
                let mut new = seg.part([&part.points[..], &[p]].concat(), [&part.labels[..], &[0.0]].concat(), Some(part))?;
                if seg.at(&new.logits, p) > 0.0 {
                    // Still there: the click contradicts the part's own clicks (e.g. Ctrl+click on something
                    // added with Shift+click). Cut the object under the click out of it.
                    if cut.is_none() {
                        cut = Some(seg.part(vec![p], vec![1.0], None)?.low);
                    }
                    new.logits.iter_mut().zip(cut.as_ref().unwrap()).for_each(|(v, c)| *v = v.min(-c));
                }
                *part = Arc::new(new);
            }
            parts.retain(|part| part.logits.iter().any(|v| *v > 0.0));
        }
        Mode::Reach | Mode::Undo => {}
    }
    if parts.is_empty() {
        return Ok((parts, None));
    }
    let (w, h) = (seg.image.w, seg.image.h);
    let mut low = parts[0].logits.clone();
    for part in &parts[1..] {
        low.iter_mut().zip(&part.logits).for_each(|(a, b)| *a = a.max(*b));
    }
    let mut logits = upscale(&low, LOW, LOW, w, h);
    if parts.len() > 1 {
        // Where two objects touch, both are only half sure at the shared edge: that seam is filled
        let inside: Vec<bool> = logits.iter().map(|v| *v > 0.0).collect();
        let closed = grow(&grow(&inside, w, h, 4, false), w, h, 4, true);
        logits.iter_mut().zip(closed.iter().zip(&inside)).filter(|(_, (c, i))| **c && !**i).for_each(|(v, _)| *v = 4.0);
    }
    let reach = ((w.max(h) as f32 / 128.0).round() as usize).max(4); // about 2 of the model's 256 px pixels
    let inside: Vec<bool> = logits.iter().map(|v| v + pick.bias > 0.0).collect();
    drop(logits);
    let seeds: Vec<(f32, f32)> = parts.iter().flat_map(|part| part.points.iter().zip(&part.labels).filter(|(_, l)| **l == 1.0).map(|(p, _)| *p)).collect();
    let inside = tidy(snap_to_edges(&inside, &seg.guide, w, h, reach), w, h, &seeds);
    let mut mask = box_blur(&to_f32(&inside), w, h, ((w.max(h) as f32 / 1200.0).round() as usize).max(1));
    if let Some(edges) = edges {
        mask = sharpen(&inside, mask, edges, w, h, reach, pick.bias);
    }
    if !mask.iter().any(|v| *v > 0.5) {
        return Ok((parts, None));
    }
    let overlay = subject_layers(&mask, w, h);
    Ok((parts, Some((mask, overlay))))
}

/// The mask's outline moved onto the picture's own edges nearby (guided filter, He et al. 2010).
/// The model draws at 256 px, so on a big picture its outline is blocky and a few pixels off.
fn snap_to_edges(inside: &[bool], guide: &[f32], w: usize, h: usize, r: usize) -> Vec<bool> {
    let p = to_f32(inside);
    let blur = |a: &[f32]| box_blur(a, w, h, r);
    let (mean_i, mean_p) = (blur(guide), blur(&p));
    let corr_ip = blur(&guide.iter().zip(&p).map(|(g, p)| g * p).collect::<Vec<_>>());
    let corr_ii = blur(&guide.iter().map(|g| g * g).collect::<Vec<_>>());
    let a: Vec<f32> = (0..p.len()).map(|i| (corr_ip[i] - mean_i[i] * mean_p[i]) / (corr_ii[i] - mean_i[i] * mean_i[i] + 1e-3)).collect();
    let b: Vec<f32> = (0..p.len()).map(|i| mean_p[i] - a[i] * mean_i[i]).collect();
    let (a, b) = (blur(&a), blur(&b));
    (0..p.len()).map(|i| a[i] * guide[i] + b[i] > 0.5).collect()
}

/// Drops the specks not connected to any click in `seeds` and fills small holes.
fn tidy(inside: Vec<bool>, w: usize, h: usize, seeds: &[(f32, f32)]) -> Vec<bool> {
    let (labels, _) = components(&inside, w, h);
    let keep: Vec<u32> = seeds.iter().map(|&(x, y)| labels[(y as usize).min(h - 1) * w + (x as usize).min(w - 1)]).filter(|l| *l > 0).collect();
    let mut inside = if keep.is_empty() { inside } else { labels.iter().map(|l| keep.contains(l)).collect() };
    let (holes, count) = components(&inside.iter().map(|v| !v).collect::<Vec<_>>(), w, h);
    let mut sizes = vec![0usize; count as usize + 1];
    holes.iter().for_each(|&l| sizes[l as usize] += 1);
    let edge = (0..w).flat_map(|x| [x, (h - 1) * w + x]).chain((0..h).flat_map(|y| [y * w, y * w + w - 1]));
    edge.for_each(|i| sizes[holes[i] as usize] = usize::MAX); // holes touching the border are outside
    for (v, &l) in inside.iter_mut().zip(&holes) {
        if l > 0 && sizes[l as usize] < w * h / 1000 {
            *v = true;
        }
    }
    inside
}

/// `mask` with its outline redrawn by the edge model where the two agree. The edge model draws at 1024 px
/// with soft hair and fur; Segment Anything at 256 px. Where the edge model disagrees (it only knows
/// "main subject", so e.g. a picked pond is background to it) the selection's own outline stays.
/// `bias` is the Less/More slider (-8..8): it moves how sure the edge model must be.
fn sharpen(inside: &[bool], mut mask: Vec<f32>, edges: &[f32], w: usize, h: usize, r: usize, bias: f32) -> Vec<f32> {
    let near = box_blur(&to_f32(inside), w, h, r);
    let agree = box_blur(&edges.iter().zip(inside).map(|(e, i)| ((*e > 0.5) == *i) as u8 as f32).collect::<Vec<_>>(), w, h, 2 * r);
    let middle = 0.5 - bias / 8.0 * 0.4; // More: also keep what the edge model is less sure of
    for i in 0..mask.len() {
        if near[i] > 0.0 && near[i] < 1.0 && agree[i] > 0.8 {
            mask[i] = ((edges[i] - middle) / 0.9 + 0.5).clamp(0.0, 1.0); // its faintest guesses count as fully out: no haze
        }
    }
    mask
}

/// Premultiplied pixels over the picture: everything else dimmed, and a light outline around the subject.
// ponytail: one image, no glow and no lift animation. Add a blurred copy of the outline if missed.
fn subject_layers(mask: &[f32], w: usize, h: usize) -> Vec<u32> {
    let inside: Vec<bool> = mask.iter().map(|v| *v > 0.5).collect();
    let shrunk = grow(&inside, w, h, ((w.max(h) as f32 / 450.0).round() as usize).max(2), true); // the outline grows with the picture
    (0..mask.len())
        .map(|i| if inside[i] && !shrunk[i] { 0xd9d9d9d9 } else { (((1.0 - mask[i]) * 150.0) as u32) << 24 })
        .collect()
}

/// The picture with only the masked part visible, cropped to it: premultiplied pixels, width, height.
fn cut_out(image: &Image, mask: &[f32]) -> Option<(Vec<u32>, usize, usize)> {
    let (x0, y0, x1, y1) = bounds(mask, image.w)?;
    let mut px = Vec::with_capacity((x1 - x0 + 1) * (y1 - y0 + 1));
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (p, m) = (image.px[y * image.w + x], mask[y * image.w + x]);
            let a = (p >> 24) as f32 / 255.0 * m;
            let c = |shift: u32| (((p >> shift) & 255) as f32 * a).round() as u32;
            px.push(((a * 255.0).round() as u32) << 24 | c(16) << 16 | c(8) << 8 | c(0));
        }
    }
    Some((px, x1 - x0 + 1, y1 - y0 + 1))
}

/// `stem (subject).png` next to the file, or `stem (subject) (2).png`… when taken.
fn free_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
    let folder = path.parent().unwrap_or(Path::new("."));
    (1..)
        .map(|n| folder.join(if n == 1 { format!("{stem} (subject).png") } else { format!("{stem} (subject) ({n}).png") }))
        .find(|p| !p.exists())
        .unwrap()
}

// ---------------------------------------------------------------- the QML object

#[derive(Default)]
pub struct ImageToolsRust {
    status: QString,
    busy: bool,
    lines: QString,
    overlay: i32,
    generation: u64, // bumped by reset: results for an older picture are dropped
    reading: bool,
    loading: bool,
    segmenter: Option<Arc<Segmenter>>,
    edges: Option<Arc<Vec<f32>>>,
    parts: Vec<Arc<Part>>,
    history: Vec<Vec<Arc<Part>>>, // earlier `parts`, for Ctrl+Z
    pending: Vec<Pick>, // clicks waiting for the picture to be read, or for the previous click
    working: bool,
    active: bool, // subject picking is on
    bias: f32,
    mask: Option<Vec<f32>>,
}

impl Drop for ImageToolsRust {
    fn drop(&mut self) {
        self.segmenter = None;
        free_memory();
    }
}

type Thread = CxxQtThread<qobject::ImageTools>;

/// Status text from a worker thread, shown while the picture is still the same.
fn reporter(thread: Thread, generation: u64) -> impl Fn(String) + Send + Sync + 'static {
    move |text| {
        let _ = thread.queue(move |mut tools| {
            if tools.rust().generation == generation {
                tools.as_mut().set_status(QString::from(text.as_str()));
            }
        });
    }
}

impl qobject::ImageTools {
    fn available(&self) -> bool {
        runtime().is_ok()
    }

    fn reset(mut self: Pin<&mut Self>) {
        let rust = self.as_mut().rust_mut().get_mut();
        rust.generation += 1;
        (rust.reading, rust.loading, rust.working, rust.active) = (false, false, false, false);
        (rust.segmenter, rust.edges, rust.mask) = (None, None, None);
        rust.parts.clear();
        rust.history.clear();
        rust.pending.clear();
        trim();
        self.as_mut().set_status(QString::default());
        self.as_mut().set_busy(false);
        self.as_mut().set_lines(QString::default());
        self.as_mut().set_overlay(0);
    }

    fn update_busy(mut self: Pin<&mut Self>) {
        let busy = self.rust().reading || self.rust().loading;
        self.as_mut().set_busy(busy);
    }

    fn read_text(mut self: Pin<&mut Self>, picture: &QString, path: &QString) {
        if !self.lines().is_empty() || self.rust().reading {
            return; // already read, or reading
        }
        self.as_mut().rust_mut().reading = true;
        self.as_mut().update_busy();
        self.as_mut().set_status(QString::from("Finding text…"));
        let (picture, path, thread, generation) = (picture.to_string(), path.to_string(), self.qt_thread(), self.rust().generation);
        let report = reporter(thread.clone(), generation);
        std::thread::spawn(move || {
            let result = read_lines(&picture, &path, &report);
            let _ = thread.queue(move |mut tools| {
                if tools.rust().generation != generation {
                    return;
                }
                tools.as_mut().rust_mut().reading = false;
                tools.as_mut().update_busy();
                let status = match result {
                    Ok(json) => {
                        let count = json.matches("\"block\"").count();
                        tools.as_mut().set_lines(QString::from(json.as_str()));
                        if count > 0 { format!("{count} lines found · click one to copy it") } else { "No text found".into() }
                    }
                    Err(e) => format!("Text finding failed: {}", e.chars().take(100).collect::<String>()),
                };
                tools.as_mut().set_status(QString::from(status.as_str()));
            });
        });
    }

    fn start_subject(mut self: Pin<&mut Self>, picture: &QString) {
        self.as_mut().rust_mut().active = true;
        if self.rust().segmenter.is_some() {
            self.as_mut().set_status(QString::from(HINT));
            return;
        }
        if self.rust().loading {
            return;
        }
        self.as_mut().rust_mut().loading = true;
        self.as_mut().update_busy();
        self.as_mut().set_status(QString::from("Starting…"));
        let (picture, thread, generation) = (picture.to_string(), self.qt_thread(), self.rust().generation);
        let report = reporter(thread.clone(), generation);
        std::thread::spawn(move || {
            let result = Image::read(&picture).and_then(|image| Segmenter::new(image, &report)).map(Arc::new);
            let segmenter = result.as_ref().ok().cloned();
            let _ = thread.queue(move |mut tools| {
                if tools.rust().generation != generation {
                    return;
                }
                tools.as_mut().rust_mut().loading = false;
                tools.as_mut().update_busy();
                match result {
                    Ok(segmenter) => {
                        tools.as_mut().rust_mut().segmenter = Some(segmenter);
                        tools.as_mut().set_status(QString::from("Clicks work now · sharpening the edges…"));
                        tools.as_mut().next_pick();
                    }
                    Err(e) => tools.as_mut().set_status(QString::from(format!("Subject picking failed: {}", e.chars().take(100).collect::<String>()).as_str())),
                }
            });
            // Meanwhile the edge model studies the picture; the outline sharpens when it's done
            let Some(segmenter) = segmenter else { return };
            let edges = matte(&segmenter.image, &report);
            drop(segmenter);
            trim(); // the edge model is up to 1 GB
            let _ = thread.queue(move |mut tools| {
                if tools.rust().generation != generation {
                    return;
                }
                match edges {
                    Ok(edges) => {
                        tools.as_mut().rust_mut().edges = Some(Arc::new(edges));
                        tools.as_mut().set_status(QString::from(HINT));
                        if !tools.rust().parts.is_empty() {
                            let bias = tools.rust().bias;
                            tools.as_mut().rust_mut().pending.push(Pick { x: 0.0, y: 0.0, mode: Mode::Reach, bias }); // redraw with sharp edges
                            tools.as_mut().next_pick();
                        }
                    }
                    Err(e) => tools.as_mut().set_status(QString::from(format!("Edge sharpening failed, outlines stay rougher: {}", e.chars().take(80).collect::<String>()).as_str())),
                }
            });
        });
    }

    fn clear_subject(mut self: Pin<&mut Self>) {
        let rust = self.as_mut().rust_mut().get_mut();
        rust.parts.clear();
        rust.mask = None;
        self.as_mut().set_overlay(0);
    }

    fn stop_subject(mut self: Pin<&mut Self>) {
        self.as_mut().clear_subject();
        let rust = self.as_mut().rust_mut().get_mut();
        rust.pending.clear();
        rust.history.clear();
        rust.active = false;
        if !rust.reading {
            self.as_mut().set_status(QString::default());
        }
    }

    fn pick(mut self: Pin<&mut Self>, x: f64, y: f64, mode: &QString, reach: i32) {
        let Some(seg) = self.rust().segmenter.clone() else { return }; // still reading the picture: clicks made now are ignored
        let mode = match mode.to_string().as_str() {
            "add" => Mode::Add,
            "remove" => Mode::Remove,
            "reach" => Mode::Reach,
            "undo" => Mode::Undo,
            _ => Mode::New,
        };
        let bias = reach as f32;
        self.as_mut().rust_mut().bias = bias;
        let outside = x < 0.0 || y < 0.0 || x >= seg.image.w as f64 || y >= seg.image.h as f64;
        if matches!(mode, Mode::New | Mode::Add | Mode::Remove) {
            if outside {
                if mode == Mode::New && !self.rust().parts.is_empty() {
                    let parts = self.rust().parts.clone();
                    self.as_mut().rust_mut().history.push(parts);
                    self.as_mut().clear_subject(); // a click beside the picture lets go
                }
                return;
            }
            self.as_mut().rust_mut().pending.retain(|p| p.mode != Mode::Reach); // the click redoes the mask anyway
        }
        self.as_mut().rust_mut().pending.push(Pick { x: x as f32, y: y as f32, mode, bias });
        self.as_mut().next_pick();
    }

    /// Handles the oldest waiting click (or slider move), one at a time, off the GUI thread.
    fn next_pick(mut self: Pin<&mut Self>) {
        loop {
            let rust = self.as_mut().rust_mut().get_mut();
            if rust.working || rust.pending.is_empty() {
                return;
            }
            let Some(seg) = rust.segmenter.clone() else { return };
            let pick = rust.pending.remove(0);
            if pick.mode == Mode::Reach && rust.pending.iter().any(|p| p.mode == Mode::Reach) {
                continue; // a newer slider position is already waiting
            }
            if pick.mode == Mode::Undo {
                let Some(parts) = rust.history.pop() else { continue };
                rust.parts = parts;
                if rust.parts.is_empty() {
                    self.as_mut().clear_subject();
                    continue;
                }
            }
            rust.working = true;
            let (parts, edges, generation) = (rust.parts.clone(), rust.edges.clone(), rust.generation);
            let thread = self.qt_thread();
            std::thread::spawn(move || {
                let result = work(&seg, parts, &pick, edges.as_deref().map(|e| &e[..]));
                let _ = thread.queue(move |mut tools| {
                    if tools.rust().generation != generation {
                        return;
                    }
                    tools.as_mut().rust_mut().working = false;
                    tools.as_mut().show_subject(result, pick.mode);
                    tools.as_mut().next_pick();
                });
            });
            return;
        }
    }

    fn show_subject(mut self: Pin<&mut Self>, result: Res<(Vec<Arc<Part>>, Option<(Vec<f32>, Vec<u32>)>)>, mode: Mode) {
        let (parts, shown) = match result {
            Ok(result) => result,
            Err(e) => {
                self.as_mut().set_status(QString::from(format!("Subject picking failed: {}", e.chars().take(100).collect::<String>()).as_str()));
                return;
            }
        };
        if !self.rust().active {
            return; // picking was switched off meanwhile
        }
        let rust = self.as_mut().rust_mut().get_mut();
        if matches!(mode, Mode::New | Mode::Add | Mode::Remove) {
            let old = std::mem::take(&mut rust.parts);
            rust.history.push(old); // Ctrl+Z goes back here
            let extra = rust.history.len().saturating_sub(30);
            rust.history.drain(..extra);
        }
        match shown {
            None => self.as_mut().clear_subject(),
            Some((mask, overlay)) => {
                let (w, h) = self.rust().segmenter.as_ref().map_or((0, 0), |s| (s.image.w as i32, s.image.h as i32));
                helpers::set_ai_overlay(w, h, &overlay);
                let rust = self.as_mut().rust_mut().get_mut();
                rust.parts = parts;
                rust.mask = Some(mask);
                let revision = *self.overlay() + 1;
                self.as_mut().set_overlay(revision);
                self.as_mut().set_status(QString::from(HINT));
            }
        }
    }

    fn cut(&self) -> Option<(Vec<u32>, usize, usize)> {
        cut_out(&self.rust().segmenter.as_ref()?.image, self.rust().mask.as_ref()?)
    }

    fn copy_subject(mut self: Pin<&mut Self>) {
        if let Some((px, w, h)) = self.cut() {
            helpers::copy_image(w as i32, h as i32, &px);
            self.as_mut().set_status(QString::from("Subject copied"));
        }
    }

    fn save_subject(mut self: Pin<&mut Self>, path: &QString) {
        let Some((px, w, h)) = self.cut() else { return };
        let dst = free_path(Path::new(&path.to_string()));
        let status = if helpers::save_png(&dst.to_string_lossy(), w as i32, h as i32, &px) {
            format!("Saved {}", dst.file_name().unwrap_or_default().to_string_lossy())
        } else {
            "Couldn't write the PNG".into()
        };
        self.as_mut().set_status(QString::from(status.as_str()));
    }

    fn copy_text(&self, text: &QString) {
        helpers::copy_text(&text.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planes_normalise_and_swap_channels() {
        let image = Image { w: 2, h: 1, px: vec![0xff_ff_00_00, 0xff_00_00_ff] }; // red, blue
        let rgb = planes(&image, false, |_, v| v / 255.0);
        assert_eq!(rgb, [1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        let bgr = planes(&image, true, |_, v| v / 127.5 - 1.0);
        assert_eq!(bgr, [-1.0, 1.0, -1.0, -1.0, 1.0, -1.0]);
        assert!((imagenet(0, 255.0) - (1.0 - 0.485) / 0.229).abs() < 1e-6);
    }

    #[test]
    fn upscale_and_box_blur_keep_flat_maps_flat() {
        let up = upscale(&[0.0, 1.0], 2, 1, 4, 2);
        assert_eq!(up, [0.0, 0.25, 0.75, 1.0, 0.0, 0.25, 0.75, 1.0]);
        assert!(box_blur(&[0.5; 30], 6, 5, 2).iter().all(|v| (*v - 0.5).abs() < 1e-6));
        let dot: Vec<bool> = (0..25).map(|i| i == 12).collect();
        assert_eq!(grow(&dot, 5, 5, 1, false).iter().filter(|v| **v).count(), 9); // a 3×3 square
        assert_eq!(grow(&grow(&dot, 5, 5, 1, false), 5, 5, 1, true), dot);
    }

    #[test]
    fn mask_bounds_and_cut_out() {
        let mut mask = vec![0.0; 20];
        mask[6] = 1.0; // x 1, y 1
        mask[13] = 0.9; // x 3, y 2
        assert_eq!(bounds(&mask, 5), Some((1, 1, 3, 2)));
        assert_eq!(bounds(&[0.2; 4], 2), None);
        let image = Image { w: 5, h: 4, px: vec![0xff_ff_80_00; 20] };
        let (px, w, h) = cut_out(&image, &mask).unwrap();
        assert_eq!((w, h, px[0], px[1]), (3, 2, 0xff_ff_80_00, 0));
    }

    #[test]
    fn tidy_drops_specks_and_fills_holes() {
        // a square around a 2×2 hole, and a stray speck far away
        let (w, h) = (200, 200); // holes under 1/1000 of the picture get filled
        let mut inside = vec![false; w * h];
        for y in 5..15 {
            for x in 5..15 {
                inside[y * w + x] = !(9..11).contains(&x) || !(9..11).contains(&y);
            }
        }
        inside[30 * w + 30] = true;
        let tidied = tidy(inside, w, h, &[(6.0, 6.0)]);
        assert!(tidied[10 * w + 10], "small hole filled");
        assert!(!tidied[30 * w + 30], "speck dropped");
        assert_eq!(tidied.iter().filter(|v| **v).count(), 100);
    }

    #[test]
    fn text_map_blobs() {
        let (w, h) = (8, 4);
        let mut map = vec![0.0; w * h];
        for x in 1..5 {
            map[w + x] = 0.9;
            map[2 * w + x] = 0.7;
        }
        map[3 * w + 7] = 0.5;
        let found = blobs(&map, w, h);
        assert_eq!(found.len(), 2);
        let (score, size, x0, y0, x1, y1) = found[0];
        assert!((score - 0.8).abs() < 1e-6);
        assert_eq!((size, x0, y0, x1, y1), (8, 1, 1, 5, 3));
    }

    #[test]
    fn ctc_drops_blanks_and_repeats() {
        // classes: 0 blank, 1 "a", 2 "b"; steps: a a blank a b
        let odds = [0.1, 0.9, 0.0, 0.2, 0.8, 0.0, 0.9, 0.1, 0.0, 0.3, 0.7, 0.0, 0.0, 0.4, 0.6];
        let (kept, sure) = ctc(&odds, 3);
        assert_eq!(kept, [1, 1, 2]);
        assert!((sure - (0.9 + 0.7 + 0.6) / 3.0).abs() < 1e-6);
        assert_eq!(ctc(&[1.0, 0.0], 2).0, Vec::<usize>::new());
    }

    #[test]
    fn words_join_into_lines_and_lines_into_paragraphs() {
        let lines = join_words(vec![("Hello".into(), [0, 0, 50, 20]), ("world".into(), [60, 2, 50, 20]), ("far".into(), [400, 0, 30, 20])]);
        assert_eq!(lines[0], ("Hello world".to_string(), [0, 0, 110, 22]));
        assert_eq!(lines.len(), 2);
        let para = vec![("one".into(), [0, 0, 100, 20]), ("two".into(), [0, 25, 90, 20]), ("apart".into(), [0, 200, 90, 20])];
        assert_eq!(text_blocks(&para), [0, 0, 1]);
    }

    #[test]
    fn rotated_left_turns_a_quarter() {
        let image = Image { w: 3, h: 2, px: vec![1, 2, 3, 4, 5, 6] };
        let turned = image.rotated_left();
        assert_eq!((turned.w, turned.h, turned.px), (2, 3, vec![3, 6, 2, 5, 1, 4]));
    }
}
