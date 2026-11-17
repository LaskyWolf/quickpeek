// SPDX-License-Identifier: GPL-3.0-or-later
//! KDEQuickPeek window. It stays running hidden after you close it, waiting for the next file
//! from the `quickpeek` launcher. A separate program, so the launcher never loads Qt.

mod app;
mod browse;
mod files;
mod helpers;
mod media;
mod model;
mod sheet;
mod socket;
mod text;

use cxx_qt_lib::{QQmlApplicationEngine, QUrl};

fn main() {
    helpers::create_app(std::env::args().collect());
    let mut engine = QQmlApplicationEngine::new();
    if let Some(mut engine) = engine.as_mut() {
        helpers::block_network(engine.as_mut());
        engine.load(&QUrl::from("qrc:/qt/qml/com/quickpeek/qml/Main.qml"));
    }
    // Exit without tearing the QML scene down: worker threads may still be finishing,
    // and the window only quits on `quickpeek --quit`, when nothing is left to save
    std::process::exit(helpers::exec_app());
}
