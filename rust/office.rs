// SPDX-License-Identifier: GPL-3.0-or-later
//! Office documents and slides for qml/OfficeView.qml. The file's own thumbnail or text shows at once;
//! ONLYOFFICE's x2t turns it into a PDF in the background (rust/convert.rs), then the real pages swap in.
//! For PowerPoint Morph, a second deck renders every object of the Morph slides alone on black and on white;
//! the two renders give each object cut out with its transparency (see `sprite`), which SlideView.qml moves.

use crate::{convert, helpers};
use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QString, QUrl};
use roxmltree::{Document, Node};
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

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
        #[qproperty(QString, text)] // extracted text, shown until the pages are ready
        #[qproperty(QUrl, thumbnail)] // the first slide as the file saved it
        #[qproperty(QUrl, pdf)] // the converted pages
        #[qproperty(bool, loading)]
        #[qproperty(bool, presentation)]
        #[qproperty(QString, deck)] // JSON for SlideView.qml's Morph, see `deck_json`
        type Office = super::OfficeRust;
    }

    impl cxx_qt::Threading for Office {}
    impl cxx_qt::Initialize for Office {}
}

use qobject::Office;

const WORD: &[&str] = &["docx", "docm", "doc", "odt", "rtf", "epub", "fb2"];
const SLIDES: &[&str] = &["pptx", "ppt", "odp"];
const OTHER: &[&str] = &["djvu", "xps", "oxps"]; // x2t turns these into PDF too
const SPRITE_WIDTH: i32 = 1920; // ponytail: Morph sprites at a fixed size; render per window size if they look soft

fn extension(path: &Path) -> String {
    path.extension().map_or(String::new(), |e| e.to_string_lossy().to_lowercase())
}

/// Files this view shows.
pub fn handles(path: &Path) -> bool {
    let ext = extension(path);
    [WORD, SLIDES, OTHER].iter().any(|set| set.contains(&ext.as_str()))
}

#[derive(Default)]
pub struct OfficeRust {
    path: QString,
    text: QString,
    thumbnail: QUrl,
    pdf: QUrl,
    loading: bool,
    presentation: bool,
    deck: QString,
    generation: u64, // results arriving after the file changed are dropped
    job: Option<convert::Job>, // dropping it stops the conversion
}

impl cxx_qt::Initialize for Office {
    fn initialize(self: Pin<&mut Self>) {
        self.on_path_changed(|office| office.load()).release();
    }
}

impl qobject::Office {
    fn load(mut self: Pin<&mut Self>) {
        let path = PathBuf::from(self.rust().path.to_string());
        let generation = self.rust().generation + 1;
        let mut rust = self.as_mut().rust_mut();
        rust.generation = generation;
        rust.job = None;
        drop(rust);
        self.as_mut().set_text(QString::default());
        self.as_mut().set_thumbnail(QUrl::default());
        self.as_mut().set_pdf(QUrl::default());
        self.as_mut().set_deck(QString::default());
        self.as_mut().set_presentation(SLIDES.contains(&extension(&path).as_str()));

        let thread = self.qt_thread();
        let cheap = path.clone();
        std::thread::spawn(move || {
            let thumbnail = slides_thumbnail(&cheap);
            let text = if thumbnail.is_none() { office_text(&cheap).unwrap_or_default() } else { String::new() };
            let _ = thread.queue(move |mut office| {
                if office.rust().generation != generation {
                    return;
                }
                if let Some((bytes, mime)) = thumbnail {
                    let base64 = QByteArray::from(bytes.as_slice()).to_base64(Default::default());
                    let url = format!("data:{mime};base64,{}", String::from_utf8_lossy(base64.as_slice()));
                    office.as_mut().set_thumbnail(QUrl::from(url.as_str()));
                }
                office.as_mut().set_text(QString::from(text.as_str()));
            });
        });

        let (Some(_), Some(key)) = (convert::x2t(), convert::cache_key(&path)) else {
            self.as_mut().set_loading(false);
            return;
        };
        let out = convert::cache_dir().join(format!("{key}.pdf"));
        self.as_mut().set_loading(!out.exists());
        let thread = self.qt_thread();
        let source = path.clone();
        let job = convert::start(
            out,
            move |part, runner| convert::x2t_convert(&source, part, convert::X2T_PDF, runner),
            move |pdf| {
                let deck = if extension(&path) == "pptx" { read_slides(&path) } else { None };
                let _ = thread.queue(move |mut office| {
                    if office.rust().generation != generation {
                        return;
                    }
                    office.as_mut().set_loading(false);
                    let Some(pdf) = pdf else { return };
                    if let Some(deck) = &deck {
                        office.as_mut().set_deck(QString::from(deck_json(deck, None).as_str()));
                    }
                    office.as_mut().set_pdf(QUrl::from_local_file(&QString::from(pdf.to_string_lossy().as_ref())));
                    if let Some(deck) = deck.filter(|d| d.slides.iter().any(|s| s.morph)) {
                        office.morph(path, key, deck, generation);
                    }
                });
            },
        );
        self.as_mut().rust_mut().job = Some(job);
    }

