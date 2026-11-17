use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("com.quickpeek").qml_files(["qml/Main.qml", "qml/ImageView.qml", "qml/FontView.qml", "qml/MediaView.qml", "qml/SheetView.qml"]))
        .qt_module("Widgets")
        .qt_module("DBus")
        .qt_module("QuickControls2")
        .files(["rust/app.rs", "rust/helpers.rs", "rust/media.rs", "rust/sheet.rs"])
        .cpp_file("cpp/helpers.cpp")
        .build();
}
