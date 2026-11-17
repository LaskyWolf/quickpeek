// SPDX-License-Identifier: GPL-3.0-or-later
// A folder's contents as icons or thumbnails, with the total size counted on a worker thread.
// Double-clicking an item previews it.
import QtQuick
import QtQuick.Controls
import QtQuick.Controls.impl // IconImage: theme icons by name
import QtQuick.Layouts
import com.quickpeek

ColumnLayout {
    id: view
    property string path
    signal opened(string path)
    spacing: 0

    Folder { id: folder }
    onPathChanged: folder.load(path)
    Component.onCompleted: folder.load(path)

    Label {
        Layout.fillWidth: true
        Layout.margins: 8
        text: folder.summary
        elide: Text.ElideRight
    }

    GridView {
        id: grid
        Layout.fillWidth: true
        Layout.fillHeight: true
        clip: true
        cellWidth: 130
        cellHeight: 140
        model: JSON.parse(folder.items || "[]")
        ScrollBar.vertical: ScrollBar {}

        delegate: Item {
            width: grid.cellWidth
            height: grid.cellHeight

            Rectangle {
                anchors.fill: parent
                anchors.margins: 2
                radius: 4
                color: palette.highlight
                opacity: hover.hovered ? 0.2 : 0
            }

            Column {
                anchors.fill: parent
                anchors.margins: 6
                spacing: 4

                Item {
                    width: parent.width
                    height: 96
                    Image {
                        anchors.centerIn: parent
                        width: 96
                        height: 96
                        visible: modelData.t !== ""
                        source: modelData.t
                        sourceSize: Qt.size(96, 96)
                        fillMode: Image.PreserveAspectFit
                        asynchronous: true
                    }
                    IconImage {
                        anchors.centerIn: parent
                        visible: modelData.t === ""
                        name: modelData.i
                        sourceSize: Qt.size(96, 96)
                    }
                }
                Label {
                    width: parent.width
                    text: modelData.n
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WrapAnywhere
                    maximumLineCount: 2
                    elide: Text.ElideRight
                }
            }

            HoverHandler { id: hover }
            TapHandler { onDoubleTapped: view.opened(modelData.p) }
            ToolTip.visible: hover.hovered
            ToolTip.delay: 600
            ToolTip.text: modelData.n
        }
    }

    Label {
        Layout.margins: 8
        visible: folder.more > 0
        text: "… and " + folder.more + " more"
    }
}
