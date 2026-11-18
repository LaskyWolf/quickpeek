// SPDX-License-Identifier: GPL-3.0-or-later
//! The grid of qml/SheetView.qml: xlsx, xlsm, xlsb, xls and ods through calamine, csv and tsv through csv.
//! A sheet is read on a worker thread, so a big file never freezes the window.

use crate::helpers;
use calamine::{Data, Reader, Style};
use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QStringList, QVariant};
use std::io::{BufRead, Read};
use std::path::Path;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!(<QtCore/QAbstractTableModel>);
        type QAbstractTableModel;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qt.h");
        #[namespace = "Qt"]
        type Orientation = cxx_qt_lib::Orientation;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[base = QAbstractTableModel]
        #[qproperty(QString, path)]
        #[qproperty(QStringList, sheets)]
        #[qproperty(i32, sheet)]
        #[qproperty(bool, loading)]
        #[qproperty(QString, error)]
        #[qproperty(QString, note)]
        type SheetModel = super::SheetModelRust;

        /// Shows another sheet of the workbook.
        #[qinvokable]
        fn select(self: Pin<&mut SheetModel>, sheet: i32);

        /// Width of the column's longest text, in characters.
        #[qinvokable]
        #[cxx_name = "columnChars"]
        fn column_chars(self: &SheetModel, column: i32) -> i32;

        /// "B7"
        #[qinvokable]
        #[cxx_name = "cellName"]
        fn cell_name(self: &SheetModel, row: i32, column: i32) -> QString;

        #[cxx_override]
        fn data(self: &SheetModel, index: &QModelIndex, role: i32) -> QVariant;

        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &SheetModel, parent: &QModelIndex) -> i32;

        #[cxx_override]
        #[cxx_name = "columnCount"]
        fn column_count(self: &SheetModel, parent: &QModelIndex) -> i32;

        #[cxx_override]
        #[cxx_name = "headerData"]
        fn header_data(self: &SheetModel, section: i32, orientation: Orientation, role: i32) -> QVariant;

        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &SheetModel) -> QHash_i32_QByteArray;

        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut SheetModel>);

        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut SheetModel>);
    }

    impl cxx_qt::Threading for SheetModel {}
    impl cxx_qt::Initialize for SheetModel {}
}

use qobject::SheetModel;

// ponytail: hard cap; a lazy model that reads rows on demand if it bites
const MAX_ROWS: usize = 5000;
const MAX_COLS: usize = 200;
// Qt.UserRole (0x100) and up, in this order
const ROLES: [&str; 7] = ["formula", "bold", "italic", "underline", "foreground", "background", "alignment"];

#[derive(Default)]
struct CellStyle {
    bold: bool,
    italic: bool,
    underline: bool,
    foreground: String, // "#rrggbb", or empty for the palette's colour
    background: String,
    alignment: i32, // Qt.AlignLeft/HCenter/Right, 0 = by value type
}

#[derive(Default)]
struct Cell {
    text: String,
    formula: String, // "=SUM(A1:A3)", or the unformatted value, for the bar above the grid
    number: bool,
    style: Option<Box<CellStyle>>,
}

#[derive(Default)]
struct Table {
    names: Vec<String>,
    rows: Vec<Vec<Cell>>,
    columns: usize,
    widths: Vec<usize>,
    note: String,
}

#[derive(Default)]
pub struct SheetModelRust {
    path: QString,
    sheets: QStringList,
    sheet: i32,
    loading: bool,
    error: QString,
    note: QString,
    table: Table,
    generation: u64, // a slow read finishing after the user moved on is dropped
}

impl cxx_qt::Initialize for SheetModel {
    fn initialize(self: Pin<&mut Self>) {
        self.on_path_changed(|model| model.load(0)).release();
    }
}

impl qobject::SheetModel {
    fn select(self: Pin<&mut Self>, sheet: i32) {
        if sheet != self.rust().sheet {
            self.load(sheet);
        }
    }

