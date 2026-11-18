// SPDX-License-Identifier: GPL-3.0-or-later
#include "quickpeek/rust/helpers.cxx.h"

#include <QApplication>
#include <QClipboard>
#include <QFile>
#include <QImage>
#include <QLocale>
#include <QMimeData>
#include <QCollator>
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QDBusVariant>
#include <QDesktopServices>
#include <QImageReader>
#include <QMimeDatabase>
#include <QQuickStyle>
#include <QRegularExpression>
#include <QUrl>
#include <string>
#include <sys/xattr.h>
#include <vector>

static QString q(rust::Str s) { return QString::fromUtf8(s.data(), qsizetype(s.size())); }
static rust::String r(const QString &s) { return rust::String(s.toStdString()); }

void create_app(rust::Vec<rust::String> args)
{
    // QApplication, not QGuiApplication: KDE's QML style draws with the desktop's widget style.
    // Qt keeps pointers to argc and argv for the whole run, so they live in statics.
    static std::vector<std::string> strings;
    static std::vector<char *> argv;
    static int argc;
    for (const auto &arg : args)
        strings.emplace_back(arg);
    for (auto &arg : strings)
        argv.push_back(arg.data());
    argv.push_back(nullptr);
    argc = int(strings.size());
    QQuickStyle::setStyle(QStringLiteral("org.kde.desktop"));
    auto app = new QApplication(argc, argv.data());
    app->setDesktopFileName(QStringLiteral("quickpeek"));
    app->setQuitOnLastWindowClosed(false); // closing hides the window; it waits for the next file
}

int exec_app() { return QApplication::exec(); }

MimeInfo mime_info(rust::Str path)
{
    const QMimeType mime = QMimeDatabase().mimeTypeForFile(q(path));
    return MimeInfo{r(mime.name()), r(mime.comment()), r(mime.iconName()), r(mime.genericIconName())};
}

bool mime_inherits(rust::Str path, rust::Str parent)
{
    return QMimeDatabase().mimeTypeForFile(q(path)).inherits(q(parent));
}

bool can_read_image(rust::Str path) { return QImageReader(q(path)).canRead(); }

int collate(rust::Str a, rust::Str b)
{
    // One per thread: QCollator::compare is not thread-safe (tests sort on several threads at once)
    thread_local const QCollator collator = [] {
        QCollator c;
        c.setNumericMode(true); // "img2" before "img10"
        c.setCaseSensitivity(Qt::CaseInsensitive);
        return c;
    }();
    return collator.compare(q(a), q(b));
}

rust::String read_xattr(rust::Str path, rust::Str name)
{
    const QByteArray p = q(path).toUtf8(), n = q(name).toUtf8();
    const ssize_t size = getxattr(p.constData(), n.constData(), nullptr, 0);
    if (size <= 0)
        return {};
    QByteArray value(size, Qt::Uninitialized);
    if (getxattr(p.constData(), n.constData(), value.data(), size) != size)
        return {};
    return r(QString::fromUtf8(value));
}

