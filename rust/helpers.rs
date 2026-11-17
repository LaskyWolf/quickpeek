// SPDX-License-Identifier: GPL-3.0-or-later
//! Qt features cxx-qt-lib doesn't wrap, implemented in cpp/helpers.cpp.

#[cxx::bridge]
pub mod ffi {
    struct MimeInfo {
        name: String,
        comment: String,
        icon: String,
        generic_icon: String,
    }

    /// Sorting of a Dolphin window. `found` is false when no Dolphin window shows the folder.
    struct DolphinSort {
        found: bool,
        role: String,
        descending: bool,
        folders_first: bool,
        hidden_last: bool,
        show_hidden: bool,
    }

    unsafe extern "C++" {
        include!("quickpeek/cpp/helpers.h");
        fn create_app(args: Vec<String>);
        fn exec_app() -> i32;
        fn mime_info(path: &str) -> MimeInfo;
        fn mime_inherits(path: &str, parent: &str) -> bool;
        fn can_read_image(path: &str) -> bool;
        fn collate(a: &str, b: &str) -> i32;
        fn read_xattr(path: &str, name: &str) -> String;
        fn dolphin_live_sort(folder: &str) -> DolphinSort;
        fn open_url(path: &str);
        fn formatted_size(bytes: u64) -> String;
        fn copy_text(text: &str);
        fn copy_file(path: &str, with_image: bool);
        fn move_to_trash(path: &str) -> bool;

        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;
        fn block_network(engine: Pin<&mut QQmlApplicationEngine>);
    }
}

pub use ffi::*;
