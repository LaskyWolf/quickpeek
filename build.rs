use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("com.quickpeek").qml_files(["qml/Main.qml", "qml/ImageView.qml", "qml/FontView.qml", "qml/MediaView.qml", "qml/SheetView.qml", "qml/TextView.qml", "qml/PdfView.qml", "qml/FolderView.qml", "qml/ArchiveView.qml", "qml/OfficeView.qml", "qml/SlideView.qml", "qml/ModelView.qml", "qml/SidePanel.qml", "qml/FileMenu.qml", "qml/PdfEditor.qml"]))
        .qt_module("Widgets")
        .qt_module("DBus")
        .qt_module("Network")
        .qt_module("QuickControls2")
        .qt_module("Quick")
        .qt_module("Pdf")
        .files(["rust/app.rs", "rust/helpers.rs", "rust/media.rs", "rust/sheet.rs", "rust/text.rs", "rust/browse.rs", "rust/office.rs", "rust/model.rs", "rust/details.rs", "rust/desktop.rs", "rust/pdfedit.rs", "rust/ai.rs"])
        .cpp_file("cpp/helpers.cpp")
        .build();
}
