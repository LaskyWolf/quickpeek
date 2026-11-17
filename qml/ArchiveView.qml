// SPDX-License-Identifier: GPL-3.0-or-later
// The files inside an archive as a folder tree. Read on a worker thread, so big archives don't freeze the window.
import QtQuick
import QtQuick.Controls
import QtQuick.Controls.impl // IconImage: theme icons by name
import QtQuick.Layouts
import com.quickpeek

ColumnLayout {
    id: view
    property string path
    property var rows: [] // every row, each folder followed by its contents; .d depth, .f folder, .o open
    spacing: 0

    Archive {
        id: archive
        onRowsChanged: { view.rows = JSON.parse(rows); view.refresh() }
    }
    onPathChanged: archive.load(path)
    Component.onCompleted: archive.load(path)

    // Shows the rows whose folders are all open
    function refresh() {
        const y = list.contentY
        const shown = []
        let hideBelow = Infinity
        for (const row of rows) {
            if (row.d > hideBelow)
                continue
            hideBelow = row.f && !row.o ? row.d : Infinity
            shown.push(row)
        }
        list.model = shown
        list.contentY = y // a new model scrolls to the top
    }

    Label {
        Layout.fillWidth: true
        Layout.margins: 8
        text: archive.summary
        elide: Text.ElideRight
    }

    RowLayout {
        Layout.fillWidth: true
        Layout.leftMargin: 8
        Layout.rightMargin: 8
        spacing: 12
        opacity: 0.7
        Label { text: "Name"; Layout.fillWidth: true; leftPadding: 22 }
        Label { text: "Size"; Layout.preferredWidth: 90; horizontalAlignment: Text.AlignRight }
        Label { text: "Modified"; Layout.preferredWidth: 130 }
    }

    ListView {
        id: list
        Layout.fillWidth: true
        Layout.fillHeight: true
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: ScrollBar {}

        delegate: Item {
            width: ListView.view.width
            height: 26

            Rectangle {
                anchors.fill: parent
                color: palette.highlight
                opacity: hover.hovered ? 0.15 : 0
            }

            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                spacing: 12

                RowLayout {
                    Layout.fillWidth: true
                    Layout.leftMargin: modelData.d * 18
                    spacing: 4
                    IconImage {
                        name: modelData.f ? (modelData.o ? "go-down" : "go-next") : ""
                        sourceSize: Qt.size(16, 16)
                        Layout.preferredWidth: 16
                    }
                    IconImage { name: modelData.i; sourceSize: Qt.size(16, 16) }
                    Cell { text: modelData.n; elide: Text.ElideRight; Layout.fillWidth: true }
                }
                Cell {
                    text: modelData.s
                    Layout.preferredWidth: 90
                    horizontalAlignment: Text.AlignRight
                    font.features: ({ "tnum": 1 }) // equal-width digits line the sizes up
                }
                Cell { text: modelData.m; Layout.preferredWidth: 130; font.features: ({ "tnum": 1 }) }
            }

            HoverHandler { id: hover }
            TapHandler {
                enabled: modelData.f
                onTapped: { modelData.o = !modelData.o; view.refresh() }
            }
        }
    }

    // Plain Text, not Label: thousands of Labels in delegates crashed Qt while the app quit
    component Cell: Text { color: palette.windowText }
}