    fn morph(mut self: Pin<&mut Self>, path: PathBuf, key: String, deck: Deck, generation: u64) {
        let thread = self.qt_thread();
        let slides = deck.slides.clone();
        let job = convert::start(
            convert::cache_dir().join(format!("{key}.morph")),
            move |part, runner| morph_sprites(&path, &slides, part, runner),
            move |dir| {
                let Some(dir) = dir else { return };
                let _ = thread.queue(move |mut office| {
                    if office.rust().generation == generation {
                        office.as_mut().set_deck(QString::from(deck_json(&deck, Some(&dir)).as_str()));
                    }
                });
            },
        );
        self.as_mut().rust_mut().job = Some(job);
    }
}

// ---------------------------------------------------------------- cheap previews

fn open_zip(path: &Path) -> Option<ZipArchive<File>> {
    ZipArchive::new(File::open(path).ok()?).ok()
}

fn read_bytes(zip: &mut ZipArchive<File>, name: &str) -> Option<Vec<u8>> {
    let mut file = zip.by_name(name).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

fn read_text(zip: &mut ZipArchive<File>, name: &str) -> Option<String> {
    String::from_utf8(read_bytes(zip, name)?).ok()
}

/// The first slide as the file saved it (OpenDocument: Thumbnails/thumbnail.png, Office: docProps/thumbnail.jpeg).
fn slides_thumbnail(path: &Path) -> Option<(Vec<u8>, &'static str)> {
    if !["pptx", "odp"].contains(&extension(path).as_str()) {
        return None;
    }
    let mut zip = open_zip(path)?;
    [("Thumbnails/thumbnail.png", "image/png"), ("docProps/thumbnail.jpeg", "image/jpeg")]
        .into_iter()
        .find_map(|(name, mime)| Some((read_bytes(&mut zip, name)?, mime)))
}

/// Plain text of Office/OpenDocument files: paragraphs on lines, table cells split by tabs.
fn office_text(path: &Path) -> Option<String> {
    let mut zip = open_zip(path)?;
    let names: Vec<String> = zip.file_names().flatten().map(|n| n.into_owned()).collect();
    let has = |name: &str| names.iter().any(|n| n == name);
    let (parts, paragraphs, cell, row): (Vec<String>, &[&str], _, _) = if has("content.xml") {
        (vec!["content.xml".into()], &["p", "h"], "table-cell", "table-row") // OpenDocument: odt, odp
    } else if has("word/document.xml") {
        (vec!["word/document.xml".into()], &["p"], "tc", "tr")
    } else if has("ppt/presentation.xml") {
        let mut slides: Vec<(u32, String)> = names
            .iter()
            .filter_map(|n| Some((n.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse().ok()?, n.clone())))
            .collect();
        slides.sort();
        (slides.into_iter().map(|(_, n)| n).collect(), &["p"], "tc", "tr")
    } else {
        return None;
    };
    let mut out = Vec::new();
    fn walk(node: Node, paragraphs: &[&str], cell: &str, row: &str, out: &mut Vec<String>) {
        let name = node.tag_name().name();
        if name == row {
            let cells: Vec<String> = node.children().filter(|c| c.tag_name().name() == cell).map(|c| all_text(c)).collect();
            out.push(cells.join("\t").trim_end_matches('\t').to_string());
        } else if paragraphs.contains(&name) {
            out.push(all_text(node));
        } else {
            for child in node.children().filter(Node::is_element) {
                walk(child, paragraphs, cell, row, out);
            }
        }
    }
    for part in parts {
        let text = read_text(&mut zip, &part)?;
        let doc = Document::parse(&text).ok()?;
        walk(doc.root_element(), paragraphs, cell, row, &mut out);
        out.push(String::new()); // blank line between slides
    }
    Some(out.join("\n"))
}

fn all_text(node: Node) -> String {
    node.descendants().filter(Node::is_text).filter_map(|t| t.text()).collect()
}

// ---------------------------------------------------------------- reading slides

const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";
const MORPH: &str = "http://schemas.microsoft.com/office/powerpoint/2015/09/main";
const SHAPES: &[&str] = &["sp", "pic", "grpSp", "graphicFrame", "cxnSp"]; // what a slide's shape tree draws

/// x, y, width, height in EMU, rotation in degrees.
type Box5 = [f64; 5];

#[derive(Clone, Debug)]
struct Object {
    name: String,
    them: Box5,
    shape: usize, // position among the slide's shapes
}

#[derive(Clone, Debug)]
struct Slide {
    part: String,
    morph: bool,
    duration: u32, // ms
    objects: Vec<Object>,
}

#[derive(Clone, Debug)]
struct Deck {
    slides: Vec<Slide>,
    size: (f64, f64), // EMU
}

fn child<'a, 'i>(node: Node<'a, 'i>, ns: &str, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|c| is(*c, ns, name))
}

