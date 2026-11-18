// SPDX-License-Identifier: GPL-3.0-or-later
// The side panel (key I): the file's name and type, everything QuickPeek knows about it, then its tools.
// Main.qml sets `path` only while the panel is open, so browsing with it closed reads nothing.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import com.quickpeek

Pane {
    id: panel
    property alias path: details.path
    property url source // the shown file, for the picture thumbnail
    property string icon
    property bool picture // draw a thumbnail instead of the type icon
    leftPadding: 10
    rightPadding: 10
    bottomPadding: 10

    Details { id: details }

    // Python's details are one two-column table, so the label column lines up across sections
    readonly property var cells: {
        if (!details.sections)
            return []
        const out = []
        for (const section of JSON.parse(details.sections)) {
            out.push({ kind: "title", text: section.t })
            for (const [label, value, link] of section.r)
                out.push({ kind: "label", text: label }, { kind: "value", text: value, link: link })
        }
        return out
    }

    // Right-clicks here are not for the file menu behind the panel
    MouseArea { anchors.fill: parent; acceptedButtons: Qt.RightButton }

    Rectangle { // the thin line between the preview and the panel
        x: -panel.leftPadding
        width: 1
        height: parent.height + panel.topPadding + panel.bottomPadding
        y: -panel.topPadding
        color: Qt.alpha(palette.mid, 0.6)
    }

    ScrollView {
        id: scroll
        anchors.fill: parent
        contentWidth: availableWidth // long values wrap instead of scrolling sideways
        ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

        ColumnLayout {
            width: scroll.availableWidth
            spacing: 8

            RowLayout {
                spacing: 10
                Item {
                    implicitWidth: 56
                    implicitHeight: 56
                    Image {
                        anchors.centerIn: parent
                        visible: panel.picture
                        source: panel.picture ? panel.source : ""
                        sourceSize: Qt.size(56, 56) // decodes only a small copy
                        asynchronous: true
                        fillMode: Image.PreserveAspectFit
                    }
                    Kirigami.Icon {
                        anchors.centerIn: parent
                        visible: !panel.picture
                        width: 48
                        height: 48
                        source: panel.icon || "unknown"
                    }
                }
                Label {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    textFormat: Text.StyledText
                    text: "<b>" + escaped(panel.path.split("/").pop()) + "</b><br><small>" + escaped(details.comment) + "</small>"
                    function escaped(s) { return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;") }
                }
            }

            Label {
                visible: !details.sections
                text: "Reading…"
                opacity: 0.6
            }

            GridLayout {
                Layout.fillWidth: true
                columns: 2
                columnSpacing: 6
                rowSpacing: 2
                Repeater {
                    model: panel.cells
                    Label {
                        required property var modelData
                        required property int index
                        Layout.columnSpan: modelData.kind === "title" ? 2 : 1
                        Layout.topMargin: modelData.kind === "title" && index > 0 ? 10 : 0
                        Layout.preferredWidth: modelData.kind === "label" ? panel.availableWidth * 0.36 : -1
                        Layout.fillWidth: modelData.kind !== "label"
                        Layout.alignment: Qt.AlignTop
                        font.bold: modelData.kind === "title"
                        opacity: modelData.kind === "label" ? 0.65 : 1
                        wrapMode: Text.Wrap // breaks long paths and codec names anywhere when it must
                        textFormat: modelData.link ? Text.StyledText : Text.PlainText
                        text: modelData.link ? "<a href=\"" + modelData.link + "\">" + modelData.text + "</a>" : modelData.text
                        onLinkActivated: link => Qt.openUrlExternally(link)
                        HoverHandler { enabled: !!modelData.link; cursorShape: Qt.PointingHandCursor }
                    }
                }
            }

            // ---- Tools page: conversions (ffmpeg, assimp, x2t, images) and the AI tools go here.
            // Python: SidePanel.show_file / start and convert_targets(): one button per target, the latest
            // result as a link that previews it, "Saved next to the original. Existing files are never replaced."
            Label {
                Layout.topMargin: 10
                text: "Tools"
                font.bold: true
            }
            ColumnLayout {
                id: tools
                Layout.fillWidth: true
                spacing: 4
                Label {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    text: "No tools for this type of file."
                    enabled: false // the theme's dimmed text color
                }
            }
        }
    }
}
