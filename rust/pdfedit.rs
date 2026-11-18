// SPDX-License-Identifier: GPL-3.0-or-later
//! Saves the PDF editor's drawings as real annotations (ink, line, square, circle, highlight, free text),
//! so any PDF app can still move or delete them. Each one carries its own appearance stream:
//! viewers that don't draw a type themselves still show it.

use cxx_qt_lib::QString;
use lopdf::{Dictionary, Document, IncrementalDocument, Object, ObjectId, Stream, StringFormat, dictionary};
use serde_json::Value;
use std::path::Path;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type PdfAnnotator = super::PdfAnnotatorRust;

        /// Writes `drawings` (JSON from PdfEditor.qml) into `source` and saves the result as `target`,
        /// which may be `source` itself: the drawings then belong to the file. Returns an error message, empty when it worked.
        #[qinvokable]
        fn save(self: &PdfAnnotator, source: &QString, drawings: &QString, target: &QString) -> QString;
    }
}

#[derive(Default)]
pub struct PdfAnnotatorRust;

impl qobject::PdfAnnotator {
    fn save(&self, source: &QString, drawings: &QString, target: &QString) -> QString {
        // ponytail: saves on the UI thread; move to a worker if big PDFs make the window stall
        let result = std::fs::read(source.to_string())
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                let drawings = serde_json::from_str::<Vec<Value>>(&drawings.to_string()).map_err(|error| error.to_string())?;
                annotated(bytes, &drawings)
            })
            .and_then(|pdf| replace(Path::new(&target.to_string()), &pdf).map_err(|error| error.to_string()));
        QString::from(&result.err().unwrap_or_default())
    }
}

/// Writes through a temporary file, so a failed save never leaves half a PDF where the old one was.
fn replace(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let part = target.with_file_name(format!(".{name}.part"));
    std::fs::write(&part, bytes)?;
    if let Ok(old) = std::fs::metadata(target) {
        std::fs::set_permissions(&part, old.permissions())?;
    }
    std::fs::rename(&part, target).inspect_err(|_| _ = std::fs::remove_file(&part))
}

/// The PDF `bytes` with the drawings appended as an incremental update: the original bytes stay as they are,
/// so signatures and everything we don't understand survive, and encrypted files stay encrypted.
pub fn annotated(bytes: Vec<u8>, drawings: &[Value]) -> Result<Vec<u8>, String> {
    let doc = Document::load_mem(&bytes).map_err(|error| error.to_string())?;
    if doc.is_encrypted() {
        // many "protected" PDFs only restrict editing and open with an empty password; this one didn't
        return Err("this PDF needs a password".into());
    }
    let pages = doc.get_pages();
    let mut pdf = IncrementalDocument::create_from(bytes, doc);
    for drawing in drawings {
        let number = drawing["page"].as_u64().unwrap_or(0) as u32 + 1;
        let &page = pages.get(&number).ok_or("the drawing's page is missing")?;
        let to_pdf = page_matrix(pdf.get_prev_documents(), page);
        for (mut annot, appearance) in annotations(drawing, &to_pdf) {
            let appearance = pdf.new_document.add_object(appearance); // a stream is always its own object
            annot.set("AP", dictionary! { "N" => appearance });
            annot.set("P", page);
            let id = pdf.new_document.add_object(annot);
            attach(&mut pdf, page, id).map_err(|error| error.to_string())?;
        }
    }
    let mut out = Vec::new();
    pdf.save_to(&mut out).map_err(|error| error.to_string())?;
    Ok(out)
}

fn attach(pdf: &mut IncrementalDocument, page: ObjectId, annot: ObjectId) -> lopdf::Result<()> {
    pdf.opt_clone_object_to_new_document(page)?;
    let annots = pdf.new_document.get_dictionary(page)?.get(b"Annots").ok().cloned();
    if let Some(Object::Reference(list)) = annots {
        pdf.opt_clone_object_to_new_document(list)?;
        if let Object::Array(items) = pdf.new_document.get_object_mut(list)? {
            items.push(annot.into());
            return Ok(());
        }
    }
    let page = pdf.new_document.get_dictionary_mut(page)?;
    match page.get_mut(b"Annots") {
        Ok(Object::Array(items)) => items.push(annot.into()),
        _ => page.set("Annots", vec![Object::from(annot)]),
    }
    Ok(())
}