fn is(node: Node, ns: &str, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name && node.tag_name().namespace() == Some(ns)
}

fn number(node: Node, name: &str) -> Option<f64> {
    node.attribute(name)?.parse().ok()
}

/// The zip name of `target` seen from `folder`: "ppt/slides" + "../media/a.png" → "ppt/media/a.png".
fn resolve(folder: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') { vec![] } else { folder.split('/').collect() };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

fn rels_name(part: &str) -> String {
    let (folder, name) = part.rsplit_once('/').unwrap_or(("", part));
    format!("{folder}/_rels/{name}.rels")
}

/// [(relationship id, type, target part)] of a part, targets resolved to zip names.
fn part_rels(zip: &mut ZipArchive<File>, part: &str) -> Vec<(String, String, String)> {
    let Some(text) = read_text(zip, &rels_name(part)) else { return vec![] };
    let Ok(doc) = Document::parse(&text) else { return vec![] };
    let folder = part.rsplit_once('/').map_or("", |(f, _)| f);
    doc.root_element()
        .children()
        .filter(Node::is_element)
        .filter_map(|rel| {
            let kind = rel.attribute("Type")?.rsplit('/').next()?.to_string();
            Some((rel.attribute("Id")?.to_string(), kind, resolve(folder, rel.attribute("Target")?)))
        })
        .collect()
}

/// Where a shape sits. Placeholders without their own position take it from the layout, then the
/// master (`inherited`: their shape trees).
fn shape_box(shape: Node, inherited: &[Node]) -> Option<Box5> {
    let xfrm = [child(shape, P, "spPr"), child(shape, P, "grpSpPr")]
        .into_iter()
        .flatten()
        .filter_map(|pr| child(pr, A, "xfrm"))
        .chain(child(shape, P, "xfrm"))
        .find(|x| child(*x, A, "off").is_some());
    let Some(xfrm) = xfrm else {
        let ph = shape.descendants().filter(|n| is(*n, P, "nvPr")).find_map(|n| child(n, P, "ph"))?;
        let same = |theirs: Node| match ph.attribute("idx") {
            Some(idx) => theirs.attribute("idx") == Some(idx),
            None => theirs.attribute("type").unwrap_or("body") == ph.attribute("type").unwrap_or("body"),
        };
        return inherited.iter().flat_map(|tree| tree.children().filter(Node::is_element)).find_map(|other| {
            let theirs = other.descendants().filter(|n| is(*n, P, "nvPr")).find_map(|n| child(n, P, "ph"))?;
            if same(theirs) { shape_box(other, &[]) } else { None }
        });
    };
    let (off, ext) = (child(xfrm, A, "off")?, child(xfrm, A, "ext")?);
    let turn = number(xfrm, "rot").unwrap_or(0.0) / 60000.0;
    Some([number(off, "x")?, number(off, "y")?, number(ext, "cx")?, number(ext, "cy")?, turn])
}

fn shapes<'a, 'i>(tree: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    tree.children().filter(|c| c.is_element() && SHAPES.contains(&c.tag_name().name())).collect()
}

/// The shown slides of a .pptx, in order, with where each object sits. None for anything that isn't a readable .pptx.
fn read_slides(path: &Path) -> Option<Deck> {
    let mut zip = open_zip(path)?;
    let rels = part_rels(&mut zip, "ppt/presentation.xml");
    let text = read_text(&mut zip, "ppt/presentation.xml")?;
    let pres = Document::parse(&text).ok()?;
    let size = child(pres.root_element(), P, "sldSz")?;
    let mut slides = Vec::new();
    for sld in child(pres.root_element(), P, "sldIdLst")?.children().filter(Node::is_element) {
        let id = sld.attribute((R, "id"))?;
        let part = rels.iter().find(|r| r.0 == id)?.2.clone();
        let text = read_text(&mut zip, &part)?;
        let doc = Document::parse(&text).ok()?;
        let root = doc.root_element();
        if root.attribute("show") == Some("0") {
            continue; // hidden slides are left out of the PDF too
        }
        let layout = part_rels(&mut zip, &part).into_iter().find(|r| r.1 == "slideLayout").map(|r| r.2);
        let master = layout.as_ref().and_then(|l| part_rels(&mut zip, l).into_iter().find(|r| r.1 == "slideMaster")).map(|r| r.2);
        let texts: Vec<String> = [layout, master].into_iter().flatten().filter_map(|p| read_text(&mut zip, &p)).collect();
        let docs: Vec<Document> = texts.iter().filter_map(|t| Document::parse(t).ok()).collect();
        let inherited: Vec<Node> = docs.iter().filter_map(|d| child(child(d.root_element(), P, "cSld")?, P, "spTree")).collect();

        let transition = root.descendants().find(|n| is(*n, P, "transition"));
        let morph = root.descendants().any(|n| is(n, MORPH, "morph"));
        let duration = transition.map_or(0, |t| {
            let speed = match t.attribute("spd") {
                Some("fast") => 500,
                Some("med") => 750,
                _ => 1000,
            };
            t.attribute((P14, "dur")).and_then(|d| d.parse().ok()).unwrap_or(if morph { 2000 } else { speed })
        });
        let tree = child(child(root, P, "cSld")?, P, "spTree")?;
        let objects = shapes(tree)
            .into_iter()
            .enumerate()
            .filter_map(|(k, shape)| {
                let them = shape_box(shape, &inherited)?;
                let name = shape.descendants().find(|n| is(*n, P, "cNvPr")).and_then(|n| n.attribute("name")).unwrap_or("");
                Some(Object { name: name.to_string(), them, shape: k })
            })
            .collect();
        slides.push(Slide { part, morph, duration, objects });
    }
    Some(Deck { slides, size: (number(size, "cx")?, number(size, "cy")?) })
}

// ---------------------------------------------------------------- the Morph deck

/// Text with the `edits` (range → replacement) applied. Inserts (empty ranges) go before removals at the same place.
fn splice(text: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|(range, _)| (range.start, range.end));
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (range, new) in edits {
        out.push_str(&text[at..range.start.max(at)]);
        out.push_str(&new);
        at = at.max(range.end);
    }
    out.push_str(&text[at..]);
    out
}