// Sorting of the Dolphin window that shows `folder`, read from its View menu over D-Bus.
DolphinSort dolphin_live_sort(rust::Str folder)
{
    DolphinSort result{false, "text", false, true, false, false};
    QDBusConnection bus = QDBusConnection::sessionBus();
    auto call = [&](const QString &service, const QString &path, const QString &iface, const QString &method,
                    const QVariantList &args) -> QVariant {
        QDBusMessage message = QDBusMessage::createMethodCall(service, path, iface, method);
        message.setArguments(args);
        const QDBusMessage reply = bus.call(message, QDBus::Block, 300); // a frozen Dolphin must not freeze us
        if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().isEmpty())
            return {};
        const QVariant value = reply.arguments().first();
        return value.canConvert<QDBusVariant>() ? value.value<QDBusVariant>().variant() : value;
    };
    const QString url = QUrl::fromLocalFile(q(folder)).toString();
    static const QRegularExpression windows(QStringLiteral("<node name=\"(Dolphin_\\d+)\""));
    for (const QString &service : bus.interface()->registeredServiceNames().value()) {
        if (!service.startsWith(QLatin1String("org.kde.dolphin-")))
            continue;
        const QString tree = call(service, QStringLiteral("/dolphin"), QStringLiteral("org.freedesktop.DBus.Introspectable"),
                                  QStringLiteral("Introspect"), {}).toString();
        for (const auto &match : windows.globalMatch(tree)) {
            const QString path = QStringLiteral("/dolphin/") + match.captured(1);
            if (!call(service, path, QStringLiteral("org.kde.dolphin.MainWindow"), QStringLiteral("isUrlOpen"), {url}).toBool())
                continue;
            auto checked = [&](const QString &action) { // the menu follows the window's active view
                return call(service, path + QStringLiteral("/actions/") + action, QStringLiteral("org.freedesktop.DBus.Properties"),
                            QStringLiteral("Get"), {QStringLiteral("org.qtproject.Qt.QAction"), QStringLiteral("checked")}).toBool();
            };
            result.found = true;
            for (const char *role : {"text", "size", "modificationtime", "creationtime", "accesstime", "type", "comment", "rating", "tags"}) {
                if (checked(QStringLiteral("sort_by_") + QLatin1String(role))) {
                    result.role = role;
                    break;
                }
            }
            result.descending = checked(QStringLiteral("descending"));
            result.folders_first = checked(QStringLiteral("folders_first"));
            result.hidden_last = checked(QStringLiteral("hidden_last"));
            result.show_hidden = checked(QStringLiteral("show_hidden_files"));
            return result;
        }
    }
    return result;
}

void open_url(rust::Str path) { QDesktopServices::openUrl(QUrl::fromLocalFile(q(path))); }

rust::String formatted_size(std::uint64_t bytes) { return r(QLocale().formattedDataSize(qint64(bytes))); }

void copy_text(rust::Str text) { QGuiApplication::clipboard()->setText(q(text)); }

// The file itself: Dolphin pastes the file, chat apps and image editors the picture, text fields the path
void copy_file(rust::Str path, bool with_image)
{
    const QUrl url = QUrl::fromLocalFile(q(path));
    auto data = new QMimeData;
    data->setUrls({url});
    data->setText(q(path));
    data->setData(QStringLiteral("x-special/gnome-copied-files"), "copy\n" + url.toEncoded()); // GNOME apps' paste
    if (with_image)
        data->setImageData(QImage(q(path)));
    QGuiApplication::clipboard()->setMimeData(data);
}

bool move_to_trash(rust::Str path) { return QFile::moveToTrash(q(path)); }

#include <QDateTime>

// Spreadsheet cells, in the system locale. QLocale is reentrant: these run on the sheet's worker thread.
rust::String locale_number(double value) { return r(QLocale().toString(value, 'g', 10)); }
rust::String locale_decimal_point() { return r(QLocale().decimalPoint()); }
rust::String locale_group_separator() { return r(QLocale().groupSeparator()); }

// "2024-01-31" or "2024-01-31T10:00:00" in the locale's short format; anything else as it is
rust::String locale_datetime(rust::Str iso)
{
    const QString text = q(iso);
    if (!text.contains(u'T')) {
        const QDate date = QDate::fromString(text, Qt::ISODate);
        return r(date.isValid() ? QLocale().toString(date, QLocale::ShortFormat) : text);
    }
    const QDateTime time = QDateTime::fromString(text, Qt::ISODate);
    return r(time.isValid() ? QLocale().toString(time, QLocale::ShortFormat) : text);
}

#include <QNetworkAccessManager>
#include <QNetworkRequest>
#include <QQmlApplicationEngine>
#include <QQmlNetworkAccessManagerFactory>

