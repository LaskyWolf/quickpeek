// SPDX-License-Identifier: GPL-3.0-or-later
// Qt features cxx-qt-lib doesn't wrap. Declared for Rust in rust/helpers.rs.
#pragma once
#include "rust/cxx.h"
#include <cstdint>

struct MimeInfo;
struct DolphinSort;

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
