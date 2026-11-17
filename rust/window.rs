// SPDX-License-Identifier: GPL-3.0-or-later
//! KDEQuickPeek window. It stays running hidden after you close it, waiting for the next file
//! from the `quickpeek` launcher. A separate program, so the launcher never loads Qt.

mod app;
mod files;
mod helpers;
mod sheet;
mod socket;

use cxx_qt_lib::{QQmlApplicationEngine, QUrl};
use std::process::ExitCode;

fn main() -> ExitCode {
    helpers::create_app(std::env::args().collect());
    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/com/quickpeek/qml/Main.qml"));
    }
    ExitCode::from(helpers::exec_app() as u8)
}