namespace {
// Previews never reach the web: an HTML or Markdown file can't load remote images that report it was opened
class LocalOnly : public QNetworkAccessManager {
protected:
    QNetworkReply *createRequest(Operation op, const QNetworkRequest &request, QIODevice *data) override
    {
        const QUrl url = request.url();
        const bool local = url.isLocalFile() || url.scheme() == u"qrc" || url.scheme() == u"data";
        return QNetworkAccessManager::createRequest(op, local ? request : QNetworkRequest(), data);
    }
};

struct LocalOnlyFactory : QQmlNetworkAccessManagerFactory {
    QNetworkAccessManager *create(QObject *parent) override
    {
        auto *manager = new LocalOnly;
        manager->setParent(parent);
        return manager;
    }
};
}

void block_network(QQmlApplicationEngine &engine)
{
    static LocalOnlyFactory factory;
    engine.setNetworkAccessManagerFactory(&factory);
}

#include <QCryptographicHash>

// Name of a file's thumbnail in ~/.cache/thumbnails (freedesktop spec: md5 of the file URI)
rust::String thumbnail_key(rust::Str path)
{
    const QByteArray uri = QUrl::fromLocalFile(q(path)).toEncoded();
    return r(QString::fromLatin1(QCryptographicHash::hash(uri, QCryptographicHash::Md5).toHex() + ".png"));
}

// By extension only: archive entries aren't on disk, so their content can't be sniffed
rust::String mime_icon_for_name(rust::Str name)
{
    return r(QMimeDatabase().mimeTypeForFile(q(name), QMimeDatabase::MatchExtension).iconName());
}

rust::String local_time(std::int64_t secs)
{
    return r(QDateTime::fromSecsSinceEpoch(secs).toString(QStringLiteral("yyyy-MM-dd HH:mm")));
}

#include <QPainter>
#include <QPdfDocument>

Pixels render_pdf_page(rust::Str pdf, std::int32_t page, std::int32_t width)
{
    QPdfDocument doc; // its own document: QtPdf locks pdfium, so a worker thread may render
    doc.load(q(pdf));
    const QSizeF points = doc.pagePointSize(page);
    if (points.isEmpty())
        return Pixels{0, 0, {}};
    const QSize size(width, qRound(width * points.height() / points.width()));
    QImage image(size, QImage::Format_RGB32);
    image.fill(Qt::white);
    QPainter(&image).drawImage(0, 0, doc.render(page, size));
    Pixels pixels{size.width(), size.height(), {}};
    pixels.data.reserve(size_t(size.width()) * size.height());
    for (int y = 0; y < size.height(); ++y) {
        const auto *line = reinterpret_cast<const std::uint32_t *>(image.constScanLine(y));
        for (int x = 0; x < size.width(); ++x)
            pixels.data.push_back(line[x]);
    }
    return pixels;
}

bool save_png(rust::Str path, std::int32_t width, std::int32_t height, rust::Slice<const std::uint32_t> pixels)
{
    if (pixels.size() != size_t(width) * height)
        return false;
    const QImage image(reinterpret_cast<const uchar *>(pixels.data()), width, height, width * 4, QImage::Format_ARGB32_Premultiplied);
    return image.save(q(path), "PNG");
}

#include <QBuffer>
#include <QLibraryInfo>

// Where Qt's tools live, like balsam
rust::String qt_binaries_dir() { return r(QLibraryInfo::path(QLibraryInfo::BinariesPath)); }

// RGBA rows stored bottom row first (Blender's thumbnails) as PNG bytes; empty if they don't fit the size
// Side panel and file menu
ImageHeader image_header(rust::Str path)
{
    QImageReader reader(q(path)); // reads the header only, no decode
    const QSize size = reader.size();
    return ImageHeader{size.width(), size.height(), r(QString::fromLatin1(reader.format()).toUpper())};
}

// The type and every type it inherits: a .py file also opens in anything that takes text/plain
rust::Vec<rust::String> mime_ancestors(rust::Str path)
{
    const QMimeType mime = QMimeDatabase().mimeTypeForFile(q(path));
    rust::Vec<rust::String> out;
    out.push_back(r(mime.name()));
    for (const QString &name : mime.allAncestors())
        out.push_back(r(name));
    return out;
}

