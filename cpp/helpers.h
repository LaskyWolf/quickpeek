// SPDX-License-Identifier: GPL-3.0-or-later
// Qt features cxx-qt-lib doesn't wrap. Declared for Rust in rust/helpers.rs.
#pragma once
#include "rust/cxx.h"
#include <cstdint>

struct MimeInfo;
struct DolphinSort;
struct Pixels;

void create_app(rust::Vec<rust::String> args);
int exec_app();
MimeInfo mime_info(rust::Str path);
bool mime_inherits(rust::Str path, rust::Str parent);
bool can_read_image(rust::Str path);
int collate(rust::Str a, rust::Str b);
rust::String read_xattr(rust::Str path, rust::Str name);
DolphinSort dolphin_live_sort(rust::Str folder);
void open_url(rust::Str path);
rust::String formatted_size(std::uint64_t bytes);
void copy_text(rust::Str text);
void copy_file(rust::Str path, bool with_image);
bool move_to_trash(rust::Str path);
rust::String locale_number(double value);
rust::String locale_decimal_point();
rust::String locale_group_separator();
rust::String locale_datetime(rust::Str iso);
class QQmlApplicationEngine;
void block_network(QQmlApplicationEngine &engine);
rust::String thumbnail_key(rust::Str path);
rust::String mime_icon_for_name(rust::Str name);
rust::String local_time(std::int64_t secs);
rust::String md5_hex(rust::Str text);
Pixels render_pdf_page(rust::Str pdf, std::int32_t page, std::int32_t width);
bool save_png(rust::Str path, std::int32_t width, std::int32_t height, rust::Slice<const std::uint32_t> pixels);
rust::String qt_binaries_dir();
rust::Vec<std::uint8_t> rgba_bottom_up_png(rust::Slice<const std::uint8_t> pixels, int width, int height);
struct ImageHeader;
ImageHeader image_header(rust::Str path);
rust::Vec<rust::String> mime_ancestors(rust::Str path);
rust::String locale_integer(std::int64_t value);
