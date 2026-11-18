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

    signal openFile(string path) // a converted file's name was clicked: preview it here

    Details { id: details }
    Tools { id: tools; path: details.path }
    Desktop { id: desktop }

    // The details are one two-column table, so the label column lines up across sections
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

            // ---- Tools page: one button per conversion, the latest job above them
            Label {
                Layout.topMargin: 10
                text: "Tools"
                font.bold: true
            }
            RowLayout { // the latest job: progress, then a link to what it made, or the error
                Layout.fillWidth: true
                visible: tools.status !== ""
                spacing: 4
                Label {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    textFormat: Text.StyledText
                    text: tools.status
                    onLinkActivated: panel.openFile(tools.result)
                    HoverHandler { enabled: tools.result !== ""; cursorShape: Qt.PointingHandCursor }
                    ToolTip.visible: tools.result !== "" && hoveredLink !== ""
                    ToolTip.text: "Preview it here"
                }
                BusyIndicator {
                    visible: tools.busy
                    running: tools.busy
                    Layout.preferredWidth: 24
                    Layout.preferredHeight: 24
                }
                ToolButton {
                    visible: tools.result !== ""
                    icon.name: "document-open-folder"
                    focusPolicy: Qt.NoFocus
                    onClicked: desktop.showInFolder(tools.result)
                    ToolTip.visible: hovered
                    ToolTip.text: "Show in folder"
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 4
                Repeater {
                    model: tools.targets ? JSON.parse(tools.targets) : []
                    Button {
                        required property var modelData
                        Layout.fillWidth: true
                        focusPolicy: Qt.NoFocus // clicking it must not steal Space and the arrow keys
                        text: modelData[1]
                        icon.name: modelData[2]
                        onClicked: tools.start(modelData[0])
                    }
                }
            }            Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: tools.targets === "[]" || !tools.targets ? "No tools for this type of file."
                                                               : "Saved next to the original. Existing files are never replaced."
                enabled: false // the theme's dimmed text color
            }
        }
    }
}