/// "p:" for the prefix the part uses for `ns`, or `fallback` declared on the spot.
fn prefix(node: Node, ns: &str, fallback: &str) -> (String, String) {
    match node.lookup_prefix(ns) {
        Some(p) => (format!("{p}:"), String::new()),
        None => (format!("{fallback}:"), format!(r#" xmlns:{fallback}="{ns}""#)), // also when it's the default namespace
    }
}

/// Where new children go: just before the element's closing tag.
fn before_end(text: &str, node: Node) -> Range<usize> {
    let at = text[..node.range().end].rfind("</").unwrap_or(node.range().end);
    at..at
}

/// A slide with only its background (`keep` None), or with shape `keep` alone on a solid `background`.
fn slide_variant(text: &str, keep: Option<usize>, background: &str) -> Option<String> {
    let doc = Document::parse(text).ok()?;
    let root = doc.root_element();
    let csld = child(root, P, "cSld")?;
    let mut edits: Vec<(Range<usize>, String)> =
        shapes(child(csld, P, "spTree")?).iter().enumerate().filter(|(k, _)| Some(*k) != keep).map(|(_, s)| (s.range(), String::new())).collect();
    if keep.is_some() {
        match root.attributes().find(|a| a.name() == "showMasterSp" && a.namespace().is_none()) {
            Some(attr) => edits.push((attr.range_value(), "0".into())),
            None => {
                let start = root.range().start;
                let at = start + text[start..].find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
                edits.push((at..at, r#" showMasterSp="0""#.into())); // no logos or footers from the master
            }
        }
        let (p, p_ns) = prefix(csld, P, "p");
        let (a, a_ns) = prefix(csld, A, "a");
        let bg = format!(
            r#"<{p}bg{p_ns}{a_ns}><{p}bgPr><{a}solidFill><{a}srgbClr val="{background}"/></{a}solidFill><{a}effectLst/></{p}bgPr></{p}bg>"#
        );
        let first = csld.children().find(Node::is_element)?;
        edits.push((first.range().start..first.range().start, bg));
        edits.extend(csld.children().filter(|c| is(*c, P, "bg")).map(|c| (c.range(), String::new())));
    }
    Some(splice(text, edits))
}

/// A page of the Morph deck: (slide, object or None for the background alone, background color).
type Page = (usize, Option<usize>, &'static str);

/// Writes a .pptx made for drawing Morph: for every slide in a Morph pair, its background alone, then each
/// object alone on black and on white. Returns the pages in the deck's order.
fn morph_deck(path: &Path, slides: &[Slide], dst: &Path) -> io::Result<Vec<Page>> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    let mut zip = open_zip(path).ok_or_else(|| bad("not a zip"))?;
    let wanted: BTreeSet<usize> = (1..slides.len()).filter(|&i| slides[i].morph).flat_map(|i| [i - 1, i]).collect();
    let mut pages = Vec::new();
    let mut parts = Vec::new();
    for &i in &wanted {
        let text = read_text(&mut zip, &slides[i].part).ok_or_else(|| bad("slide"))?;
        parts.push(slide_variant(&text, None, "").ok_or_else(|| bad("slide"))?);
        pages.push((i, None, ""));
        for (n, object) in slides[i].objects.iter().enumerate() {
            for background in ["000000", "FFFFFF"] {
                parts.push(slide_variant(&text, Some(object.shape), background).ok_or_else(|| bad("slide"))?);
                pages.push((i, Some(n), background));
            }
        }
    }

    // the deck: the original parts, with presentation.xml pointing at the new slides only
    let pres_text = read_text(&mut zip, "ppt/presentation.xml").ok_or_else(|| bad("presentation"))?;
    let pres = Document::parse(&pres_text).map_err(|_| bad("presentation"))?;
    let root = pres.root_element();
    let (p, p_ns) = prefix(root, P, "p");
    let (r, r_ns) = prefix(root, R, "r");
    let ids: String = (0..pages.len()).map(|k| format!(r#"<{p}sldId id="{}" {r}id="rIdPeek{k}"/>"#, 256 + k)).collect();
    let id_list = child(root, P, "sldIdLst").ok_or_else(|| bad("presentation"))?;
    let mut edits = vec![(id_list.range(), format!("<{p}sldIdLst{p_ns}{r_ns}>{ids}</{p}sldIdLst>"))];
    // these name the old slides
    edits.extend(root.children().filter(|c| is(*c, P, "custShowLst") || is(*c, P, "extLst")).map(|c| (c.range(), String::new())));
    let pres_out = splice(&pres_text, edits);

    let rels_text = read_text(&mut zip, "ppt/_rels/presentation.xml.rels").ok_or_else(|| bad("relationships"))?;
    let rels = Document::parse(&rels_text).map_err(|_| bad("relationships"))?;
    let mut edits: Vec<_> = rels
        .root_element()
        .children()
        .filter(|c| c.attribute("Type").is_some_and(|t| t.ends_with("/slide")))
        .map(|c| (c.range(), String::new()))
        .collect();
    let new: String = (0..pages.len())
        .map(|k| format!(r#"<Relationship Id="rIdPeek{k}" Target="slides/quickpeek{k}.xml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide"/>"#))
        .collect();
    edits.push((before_end(&rels_text, rels.root_element()), new));
    let rels_out = splice(&rels_text, edits);

    let types_text = read_text(&mut zip, "[Content_Types].xml").ok_or_else(|| bad("content types"))?;
    let types = Document::parse(&types_text).map_err(|_| bad("content types"))?;
    let new: String = (0..pages.len())
        .map(|k| format!(r#"<Override PartName="/ppt/slides/quickpeek{k}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#))
        .collect();
    let types_out = splice(&types_text, vec![(before_end(&types_text, types.root_element()), new)]);

    let mut out = zip::ZipWriter::new(File::create(dst)?);
    let options = zip::write::SimpleFileOptions::default();
    for (k, (page, xml)) in pages.iter().zip(&parts).enumerate() {
        out.start_file(format!("ppt/slides/quickpeek{k}.xml"), options)?;
        out.write_all(xml.as_bytes())?;
        if let Some(text) = read_text(&mut zip, &rels_name(&slides[page.0].part)) {
            let doc = Document::parse(&text).map_err(|_| bad("slide relationships"))?;
            let notes = doc
                .root_element()
                .children()
                .filter(|c| c.attribute("Type").is_some_and(|t| t.ends_with("/notesSlide") || t.ends_with("/comments")))
                .map(|c| (c.range(), String::new()))
                .collect();
            out.start_file(format!("ppt/slides/_rels/quickpeek{k}.xml.rels"), options)?;
            out.write_all(splice(&text, notes).as_bytes())?;
        }
    }
    for (name, text) in [("ppt/presentation.xml", &pres_out), ("ppt/_rels/presentation.xml.rels", &rels_out), ("[Content_Types].xml", &types_out)] {
        out.start_file(name, options)?;
        out.write_all(text.as_bytes())?;
    }
    for i in 0..zip.len() {
        let file = zip.by_index_raw(i)?;
        let rewritten = ["ppt/presentation.xml", "ppt/_rels/presentation.xml.rels", "[Content_Types].xml"];
        if !file.name().is_ok_and(|name| rewritten.contains(&name.as_ref())) {
            out.raw_copy_file(file)?;
        }
    }
    out.finish()?;
    Ok(pages)
}

/// One object, cut out with its exact transparency from two renders: on black and on white.
/// Where the object covers the slide both agree; where the background shows through they differ by
/// how much of it shows, so alpha = 1 − (white − black), and the black render is the premultiplied color.
fn sprite(black: &[u32], white: &[u32]) -> Vec<u32> {
    let channel = |pixel: u32, shift: u32| ((pixel >> shift) & 0xff) as i32;
    black
        .iter()
        .zip(white)
        .map(|(&b, &w)| {
            let diff = [0, 8, 16].iter().map(|&s| channel(w, s) - channel(b, s)).sum::<i32>() as f64 / 3.0;
            let alpha = (255.0 - diff).clamp(0.0, 255.0) as u32;
            let color = |s: u32| (channel(b, s) as u32).min(alpha) << s; // premultiplied: color never above alpha
            alpha << 24 | color(16) | color(8) | color(0)
        })
        .collect()
}

/// The part of `pixels` that isn't transparent: (x, y, width, height, its pixels), or None when all is.
fn crop(pixels: &[u32], width: usize) -> Option<(usize, usize, usize, usize, Vec<u32>)> {
    let height = pixels.len() / width.max(1);
    let seen = |x: usize, y: usize| pixels[y * width + x] >> 24 != 0;
    let rows: Vec<usize> = (0..height).filter(|&y| (0..width).any(|x| seen(x, y))).collect();
    let (top, bottom) = (*rows.first()?, *rows.last()?);
    let left = (0..width).find(|&x| (top..=bottom).any(|y| seen(x, y)))?;
    let right = (0..width).rev().find(|&x| (top..=bottom).any(|y| seen(x, y)))?;
    let (w, h) = (right - left + 1, bottom - top + 1);
    let cut = (top..=bottom).flat_map(|y| pixels[y * width + left..y * width + left + w].iter().copied()).collect();
    Some((left, top, w, h, cut))
}

/// Fills `dir` with the Morph pictures: bg<slide>.png, <slide>-<object>.png cut to size, and sprites.txt
/// ("width height" of the slide, then "slide object x y width height" per picture). Worker thread.
fn morph_sprites(path: &Path, slides: &[Slide], dir: &Path, runner: &convert::Runner) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let deck = dir.join("deck.pptx");
    let pdf = dir.join("deck.pdf");
    let pages = morph_deck(path, slides, &deck)?;
    convert::x2t_convert(&deck, &pdf, convert::X2T_PDF, runner)?;
    std::fs::remove_file(&deck)?;
    let pdf_name = pdf.to_string_lossy().into_owned();
    let failed = || io::Error::other("couldn't render the Morph deck");
    let mut manifest = String::new();
    for (k, page) in pages.iter().enumerate() {
        if runner.cancelled() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        match page {
            (slide, None, _) => {
                let page = helpers::render_pdf_page(&pdf_name, k as i32, SPRITE_WIDTH);
                if page.data.is_empty() {
                    return Err(failed());
                }
                if manifest.is_empty() {
                    manifest = format!("{} {}\n", page.width, page.height);
                }
                if !helpers::save_png(&dir.join(format!("bg{slide}.png")).to_string_lossy(), page.width, page.height, &page.data) {
                    return Err(failed());
                }
            }
            (slide, Some(n), "000000") => {
                let black = helpers::render_pdf_page(&pdf_name, k as i32, SPRITE_WIDTH);
                let white = helpers::render_pdf_page(&pdf_name, k as i32 + 1, SPRITE_WIDTH);
                if black.data.is_empty() || black.data.len() != white.data.len() {
                    return Err(failed());
                }
                let Some((x, y, w, h, pixels)) = crop(&sprite(&black.data, &white.data), black.width as usize) else { continue };
                if !helpers::save_png(&dir.join(format!("{slide}-{n}.png")).to_string_lossy(), w as i32, h as i32, &pixels) {
                    return Err(failed());
                }
                manifest += &format!("{slide} {n} {x} {y} {w} {h}\n");
            }
            _ => {} // the white render, read along with the black one
        }
    }
    std::fs::remove_file(&pdf)?;
    std::fs::write(dir.join("sprites.txt"), manifest)
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out + "\""
}

/// {width, height (EMU), dir (url of the Morph pictures), slides: [{morph, duration, bg,
/// objects: [{name, box: [x, y, w, h, turn], sprite, crop: [x, y, w, h] as fractions of the slide]}]}]}
fn deck_json(deck: &Deck, dir: Option<&Path>) -> String {
    let manifest = dir.and_then(|d| std::fs::read_to_string(d.join("sprites.txt")).ok()).unwrap_or_default();
    let mut lines = manifest.lines();
    let size: Vec<f64> = lines.next().unwrap_or("").split(' ').filter_map(|n| n.parse().ok()).collect();
    let sprites: Vec<Vec<f64>> = lines.map(|l| l.split(' ').filter_map(|n| n.parse().ok()).collect()).filter(|v: &Vec<f64>| v.len() == 6).collect();
    let dir_url = dir.map_or(String::new(), |d| QUrl::from_local_file(&QString::from(d.to_string_lossy().as_ref())).to_string().to_string());
    let slides: Vec<String> = deck
        .slides
        .iter()
        .enumerate()
        .map(|(i, slide)| {
            let has_bg = dir.is_some_and(|d| d.join(format!("bg{i}.png")).exists());
            let objects: Vec<String> = slide
                .objects
                .iter()
                .enumerate()
                .map(|(n, o)| {
                    let found = sprites.iter().find(|s| s[0] == i as f64 && s[1] == n as f64).filter(|_| size.len() == 2);
                    let (sprite, crop) = found.map_or((String::new(), "[]".into()), |s| {
                        (format!("{i}-{n}.png"), format!("[{},{},{},{}]", s[2] / size[0], s[3] / size[1], s[4] / size[0], s[5] / size[1]))
                    });
                    let [x, y, w, h, turn] = o.them;
                    format!(r#"{{"name":{},"box":[{x},{y},{w},{h},{turn}],"sprite":{},"crop":{crop}}}"#, json_string(&o.name), json_string(&sprite))
                })
                .collect();
            let bg = if has_bg { format!("bg{i}.png") } else { String::new() };
            format!(r#"{{"morph":{},"duration":{},"bg":{},"objects":[{}]}}"#, slide.morph, slide.duration, json_string(&bg), objects.join(","))
        })
        .collect();
    format!(r#"{{"width":{},"height":{},"dir":{},"slides":[{}]}}"#, deck.size.0, deck.size.1, json_string(&dir_url), slides.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprite_recovers_color_and_transparency() {
        // solid red: same on both; nothing: the background; red at half strength: halfway between
        let black = [0xffff0000, 0xff000000, 0xff800000];
        let white = [0xffff0000, 0xffffffff, 0xffff7f7f];
        let out = sprite(&black, &white);
        assert_eq!(out[0], 0xffff0000);
        assert_eq!(out[1] >> 24, 0);
        assert_eq!(out[2], 0x80800000); // 255 − mean(127, 127, 127) = 128
        assert!(out.iter().all(|p| (p >> 16 & 0xff) <= p >> 24)); // premultiplied
    }

    #[test]
    fn crop_keeps_only_what_shows() {
        let mut pixels = vec![0u32; 4 * 3];
        pixels[1 * 4 + 2] = 0xff00ff00;
        assert_eq!(crop(&pixels, 4), Some((2, 1, 1, 1, vec![0xff00ff00])));
        assert_eq!(crop(&[0u32; 4], 2), None);
    }

    #[test]
    fn resolves_relationship_targets() {
        assert_eq!(resolve("ppt/slides", "../slideLayouts/slideLayout1.xml"), "ppt/slideLayouts/slideLayout1.xml");
        assert_eq!(resolve("ppt", "slides/slide1.xml"), "ppt/slides/slide1.xml");
        assert_eq!(resolve("ppt/slides", "/ppt/media/a.png"), "ppt/media/a.png");
        assert_eq!(rels_name("ppt/slides/slide1.xml"), "ppt/slides/_rels/slide1.xml.rels");
    }

    const SLIDE: &str = r#"<?xml version="1.0"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:bg><p:bgRef idx="1"/></p:bg><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Box"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm rot="5400000"><a:off x="10" y="20"/><a:ext cx="30" cy="40"/></a:xfrm></p:spPr></p:sp></p:spTree></p:cSld></p:sld>"#;
    const LAYOUT: &str = r#"<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="1" y="2"/><a:ext cx="3" cy="4"/></a:xfrm></p:spPr></p:sp></p:spTree></p:cSld></p:sldLayout>"#;

    #[test]
    fn shape_boxes_come_from_the_shape_or_its_placeholder() {
        let (slide, layout) = (Document::parse(SLIDE).unwrap(), Document::parse(LAYOUT).unwrap());
        fn tree<'a, 'i>(d: &'a Document<'i>) -> Node<'a, 'i> {
            child(child(d.root_element(), P, "cSld").unwrap(), P, "spTree").unwrap()
        }
        let shapes = shapes(tree(&slide));
        assert_eq!(shape_box(shapes[1], &[]), Some([10.0, 20.0, 30.0, 40.0, 90.0]));
        assert_eq!(shape_box(shapes[0], &[]), None);
        assert_eq!(shape_box(shapes[0], &[tree(&layout)]), Some([1.0, 2.0, 3.0, 4.0, 0.0]));
    }

    #[test]
    fn variants_keep_one_shape_on_a_solid_background() {
        let background = slide_variant(SLIDE, None, "").unwrap();
        assert!(!background.contains("<p:sp>") && background.contains("<p:bgRef"));
        let one = slide_variant(SLIDE, Some(1), "000000").unwrap();
        let doc = Document::parse(&one).unwrap();
        let root = doc.root_element();
        assert_eq!(root.attribute("showMasterSp"), Some("0"));
        let csld = child(root, P, "cSld").unwrap();
        let first = csld.children().find(Node::is_element).unwrap();
        assert!(is(first, P, "bg") && one.contains(r#"<a:srgbClr val="000000"/>"#) && !one.contains("bgRef"));
        let kept = shapes(child(csld, P, "spTree").unwrap());
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].descendants().find(|n| is(*n, P, "cNvPr")).unwrap().attribute("name"), Some("Box"));
    }

    #[test]
    fn json_escapes_names() {
        assert_eq!(json_string("a\"b\\c\n"), r#""a\"b\\c\u000a""#);
    }
}