    fn load(mut self: Pin<&mut Self>, sheet: i32) {
        let path = self.rust().path.to_string();
        let generation = self.rust().generation + 1;
        self.as_mut().rust_mut().generation = generation;
        self.as_mut().set_sheet(sheet);
        self.as_mut().set_loading(true);
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let result = read(Path::new(&path), sheet.max(0) as usize);
            let _ = thread.queue(move |mut model| {
                if model.rust().generation != generation {
                    return;
                }
                let (table, error) = match result {
                    Ok(table) => (table, String::new()),
                    Err(error) => (Table::default(), format!("Couldn't read this spreadsheet: {error}")),
                };
                let mut names = QStringList::default();
                for name in &table.names {
                    names.append(QString::from(name.as_str()));
                }
                let note = QString::from(table.note.as_str());
                model.as_mut().begin_reset_model();
                model.as_mut().rust_mut().table = table;
                model.as_mut().end_reset_model();
                model.as_mut().set_sheets(names);
                model.as_mut().set_note(note);
                model.as_mut().set_error(QString::from(error.as_str()));
                model.as_mut().set_loading(false);
            });
        });
    }

    fn cell(&self, row: i32, column: i32) -> Option<&Cell> {
        self.rust().table.rows.get(row as usize)?.get(column as usize)
    }

    fn column_chars(&self, column: i32) -> i32 {
        self.rust().table.widths.get(column as usize).copied().unwrap_or(0) as i32
    }

    fn cell_name(&self, row: i32, column: i32) -> QString {
        QString::from(format!("{}{}", column_name(column as usize), row + 1).as_str())
    }

    fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(cell) = self.cell(index.row(), index.column()) else { return QVariant::default() };
        let style = cell.style.as_deref();
        let text = |s: &str| QVariant::from(&QString::from(s));
        match role {
            0 => text(&cell.text),
            0x100 => text(if cell.formula.is_empty() { &cell.text } else { &cell.formula }),
            0x101 => QVariant::from(&style.is_some_and(|s| s.bold)),
            0x102 => QVariant::from(&style.is_some_and(|s| s.italic)),
            0x103 => QVariant::from(&style.is_some_and(|s| s.underline)),
            0x104 => text(style.map_or("", |s| &s.foreground)),
            0x105 => text(style.map_or("", |s| &s.background)),
            0x106 => {
                let set = style.map_or(0, |s| s.alignment);
                QVariant::from(&if set != 0 { set } else if cell.number { 2 } else { 1 }) // Excel: numbers right
            }
            _ => QVariant::default(),
        }
    }

    fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rust().table.rows.len() as i32
    }

    fn column_count(&self, _parent: &QModelIndex) -> i32 {
        self.rust().table.columns as i32
    }

    fn header_data(&self, section: i32, orientation: qobject::Orientation, role: i32) -> QVariant {
        if role != 0 {
            return QVariant::default();
        }
        let label = if orientation == qobject::Orientation::Horizontal {
            column_name(section as usize)
        } else {
            (section + 1).to_string()
        };
        QVariant::from(&QString::from(label.as_str()))
    }

    fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(0, QByteArray::from("display"));
        for (i, name) in ROLES.iter().enumerate() {
            roles.insert(0x100 + i as i32, QByteArray::from(*name));
        }
        roles
    }
}

/// 0 -> "A", 25 -> "Z", 26 -> "AA"
fn column_name(mut column: usize) -> String {
    let mut name = Vec::new();
    loop {
        name.push(b'A' + (column % 26) as u8);
        if column < 26 {
            break;
        }
        column = column / 26 - 1;
    }
    name.reverse();
    String::from_utf8(name).unwrap()
}

fn read(path: &Path, sheet: usize) -> Result<Table, String> {
    let ext = path.extension().map_or(String::new(), |e| e.to_string_lossy().to_lowercase());
    let mut table = if ext == "csv" || ext == "tsv" { read_csv(path, &ext)? } else { read_workbook(path, sheet)? };
    table.columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
    table.widths = vec![0; table.columns];
    for row in &table.rows {
        for (width, cell) in table.widths.iter_mut().zip(row) {
            *width = (*width).max(cell.text.chars().count());
        }
    }
    Ok(table)
}

/// The separator a CSV uses: tab for .tsv, else whichever of , ; tab | the first line has most of.
fn delimiter(first_line: &str, ext: &str) -> u8 {
    if ext == "tsv" {
        return b'\t';
    }
    // `max_by_key` keeps the last of equals, so the list ends with the default
    [b'|', b'\t', b';', b',']
        .into_iter()
        .max_by_key(|&d| first_line.bytes().filter(|&b| b == d).count())
        .unwrap()
}

