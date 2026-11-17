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
    static const QCollator collator = [] {
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

#include <QCryptographicHash>
#include <QDateTime>

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