rust::String locale_integer(std::int64_t value) { return r(QLocale().toString(qlonglong(value))); }
#include <QFileInfo>
#include <QMutex>
#include <QPainter>
#include <QPdfDocument>
#include <QQuickImageProvider>

namespace {
// Renders pages with their annotations. One document stays open, reopened when the file changes on disk.
class PdfPageProvider : public QQuickImageProvider
{
public:
    PdfPageProvider() : QQuickImageProvider(QQuickImageProvider::Image, ForceAsynchronousImageLoading) {}

    QImage requestImage(const QString &id, QSize *size, const QSize &requested) override
    {
        // PAGE/PATH/ANYTHING: the path's slashes are percent-encoded; what follows it only makes a new URL
        const int page = id.section(u'/', 0, 0).toInt();
        const QString path = QUrl::fromPercentEncoding(id.section(u'/', 1, 1).toUtf8());
        const QFileInfo info(path);
        const QString version = path + info.lastModified().toString(Qt::ISODateWithMs) + QString::number(info.size());
        QMutexLocker lock(&mutex); // images load on several threads; one document between them
        if (version != loaded) {
            doc.load(path);
            loaded = version;
        }
        const QSize pixels = requested.isValid() ? requested : doc.pagePointSize(page).toSize();
        QImage image(pixels, QImage::Format_RGB32); // pages render transparent: paper white first
        image.fill(Qt::white);
        QPdfDocumentRenderOptions options;
        options.setRenderFlags(QPdfDocumentRenderOptions::RenderFlag::Annotations);
        QPainter(&image).drawImage(0, 0, doc.render(page, pixels, options));
        *size = image.size();
        return image;
    }

private:
    QMutex mutex;
    QPdfDocument doc;
    QString loaded;
};
}

void add_pdf_pages(QQmlApplicationEngine &engine)
{
    engine.addImageProvider(QStringLiteral("pdfpage"), new PdfPageProvider);
}

#include <QFileInfo>
#include <QFontDatabase>
#include <QImageWriter>
#include <QPainter>
#include <QPdfWriter>
#include <QTextDocument>

rust::Vec<rust::String> writable_image_formats()
{
    rust::Vec<rust::String> out;
    for (const QByteArray &format : QImageWriter::supportedImageFormats())
        out.push_back(rust::String(format.toStdString()));
    return out;
}

bool image_animated(rust::Str path) { return QImageReader(q(path)).supportsAnimation(); }

rust::String convert_image(rust::Str src, rust::Str dst)
{
    QImageReader reader(q(src));
    reader.setAutoTransform(true); // honour EXIF rotation
    QImage image = reader.read();
    if (image.isNull())
        return r(reader.errorString());
    const QString out = q(dst);
    if (out.endsWith(QLatin1String(".pdf"))) {
        QPdfWriter writer(out);
        // 1 px = 1 pt, exact: the default fuzzy match would snap the page to the nearest paper size
        writer.setPageSize(QPageSize(QSizeF(image.size()), QPageSize::Point, QString(), QPageSize::ExactMatch));
        writer.setPageMargins(QMarginsF(0, 0, 0, 0));
        writer.setResolution(72);
        QPainter painter(&writer);
        painter.drawImage(0, 0, image);
        return rust::String();
    }
    if ((out.endsWith(QLatin1String(".jpg")) || out.endsWith(QLatin1String(".bmp"))) && image.hasAlphaChannel()) {
        QImage flat(image.size(), QImage::Format_RGB32); // no transparency there: flatten on white
        flat.fill(Qt::white);
        QPainter(&flat).drawImage(0, 0, image);
        image = flat;
    }
    return image.save(out, nullptr, 92) ? rust::String() : rust::String("Qt could not write this format");
}

