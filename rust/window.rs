// SPDX-License-Identifier: GPL-3.0-or-later
//! KDEQuickPeek window. It stays running hidden after you close it, waiting for the next file
//! from the `quickpeek` launcher. A separate program, so the launcher never loads Qt.

mod app;
mod browse;
mod embedded;
mod convert;
mod desktop;
mod details;
mod files;
mod helpers;
mod media;
mod office;
mod model;
mod pdfedit;
mod sheet;
mod socket;
mod text;

use cxx_qt_lib::{QQmlApplicationEngine, QUrl};

fn main() {
    helpers::create_app(std::env::args().collect());
    let mut engine = QQmlApplicationEngine::new();
    if let Some(mut engine) = engine.as_mut() {
        helpers::block_network(engine.as_mut());
        helpers::add_pdf_pages(engine.as_mut());
        engine.load(&QUrl::from("qrc:/qt/qml/com/quickpeek/qml/Main.qml"));
    }
    // Exit without tearing anything down: worker threads (ffmpeg's decoders too) may still be running,
    // and exit() would free the C library's state under them. The window only quits on `quickpeek --quit`.
    let code = helpers::exec_app();
    unsafe extern "C" {
        fn _exit(code: i32) -> !;
    }
    unsafe { _exit(code) }
}