/// A page key, looked up the page tree when the page doesn't set it (Rotate, CropBox, MediaBox inherit).
fn inherited<'a>(doc: &'a Document, page: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut node = doc.get_dictionary(page).ok()?;
    for _ in 0..32 {
        if let Ok(value) = node.get(key) {
            return doc.dereference(value).ok().map(|(_, value)| value);
        }
        node = doc.get_dictionary(node.get(b"Parent").ok()?.as_reference().ok()?).ok()?;
    }
    None
}

/// `[a b c d e f]` taking a point on the page as shown (points, y down from the top-left corner,
/// rotation applied, like QtPdf) to the page's own PDF coordinates.
type Matrix = [f64; 6];

fn page_matrix(doc: &Document, page: ObjectId) -> Matrix {
    let number = |object: &Object| doc.dereference(object).ok().and_then(|(_, n)| n.as_float().ok()).unwrap_or(0.0) as f64;
    let rect = ["CropBox", "MediaBox"]
        .iter()
        .find_map(|key| match inherited(doc, page, key.as_bytes()) {
            Some(Object::Array(r)) if r.len() == 4 => Some([number(&r[0]), number(&r[1]), number(&r[2]), number(&r[3])]),
            _ => None,
        })
        .unwrap_or([0.0, 0.0, 612.0, 792.0]);
    let (x0, y0, x1, y1) = (rect[0].min(rect[2]), rect[1].min(rect[3]), rect[0].max(rect[2]), rect[1].max(rect[3]));
    let rotate = inherited(doc, page, b"Rotate").and_then(|r| r.as_i64().ok()).unwrap_or(0).rem_euclid(360);
    match rotate {
        90 => [0.0, 1.0, 1.0, 0.0, x0, y0],
        180 => [-1.0, 0.0, 0.0, 1.0, x1, y0],
        270 => [0.0, -1.0, -1.0, 0.0, x1, y1],
        _ => [1.0, 0.0, 0.0, -1.0, x0, y1],
    }
}

fn map(m: &Matrix, (x, y): (f64, f64)) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

fn pair(value: &Value) -> (f64, f64) {
    (value[0].as_f64().unwrap_or(0.0), value[1].as_f64().unwrap_or(0.0))
}

fn numbers(value: &Value) -> Vec<f64> {
    value.as_array().map(|list| list.iter().filter_map(Value::as_f64).collect()).unwrap_or_default()
}

/// Line width factor: a light touch draws at 0.15×, a full press at 2× the chosen width.
pub fn pressure_width(pressure: f64) -> f64 {
    0.15 + 1.85 * pressure
}

/// Splits a tablet stroke into (points, width) runs of about the same width: a PDF ink line has one width.
pub fn pressure_runs(points: &[(f64, f64)], pressures: &[f64], width: f64) -> Vec<(Vec<(f64, f64)>, f64)> {
    let level = |pressure: f64| (pressure_width(pressure) * 4.0).round() / 4.0;
    let (Some(&first), Some(&pressure)) = (points.first(), pressures.first()) else { return vec![] };
    let mut runs = vec![(vec![first], level(pressure))];
    for (&point, &pressure) in points.iter().zip(pressures).skip(1) {
        let run = runs.last_mut().unwrap();
        run.0.push(point);
        if level(pressure) != run.1 {
            runs.push((vec![point], level(pressure))); // starts where the last run ends: no gap
        }
    }
    runs.into_iter().filter(|(run, _)| run.len() > 1).map(|(run, factor)| (run, width * factor)).collect()
}