fn read_csv(path: &Path, ext: &str) -> Result<Table, String> {
    let mut first_line = Vec::new();
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    std::io::BufReader::new(file.take(64 * 1024)).read_until(b'\n', &mut first_line).map_err(|e| e.to_string())?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter(&String::from_utf8_lossy(&first_line), ext))
        .from_path(path)
        .map_err(|e| e.to_string())?;
    let mut table = Table::default();
    for record in reader.byte_records() {
        let record = record.map_err(|e| e.to_string())?;
        if table.rows.len() == MAX_ROWS {
            table.note = format!("first {MAX_ROWS} rows");
            break;
        }
        let row = record.iter().take(MAX_COLS).map(|field| {
            let text = String::from_utf8_lossy(field).into_owned();
            Cell { number: text.trim().parse::<f64>().is_ok(), text, ..Default::default() }
        });
        table.rows.push(row.collect());
    }
    Ok(table)
}

fn read_workbook(path: &Path, sheet: usize) -> Result<Table, String> {
    let mut book = calamine::open_workbook_auto(path).map_err(|e| e.to_string())?;
    let names = book.sheet_names();
    let name = names.get(sheet).ok_or("no sheets")?.clone();
    let values = book.worksheet_range(&name).map_err(|e| e.to_string())?;
    let formulas = book.worksheet_formula(&name).unwrap_or_default();
    let styles = book.worksheet_style(&name).unwrap_or_default(); // xlsx only; other formats come back empty
    let (point, group) = (helpers::locale_decimal_point(), helpers::locale_group_separator());

    // Like Excel, the grid starts at A1 even when the data starts further in
    let (end_row, end_col) = values.end().unwrap_or_default();
    let (rows, cols) = (end_row as usize + 1, end_col as usize + 1);
    let mut table = Table { names, ..Default::default() };
    let capped = [(rows, MAX_ROWS, "rows"), (cols, MAX_COLS, "columns")];
    let capped: Vec<_> = capped.iter().filter(|c| c.0 > c.1).map(|(n, max, what)| format!("first {max} of {n} {what}")).collect();
    table.note = capped.join(", ");
    if values.is_empty() {
        return Ok(table);
    }
    for r in 0..rows.min(MAX_ROWS) as u32 {
        let row = (0..cols.min(MAX_COLS) as u32).map(|c| {
            let style = styles.get((r, c));
            let format = style.and_then(|s| s.number_format.as_ref()).map_or("", |f| f.format_code.as_str());
            let value = values.get_value((r, c)).unwrap_or(&Data::Empty);
            let text = format_cell(value, format, &point, &group);
            let formula = match formulas.get_value((r, c)) {
                Some(f) if !f.is_empty() => format!("={f}"),
                _ => value.to_string(),
            };
            Cell {
                formula: if formula == text { String::new() } else { formula },
                text,
                number: matches!(value, Data::Int(_) | Data::Float(_)),
                style: style.and_then(cell_style),
            }
        });
        table.rows.push(row.collect());
    }
    Ok(table)
}

fn cell_style(style: &Style) -> Option<Box<CellStyle>> {
    let font = style.font.as_ref();
    // Excel's default black text and white fill come from the theme: skip them so dark mode stays readable
    let background = style.fill.as_ref().filter(|f| f.is_visible()).and_then(|f| f.get_color()).filter(|c| !c.is_white());
    let foreground = font.and_then(|f| f.color).filter(|c| !c.is_black() || background.is_some());
    use calamine::HorizontalAlignment as H;
    let alignment = match style.alignment.as_ref().map(|a| a.horizontal) {
        Some(H::Left) => 1,
        Some(H::Right) => 2,
        Some(H::Center | H::CenterContinuous) => 4,
        _ => 0,
    };
    let cell = CellStyle {
        bold: font.is_some_and(|f| f.is_bold()),
        italic: font.is_some_and(|f| f.is_italic()),
        underline: font.is_some_and(|f| f.has_underline()),
        foreground: foreground.map_or(String::new(), |c| c.to_string()),
        background: background.map_or(String::new(), |c| c.to_string()),
        alignment,
    };
    let plain = !cell.bold && !cell.italic && !cell.underline && alignment == 0;
    (!plain || !cell.foreground.is_empty() || !cell.background.is_empty()).then(|| Box::new(cell))
}

