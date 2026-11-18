// SPDX-License-Identifier: GPL-3.0-or-later
// A spreadsheet: the grid with column letters and row numbers, sheet tabs below, the clicked cell's formula above.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.quickpeek

ColumnLayout {
    id: view
    property alias path: sheet.path
    property int row: -1 // the clicked cell
    property int column: -1
    spacing: 0

    SheetModel {
        id: sheet
        onModelReset: view.row = -1
        onSheetChanged: tabs.currentIndex = sheet.sheet
    }

    FontMetrics { id: metrics }

    RowLayout {
        Layout.fillWidth: true
        Layout.rightMargin: 52 // room for the floating side-panel button
        Layout.margins: 6
        spacing: 6

        Label {
            Layout.minimumWidth: 60
            text: view.row < 0 ? "" : sheet.cellName(view.row, view.column)
        }
        TextField {
            Layout.fillWidth: true
            readOnly: true
            placeholderText: "Click a cell to inspect it"
            text: view.row < 0 ? "" : sheet.data(sheet.index(view.row, view.column), 0x100)
            selectByMouse: true
            activeFocusOnPress: false // keep Space and the arrow keys on the window
            focusPolicy: Qt.NoFocus
        }
        Label {
            visible: text !== ""
            text: sheet.note
            opacity: 0.7
        }
    }

    GridLayout {
        Layout.fillWidth: true
        Layout.fillHeight: true
        columns: 2
        rowSpacing: 0
        columnSpacing: 0

        Item { implicitWidth: rowHeader.width; implicitHeight: columnHeader.height }
        HorizontalHeaderView {
            id: columnHeader
            Layout.fillWidth: true
            syncView: table
            clip: true
        }
        VerticalHeaderView {
            id: rowHeader
            Layout.fillHeight: true
            syncView: table
            clip: true
        }
        TableView {
            id: table
            Layout.fillWidth: true
            Layout.fillHeight: true
            model: sheet
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            keyNavigationEnabled: false
            pointerNavigationEnabled: false
            // Excel's default column is ~64 px; long texts widen theirs, up to a limit
            columnWidthProvider: c => Math.max(64, Math.min(480, sheet.columnChars(c) * metrics.averageCharacterWidth + 12))
            ScrollBar.horizontal: ScrollBar {}
            ScrollBar.vertical: ScrollBar {}

            delegate: Rectangle {
                required property int row
                required property int column
                required property string display
                required property bool bold
                required property bool italic
                required property bool underline
                required property string foreground
                required property string background
                required property int alignment
                readonly property bool current: row === view.row && column === view.column

                implicitHeight: label.implicitHeight + 6
                color: background || palette.base
                border.width: current ? 2 : 0
                border.color: palette.highlight

                Label {
                    id: label
                    anchors.fill: parent
                    anchors.leftMargin: 4
                    anchors.rightMargin: 4
                    verticalAlignment: Text.AlignVCenter
                    horizontalAlignment: alignment
                    elide: alignment === Text.AlignRight ? Text.ElideLeft : Text.ElideRight
                    text: display
                    color: foreground || palette.text
                    font.bold: bold
                    font.italic: italic
                    font.underline: underline
                }
                // Grid lines: seeing the cells is the point of this view
                Rectangle { anchors.right: parent.right; width: 1; height: parent.height; color: palette.mid; opacity: 0.5 }
                Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: palette.mid; opacity: 0.5 }

                TapHandler {
                    onTapped: { view.row = row; view.column = column }
                }
            }
        }
    }

    TabBar {
        id: tabs
        Layout.fillWidth: true
        visible: sheet.sheets.length > 1
        position: TabBar.Footer
        onCurrentIndexChanged: if (currentIndex >= 0) sheet.select(currentIndex) // -1 while the tabs are rebuilt
        Repeater {
            model: sheet.sheets
            TabButton {
                text: modelData
                width: implicitWidth
                focusPolicy: Qt.NoFocus
            }
        }
    }

    BusyIndicator {
        parent: table
        anchors.centerIn: parent
        running: sheet.loading
    }

    Label {
        parent: table
        anchors.centerIn: parent
        width: Math.min(implicitWidth, parent.width - 32)
        visible: sheet.error !== ""
        text: sheet.error
        wrapMode: Text.Wrap
    }
}