/// The annotations for one drawing, each with the picture viewers draw for it: one, or a few for a pressure stroke.
fn annotations(drawing: &Value, to_pdf: &Matrix) -> Vec<(Dictionary, Stream)> {
    let kind = drawing["kind"].as_str().unwrap_or("");
    let rgb = numbers(&drawing["color"]);
    let rgb = [0, 1, 2].map(|i| rgb.get(i).copied().unwrap_or(0.0));
    let color = format!("{:.3} {:.3} {:.3}", rgb[0], rgb[1], rgb[2]);
    let width = drawing["width"].as_f64().unwrap_or(1.0);
    let points: Vec<(f64, f64)> = drawing["points"].as_array().map(|p| p.iter().map(pair).collect()).unwrap_or_default();
    let rect = numbers(&drawing["rect"]);
    let corners = |r: &[f64]| [(r[0], r[1]), (r[2], r[1]), (r[0], r[3]), (r[2], r[3])].map(|p| map(to_pdf, p));
    let mut results = vec![];
    let mut annot = |subtype: &str, area: [f64; 4], pad: f64, ap: String, resources: Dictionary, extra: Dictionary| {
        let area = [area[0] - pad, area[1] - pad, area[2] + pad, area[3] + pad];
        let appearance = Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => area.map(Object::from).to_vec(), "Resources" => resources },
            ap.into_bytes(),
        );
        let mut dict = dictionary! {
            "Type" => "Annot",
            "Subtype" => Object::Name(subtype.into()),
            "Rect" => area.map(Object::from).to_vec(),
            "F" => 4, // print it too
            "C" => rgb.map(Object::from).to_vec(),
        };
        dict.extend(&extra);
        results.push((dict, appearance));
    };
    let border = |w: f64| dictionary! { "BS" => dictionary! { "W" => w } };
    match kind {
        "rect" | "ellipse" if rect.len() == 4 => {
            let c = corners(&rect);
            let area = bounds(&c);
            // the stroke's middle runs on the drawn rectangle; PDF borders sit inside the annotation's box
            let [x0, y0, x1, y1] = area;
            let path = if kind == "rect" {
                format!("{x0:.2} {y0:.2} {:.2} {:.2} re", x1 - x0, y1 - y0)
            } else {
                ellipse(area)
            };
            let ap = format!("{width:.2} w {color} RG {path} S");
            annot(if kind == "rect" { "Square" } else { "Circle" }, area, width / 2.0, ap, Dictionary::new(), border(width));
        }
        "arrow" if points.len() == 2 => {
            let (start, end) = (map(to_pdf, points[0]), map(to_pdf, points[1]));
            let head = arrow_head(start, end, width);
            let mut ap = format!("{width:.2} w 1 J 1 j {color} RG {:.2} {:.2} m {:.2} {:.2} l S", start.0, start.1, end.0, end.1);
            for side in head {
                ap += &format!(" {:.2} {:.2} m {:.2} {:.2} l S", end.0, end.1, side.0, side.1);
            }
            let mut extra = border(width);
            extra.set("L", [start.0, start.1, end.0, end.1].map(Object::from).to_vec());
            extra.set("LE", vec![Object::Name(b"None".to_vec()), Object::Name(b"OpenArrow".to_vec())]);
            annot("Line", bounds(&[start, end, head[0], head[1]]), width, ap, Dictionary::new(), extra);
        }
        "pen" | "marker" if points.len() > 1 => {
            let pressures = numbers(&drawing["pressures"]);
            let runs = if kind == "pen" && !pressures.is_empty() {
                pressure_runs(&points, &pressures, width)
            } else {
                vec![(points, if kind == "marker" { 12.0 } else { width })]
            };
            for (run, width) in runs {
                let run: Vec<_> = run.into_iter().map(|p| map(to_pdf, p)).collect();
                let cap = if kind == "marker" { 0 } else { 1 }; // a highlighter has a flat tip
                let mut ap = format!("{width:.2} w {cap} J 1 j {color} RG");
                let (resources, mut extra) = (see_through(kind == "marker"), border(width));
                if kind == "marker" {
                    ap = format!("/A gs {ap}");
                    extra.set("CA", 0.4);
                }
                for (i, (x, y)) in run.iter().enumerate() {
                    ap += &format!(" {x:.2} {y:.2} {}", if i == 0 { "m" } else { "l" });
                }
                ap += " S";
                let ink: Vec<Object> = run.iter().flat_map(|&(x, y)| [x, y]).map(Object::from).collect();
                extra.set("InkList", vec![Object::Array(ink)]);
                annot("Ink", bounds(&run), width / 2.0, ap, resources, extra);
            }
        }
        "highlight" => {
            let boxes: Vec<Vec<f64>> = drawing["boxes"].as_array().map(|b| b.iter().map(numbers).collect()).unwrap_or_default();
            let quads: Vec<[(f64, f64); 4]> = boxes.iter().filter(|b| b.len() == 4).map(|b| corners(b)).collect();
            if quads.is_empty() {
                return results;
            }
            let mut ap = format!("/A gs {color} rg");
            for [a, b, c, d] in &quads {
                // corners in the order QuadPoints wants: top-left, top-right, bottom-left, bottom-right
                ap += &format!(" {:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l {:.2} {:.2} l h f", a.0, a.1, b.0, b.1, d.0, d.1, c.0, c.1);
            }
            let all: Vec<(f64, f64)> = quads.iter().flatten().copied().collect();
            let mut extra = dictionary! { "CA" => 0.4 };
            extra.set("QuadPoints", all.iter().flat_map(|&(x, y)| [x, y]).map(Object::from).collect::<Vec<_>>());
            annot("Highlight", bounds(&all), 0.0, ap, see_through(true), extra);
        }
        "text" if rect.len() == 4 => {
            let text = drawing["text"].as_str().unwrap_or("");
            let size = drawing["size"].as_f64().unwrap_or(14.0);
            // Drawn in shown-page coordinates, y up: rotates with the page, so it reads upright like on screen
            let m = to_pdf;
            let mut ap = format!(
                "q {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} cm BT /Helv {size:.2} Tf {color} rg",
                m[0], m[1], -m[2], -m[3], m[4], m[5]
            );
            for (i, line) in text.lines().enumerate() {
                // the same margin and line spacing as the text box on screen (PdfEditor.qml)
                let baseline = rect[1] + 2.0 + size * 0.93 + i as f64 * size * 1.16;
                ap += &format!(" 1 0 0 1 {:.2} {:.2} Tm ({}) Tj", rect[0] + 2.0, -baseline, win_ansi(line));
            }
            ap += " ET Q";
            let font = dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" };
            let mut extra = border(0.0);
            extra.set("DA", Object::string_literal(format!("/Helv {size:.2} Tf {color} rg")));
            extra.set("Contents", text_string(text));
            extra.set("Rotate", rotation(to_pdf));
            let resources = dictionary! { "Font" => dictionary! { "Helv" => font } };
            annot("FreeText", bounds(&corners(&rect)), 0.0, ap, resources, extra);
        }
        _ => {}
    }
    results
}