bool text_to_document(rust::Str src, rust::Str dst, rust::Str mime)
{
    QFile file(q(src));
    if (!file.open(QIODevice::ReadOnly))
        return false;
    const QString text = QString::fromUtf8(file.read(1'000'000)); // same cut as the text view
    QTextDocument doc;
    doc.setBaseUrl(QUrl::fromLocalFile(QFileInfo(q(src)).absolutePath() + QLatin1Char('/')));
    if (q(mime) == QLatin1String("text/markdown")) {
        doc.setMarkdown(text);
    } else if (q(mime) == QLatin1String("text/html")) {
        doc.setHtml(text);
    } else {
        doc.setPlainText(text);
        doc.setDefaultFont(QFontDatabase::systemFont(QFontDatabase::FixedFont));
    }
    const QString out = q(dst);
    if (out.endsWith(QLatin1String(".html"))) {
        QFile html(out);
        return html.open(QIODevice::WriteOnly) && html.write(doc.toHtml().toUtf8()) >= 0;
    }
    QPdfWriter writer(out);
    writer.setPageSize(QPageSize(QPageSize::A4));
    doc.print(&writer);
    return true;
}

rust::String markdown_to_html(rust::Str text)
{
    QTextDocument doc;
    doc.setMarkdown(q(text));
    return r(doc.toHtml());
}

// AI tools on pictures (rust/ai.rs)
namespace {
Pixels to_pixels(const QImage &source)
{
    const QImage image = source.convertToFormat(QImage::Format_ARGB32);
    Pixels pixels{image.width(), image.height(), {}};
    pixels.data.reserve(size_t(image.width()) * image.height());
    for (int y = 0; y < image.height(); ++y) {
        const auto *line = reinterpret_cast<const std::uint32_t *>(image.constScanLine(y));
        for (int x = 0; x < image.width(); ++x)
            pixels.data.push_back(line[x]);
    }
    return pixels;
}

QImage wrap(std::int32_t width, std::int32_t height, rust::Slice<const std::uint32_t> pixels, QImage::Format format)
{
    if (pixels.size() != size_t(width) * height)
        return {};
    return QImage(reinterpret_cast<const uchar *>(pixels.data()), width, height, width * 4, format);
}

QMutex overlay_mutex;
QImage overlay; // the picked subject's dimming and outline, shown by ImageView as image://aioverlay/REVISION

class OverlayProvider : public QQuickImageProvider
{
public:
    OverlayProvider() : QQuickImageProvider(QQuickImageProvider::Image) {}
    QImage requestImage(const QString &, QSize *size, const QSize &) override
    {
        QMutexLocker lock(&overlay_mutex);
        *size = overlay.size();
        return overlay;
    }
};
}

// Upright, as ImageView shows it; 0xAARRGGBB, not premultiplied. Safe on worker threads.
Pixels read_image(rust::Str path)
{
    QImageReader reader(q(path));
    reader.setAutoTransform(true);
    return to_pixels(reader.read());
}

Pixels scale_pixels(std::int32_t width, std::int32_t height, rust::Slice<const std::uint32_t> pixels, std::int32_t to_width, std::int32_t to_height)
{
    return to_pixels(wrap(width, height, pixels, QImage::Format_ARGB32).scaled(to_width, to_height, Qt::IgnoreAspectRatio, Qt::SmoothTransformation));
}

void copy_image(std::int32_t width, std::int32_t height, rust::Slice<const std::uint32_t> pixels)
{
    QApplication::clipboard()->setImage(wrap(width, height, pixels, QImage::Format_ARGB32_Premultiplied).copy());
}

void set_ai_overlay(std::int32_t width, std::int32_t height, rust::Slice<const std::uint32_t> pixels)
{
    const QImage image = wrap(width, height, pixels, QImage::Format_ARGB32_Premultiplied).copy();
    QMutexLocker lock(&overlay_mutex);
    overlay = image;
}

void add_ai_overlay(QQmlApplicationEngine &engine)
{
    engine.addImageProvider(QStringLiteral("aioverlay"), new OverlayProvider);
}