/// A cell's value roughly like Excel shows it, in the system locale.
fn format_cell(value: &Data, format: &str, point: &str, group: &str) -> String {
    match value {
        Data::Empty => String::new(),
        Data::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).into(),
        Data::Int(i) => format_number(*i as f64, format, point, group).unwrap_or_else(|| helpers::locale_number(*i as f64)),
        Data::Float(f) => format_number(*f, format, point, group).unwrap_or_else(|| helpers::locale_number(*f)),
        Data::DateTime(d) if d.is_duration() => {
            let seconds = (d.as_f64() * 86400.0).round() as i64;
            format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
        }
        Data::DateTime(d) => match d.as_datetime().map(|t| t.to_string()) {
            // "2024-01-31 00:00:00" is a plain date
            Some(t) => helpers::locale_datetime(&t.strip_suffix(" 00:00:00").map_or(t.replace(' ', "T"), str::to_string)),
            None => d.as_f64().to_string(),
        },
        Data::DateTimeIso(s) => helpers::locale_datetime(s),
        Data::String(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(e) => e.to_string(),
    }
}

/// `value` with an Excel number format such as `#,##0.00 [$€-410]` or `0%`. None for "General" or
/// a format without digits, which the caller shows in the locale's own way.
fn format_number(value: f64, format: &str, point: &str, group: &str) -> Option<String> {
    if format.is_empty() || format.eq_ignore_ascii_case("general") {
        return None;
    }
    // ponytail: covers decimals, thousands, %, currency and literal text; not conditional or fraction formats
    let negative_section = value < 0.0 && format.contains(';');
    let raw = format.split(';').nth(negative_section as usize).unwrap_or_default();
    let mut section = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '[' => {
                let inner: String = chars.by_ref().take_while(|&c| c != ']').collect();
                if let Some(currency) = inner.strip_prefix('$') {
                    section.push_str(currency.split('-').next().unwrap_or_default()); // [$€-410] -> €
                } // other brackets are colours and conditions
            }
            '_' | '*' => {
                chars.next(); // padding and fill
            }
            '\\' | '"' => {}
            c => section.push(c),
        }
    }
    let start = section.find(['#', '0', '?'])?;
    let len = section[start..].find(|c| !"#0?,.".contains(c)).unwrap_or(section.len() - start);
    let digits = &section[start..start + len];
    let decimals = digits.split_once('.').map_or(0, |(_, d)| d.len());
    let number = if section.contains('%') { value.abs() * 100.0 } else { value.abs() };
    let fixed = format!("{number:.decimals$}");
    let (int, frac) = fixed.split_once('.').unwrap_or((&fixed, ""));
    let mut text = String::new();
    for (i, digit) in int.chars().enumerate() {
        // Grouped by hand: Qt skips the separator on 4-digit numbers in some locales, Excel doesn't
        if i > 0 && (int.len() - i) % 3 == 0 && digits.contains(',') {
            text.push_str(group);
        }
        text.push(digit);
    }
    if !frac.is_empty() {
        text = format!("{text}{point}{frac}");
    }
    let sign = if value < 0.0 && !format.contains(';') { "-" } else { "" };
    Some(format!("{sign}{}{text}{}", &section[..start], &section[start + len..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_names() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(column_name(702), "AAA");
    }

    #[test]
    fn number_formats() {
        let f = |v, code| format_number(v, code, ",", ".");
        assert_eq!(f(2490.0, "#,##0.00"), Some("2.490,00".into()));
        assert_eq!(f(1234567.891, "#,##0"), Some("1.234.568".into()));
        assert_eq!(f(0.256, "0.0%"), Some("25,6%".into()));
        assert_eq!(f(12.5, "#,##0.00 [$€-410]"), Some("12,50 €".into()));
        assert_eq!(f(-3.0, "0.00"), Some("-3,00".into()));
        assert_eq!(f(-3.0, "\"$\"#,##0_);[Red](\"$\"#,##0)"), Some("($3)".into()));
        assert_eq!(f(7.0, "0\" pcs\""), Some("7 pcs".into()));
        assert_eq!(f(7.0, "General"), None);
        assert_eq!(f(7.0, "@"), None);
    }

    #[test]
    fn csv_delimiters() {
        assert_eq!(delimiter("a,b,c", "csv"), b',');
        assert_eq!(delimiter("nome;prezzo;note, varie", "csv"), b';');
        assert_eq!(delimiter("a\tb\tc", "csv"), b'\t');
        assert_eq!(delimiter("single", "csv"), b',');
        assert_eq!(delimiter("a,b", "tsv"), b'\t');
    }
}