/// The page rotation a matrix from `page_matrix` undoes, for free text that stays upright in other editors.
fn rotation(m: &Matrix) -> i64 {
    match (m[0], m[1]) {
        (_, 1.0) => 90,
        (-1.0, _) => 180,
        (_, -1.0) => 270,
        _ => 0,
    }
}

/// Resources with `/A`: 40 % see-through, multiplied with the page so text under it stays black.
fn see_through(on: bool) -> Dictionary {
    if !on {
        return Dictionary::new();
    }
    let state = dictionary! { "Type" => "ExtGState", "CA" => 0.4, "ca" => 0.4, "BM" => "Multiply" };
    dictionary! { "ExtGState" => dictionary! { "A" => state } }
}

fn bounds(points: &[(f64, f64)]) -> [f64; 4] {
    let fold = |pick: fn(&(f64, f64)) -> f64, f: fn(f64, f64) -> f64, start| points.iter().map(pick).fold(start, f);
    [
        fold(|p| p.0, f64::min, f64::MAX),
        fold(|p| p.1, f64::min, f64::MAX),
        fold(|p| p.0, f64::max, f64::MIN),
        fold(|p| p.1, f64::max, f64::MIN),
    ]
}

/// The two ends of an open arrow head at `end`, as `arrow` draws it in PdfEditor.qml.
fn arrow_head(start: (f64, f64), end: (f64, f64), width: f64) -> [(f64, f64); 2] {
    let back = (start.1 - end.1).atan2(start.0 - end.0);
    let length = width * 3.0 + 5.0;
    [28f64, -28f64].map(|turn| {
        let angle = back + turn.to_radians();
        (end.0 + length * angle.cos(), end.1 + length * angle.sin())
    })
}

/// An ellipse filling `[x0 y0 x1 y1]`, from four Bézier curves.
fn ellipse([x0, y0, x1, y1]: [f64; 4]) -> String {
    let (cx, cy, rx, ry) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let (kx, ky) = (rx * 0.5523, ry * 0.5523); // the usual circle-from-Béziers constant
    format!(
        "{:.2} {cy:.2} m {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c \
         {:.2} {:.2} {:.2} {:.2} {cx:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {cy:.2} c",
        cx + rx, cx + rx, cy + ky, cx + kx, cy + ry, cy + ry,
        cx - kx, cy + ry, cx - rx, cy + ky, cx - rx,
        cx - rx, cy - ky, cx - kx, cy - ry, cy - ry,
        cx + kx, cy - ry, cx + rx, cy - ky, cx + rx,
    )
}

/// `text` for a PDF string drawn in Helvetica: Latin-1 letters, `?` for the rest.
// ponytail: built-in Helvetica only knows Western letters; embed a font if Greek/CJK text matters
fn win_ansi(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '(' | ')' | '\\' => format!("\\{c}"),
            ' '..='~' => c.to_string(),
            '\u{a0}'..='\u{ff}' => format!("\\{:03o}", c as u32),
            _ => "?".into(),
        })
        .collect()
}

