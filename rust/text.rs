// SPDX-License-Identifier: GPL-3.0-or-later
//! Syntax highlighting for the text view, done on a worker thread so a big file never freezes the window.

use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use std::sync::LazyLock;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        /// `html` is the highlighted text, empty while it's being made or when the language is unknown.
        #[qobject]
        #[qml_element]
        #[qproperty(QString, html)]
        #[qproperty(QString, background)]
        type Highlighter = super::HighlighterRust;

        /// `name` picks the language; `dark` picks a theme readable on the window's palette.
        #[qinvokable]
        fn highlight(self: Pin<&mut Highlighter>, text: &QString, name: &QString, dark: bool);
    }

    impl cxx_qt::Threading for Highlighter {}
}

const CODE_LIMIT: usize = 200_000; // characters highlighted; bigger files show as plain text

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

#[derive(Default)]
pub struct HighlighterRust {
    html: QString,
    background: QString,
    request: u64, // only the latest request's result is shown
}

fn syntax_for<'a>(syntaxes: &'a SyntaxSet, name: &str, text: &str) -> Option<&'a SyntaxReference> {
    let extension = name.rsplit_once('.').map_or(name, |(_, ext)| ext); // whole name for Makefile, Dockerfile...
    syntaxes
        .find_syntax_by_extension(extension)
        .or_else(|| syntaxes.find_syntax_by_extension(name))
        .or_else(|| syntaxes.find_syntax_by_first_line(text.lines().next()?)) // #!/bin/sh scripts
        .filter(|syntax| syntax.name != "Plain Text")
}

fn theme(dark: bool) -> &'static Theme {
    &THEMES.themes[if dark { "base16-ocean.dark" } else { "InspiredGitHub" }]
}

/// Highlighted HTML and the theme's background color, or None when the text stays plain.
fn code_html(text: &str, name: &str, dark: bool) -> Option<(String, String)> {
    if text.chars().count() > CODE_LIMIT {
        return None;
    }
    let syntax = syntax_for(&SYNTAXES, name, text)?;
    let theme = theme(dark);
    let html = syntect::html::highlighted_html_for_string(text, &SYNTAXES, syntax, theme).ok()?;
    let background = theme.settings.background.map_or(String::new(), |c| format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b));
    Some((html, background))
}

impl qobject::Highlighter {
    fn highlight(mut self: Pin<&mut Self>, text: &QString, name: &QString, dark: bool) {
        let request = self.rust().request + 1;
        self.as_mut().rust_mut().request = request;
        self.as_mut().set_html(QString::default()); // the plain text shows until the colors are ready
        let (text, name, thread) = (text.to_string(), name.to_string(), self.qt_thread());
        std::thread::spawn(move || {
            let Some((html, background)) = code_html(&text, &name, dark) else { return };
            let _ = thread.queue(move |mut highlighter| {
                if highlighter.rust().request == request {
                    highlighter.as_mut().set_background(QString::from(background.as_str()));
                    highlighter.as_mut().set_html(QString::from(html.as_str()));
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_language_by_extension_name_or_shebang() {
        let name = |file: &str, text: &str| syntax_for(&SYNTAXES, file, text).map(|s| s.name.as_str());
        assert_eq!(name("main.rs", ""), Some("Rust"));
        assert_eq!(name("Makefile", ""), Some("Makefile"));
        assert_eq!(name("run", "#!/bin/bash\necho hi"), Some("Bourne Again Shell (bash)"));
        assert_eq!(name("notes.txt", "hello"), None);
    }

    #[test]
    fn big_or_unknown_files_stay_plain() {
        assert!(code_html("fn main() {}", "a.rs", true).is_some());
        assert!(code_html(&"x".repeat(CODE_LIMIT + 1), "a.rs", false).is_none());
        assert!(code_html("hello", "notes.txt", false).is_none());
    }
}
