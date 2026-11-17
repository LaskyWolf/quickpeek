// SPDX-License-Identifier: GPL-3.0-or-later
// Office documents and slides: the file's own thumbnail or text at once, then the pages ONLYOFFICE converted.
// Text documents read as pages in PdfView (select and search text); presentations show one slide at a time.
import QtQuick
import org.kde.kirigami as Kirigami
import com.quickpeek

Item {
    id: view
    property alias path: office.path
    property string icon
    readonly property string status: office.loading ? "loading full preview…" : "" // for the window's info line

    Office { id: office }

    function stepVertical(delta) { return pages.item?.stepVertical?.(delta) ?? false }
    function copySelection() { return pages.item?.copySelection?.() ?? false }

    Loader {
        id: pages
        anchors.fill: parent
        sourceComponent: office.pdf.toString() !== "" ? (office.presentation ? slideView : pdfView)
            : office.thumbnail.toString() !== "" ? imageView
            : office.text !== "" ? textView : iconView
    }

    Component { id: pdfView; PdfView { source: office.pdf } }
    Component { id: slideView; SlideView { source: office.pdf; deck: office.deck } }
    Component { id: imageView; ImageView { source: office.thumbnail } }
    Component { id: textView; TextView { text: office.text } }
    Component {
        id: iconView
        Item {
            Kirigami.Icon {
                anchors.centerIn: parent
                width: 128
                height: 128
                source: view.icon
            }
        }
    }
}