/// A PDF text string in UTF-16, which every reader understands for any letter.
fn text_string(text: &str) -> Object {
    let bytes = [0xfe, 0xff].into_iter().chain(text.encode_utf16().flat_map(u16::to_be_bytes)).collect();
    Object::String(bytes, StringFormat::Hexadecimal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A two-page PDF with a line of text; page 2 is turned 90°.
    fn sample() -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let content = doc.add_object(Stream::new(Dictionary::new(), b"BT /F1 12 Tf 40 700 Td (Hello world) Tj ET".to_vec()));
        let kids: Vec<Object> = [0, 90]
            .map(|rotate| {
                doc.add_object(dictionary! {
                    "Type" => "Page", "Parent" => pages_id, "Contents" => content, "Rotate" => rotate,
                    "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
                })
                .into()
            })
            .to_vec();
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 2, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()] }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn subtypes(pdf: &[u8], page: u32) -> Vec<String> {
        let doc = Document::load_mem(pdf).unwrap();
        let id = doc.get_pages()[&page];
        doc.get_page_annotations(id)
            .unwrap()
            .iter()
            .map(|a| String::from_utf8_lossy(a.get(b"Subtype").unwrap().as_name().unwrap()).into_owned())
            .collect()
    }

    #[test]
    fn writes_every_kind_as_an_annotation() {
        let drawings = json!([
            { "page": 0, "kind": "rect", "color": [1, 0, 0], "width": 3, "rect": [10, 10, 100, 60] },
            { "page": 0, "kind": "ellipse", "color": [0, 0, 1], "width": 3, "rect": [10, 80, 100, 160] },
            { "page": 0, "kind": "arrow", "color": [0, 1, 0], "width": 3, "points": [[200, 200], [300, 260]] },
            { "page": 0, "kind": "pen", "color": [0, 0, 0], "width": 3, "points": [[50, 300], [80, 320], [120, 300]] },
            { "page": 0, "kind": "marker", "color": [1, 1, 0], "width": 3, "points": [[50, 400], [200, 400]] },
            { "page": 0, "kind": "highlight", "color": [1, 1, 0], "boxes": [[40, 80, 110, 94]] },
            { "page": 1, "kind": "text", "color": [0, 0, 0], "width": 0, "rect": [100, 100, 220, 124], "text": "Ciao (è)", "size": 14 },
        ]);
        let pdf = annotated(sample(), drawings.as_array().unwrap()).unwrap();
        assert_eq!(subtypes(&pdf, 1), ["Square", "Circle", "Line", "Ink", "Ink", "Highlight"]);
        assert_eq!(subtypes(&pdf, 2), ["FreeText"]);
        assert!(pdf.starts_with(&sample()), "an incremental update keeps the original bytes");

        let doc = Document::load_mem(&pdf).unwrap();
        let text = doc.get_page_annotations(doc.get_pages()[&2]).unwrap()[0].clone();
        let contents = text.get(b"Contents").unwrap().as_str().unwrap();
        assert_eq!(&contents[2..], "Ciao (è)".encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<_>>());
    }

    #[test]
    fn annotates_an_annotated_file() {
        let drawing = json!([{ "page": 0, "kind": "rect", "color": [1, 0, 0], "width": 3, "rect": [10, 10, 100, 60] }]);
        let once = annotated(sample(), drawing.as_array().unwrap()).unwrap();
        let twice = annotated(once, drawing.as_array().unwrap()).unwrap();
        assert_eq!(subtypes(&twice, 1).len(), 2, "the second save keeps the first one's drawing");
    }

    #[test]
    fn maps_shown_points_to_the_page() {
        let doc = Document::load_mem(&sample()).unwrap();
        let pages = doc.get_pages();
        // upright page: y flips from the top
        assert_eq!(map(&page_matrix(&doc, pages[&1]), (10.0, 20.0)), (10.0, 772.0));
        // turned 90° clockwise: the shown top-left corner is the page's bottom-left
        assert_eq!(map(&page_matrix(&doc, pages[&2]), (0.0, 0.0)), (0.0, 0.0));
        assert_eq!(map(&page_matrix(&doc, pages[&2]), (792.0, 0.0)), (0.0, 792.0));
    }

    #[test]
    fn pressure_splits_into_runs() {
        let points = [(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0)];
        let runs = pressure_runs(&points, &[0.5, 0.5, 1.0, 1.0], 2.0);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].0, [(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)]); // the next run starts where this one ends
        assert_eq!(runs[1], (vec![(2.0, 0.0), (3.0, 0.0)], 4.0));
    }
}
