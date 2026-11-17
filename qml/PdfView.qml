// SPDX-License-Identifier: GPL-3.0-or-later
// A PDF for reading: every page in one scrolling column, zoom, PageUp/PageDown, Ctrl+F search, drag to select text.
// An edit mode would swap `reader` for a drawing canvas and add its button to the toolbar.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Pdf

ColumnLayout {
    id: view
    property alias source: doc.source
    property string fit: "width" // "width", "page", or "" once you zoom by hand
    readonly property var found: reader.searchModel
    spacing: 0

    PdfDocument { id: doc; onStatusChanged: view.refit() }

    function refit() {
        if (doc.status !== PdfDocument.Ready || reader.width <= 0)
            return
        const margin = 24 // the scrollbar and a little air around the page
        if (fit === "width")
            reader.scaleToWidth(reader.width - margin, reader.height)
        else if (fit === "page")
            reader.scaleToPage(reader.width - margin, reader.height - margin)
    }
    onFitChanged: refit()

    function zoom(factor) {
        fit = ""
        reader.renderScale = Math.min(Math.max(reader.renderScale * factor, 0.1), 10)
    }

    function goTo(page) {
        if (page >= 0 && page < doc.pageCount)
            reader.goToPage(page)
    }

    function step(delta) {
        if (found.count === 0)
            return
        const current = found.currentResult < 0 ? (delta > 0 ? -1 : 0) : found.currentResult
        found.currentResult = (current + delta + found.count) % found.count
    }

    function copy() {
        clipboard.text = reader.selectedText
        clipboard.selectAll()
        clipboard.copy()
    }
    TextEdit { id: clipboard; visible: false } // QML's way to the clipboard

    Shortcut { sequences: [StandardKey.MoveToPreviousPage]; onActivated: view.goTo(reader.currentPage - 1) }
    Shortcut { sequences: [StandardKey.MoveToNextPage]; onActivated: view.goTo(reader.currentPage + 1) }
    Shortcut { sequences: [StandardKey.ZoomIn, "Ctrl+="]; onActivated: view.zoom(1.25) }
    Shortcut { sequences: [StandardKey.ZoomOut]; onActivated: view.zoom(1 / 1.25) }
    Shortcut { sequences: [StandardKey.Find]; onActivated: searchBar.open() }
    Shortcut { sequences: [StandardKey.SelectAll]; onActivated: reader.selectAll() } // the page in view

    // Ctrl+C copies the selected text instead of the file while there is some.
    // It takes the keyboard only for that key: Space and the arrows still reach the window.
    Item {
        id: copyKeys
        Keys.onShortcutOverride: event => event.accepted = reader.selectedText !== "" && event.matches(StandardKey.Copy)
        Keys.onPressed: event => {
            if (event.matches(StandardKey.Copy)) {
                view.copy()
                event.accepted = true
            }
        }
    }

    RowLayout {
        Layout.fillWidth: true
        Layout.margins: 4
        Layout.leftMargin: 12
        spacing: 2

        Label {
            text: doc.pageCount > 0 ? `Page ${reader.currentPage + 1} of ${doc.pageCount}` : ""
            enabled: false // dimmed: information, not a control
        }
        Item { Layout.fillWidth: true }
        Tool { name: "edit-copy"; tip: "Copy the selected text (Ctrl+C)"; label: "Copy"; visible: reader.selectedText !== ""; onClicked: view.copy() }
        Tool { name: "edit-find"; tip: "Find (Ctrl+F)"; onClicked: searchBar.open() }
        Tool { name: "zoom-out"; tip: "Zoom out (Ctrl+-)"; onClicked: view.zoom(1 / 1.25) }
        Tool { name: "zoom-in"; tip: "Zoom in (Ctrl++)"; onClicked: view.zoom(1.25) }
        Tool { name: "zoom-fit-width"; tip: "Fit width"; checked: view.fit === "width"; onClicked: view.fit = "width" }
        Tool { name: "zoom-fit-best"; tip: "Fit page"; checked: view.fit === "page"; onClicked: view.fit = "page" }
    }

    Item {
        Layout.fillWidth: true
        Layout.fillHeight: true
        clip: true

        // The pages' text selections ask for keyboard focus. Inside a scope that never gets it, they can't take
        // Space and the arrows from the window.
        FocusScope {
            anchors.fill: parent
            PdfMultiPageView {
                id: reader
                anchors.fill: parent
                document: doc
                onWidthChanged: view.refit()
                onHeightChanged: view.refit()
                onSelectedTextChanged: if (selectedText) copyKeys.forceActiveFocus()
                // Its columns are wider than the view by a scrollbar, which always shows a horizontal scrollbar.
                // ponytail: reaches into Qt's own component; copy PdfMultiPageView.qml here if a Qt update breaks it.
                Component.onCompleted: {
                    const table = children.find(child => child instanceof TableView)
                    table.columnWidthProvider = () => Math.max(table.width, doc.maxPageWidth * renderScale)
                    table.pointerNavigationEnabled = false // a click would take the keys from the window
                    table.acceptedButtons = Qt.NoButton // dragging the mouse selects text; wheel and touch still scroll
                }
            }
        }

        // Ctrl+wheel zooms; without Ctrl the wheel falls through to the pages and scrolls
        Item {
            anchors.fill: parent
            WheelHandler {
                target: null
                acceptedModifiers: Qt.ControlModifier
                onWheel: event => view.zoom(event.angleDelta.y > 0 ? 1.25 : 1 / 1.25)
            }
        }

        // Find box over the top-right corner. Typing searches, Enter / Shift+Enter jump to the next / previous match.
        Pane {
            id: searchBar
            anchors.top: parent.top
            anchors.right: parent.right
            anchors.margins: 12
            anchors.rightMargin: 24 // clear of the scrollbar
            visible: false
            padding: 4

            function open() {
                visible = true
                field.forceActiveFocus()
                field.selectAll()
            }

            function close() {
                field.clear() // clears the highlights too
                visible = false
                copyKeys.forceActiveFocus() // hand the keys back without keeping a text field focused
            }

            RowLayout {
                spacing: 2
                TextField {
                    id: field
                    implicitWidth: 220
                    placeholderText: "Find"
                    onTextChanged: reader.searchString = text
                    // Escape closes the box, not the window
                    Keys.onShortcutOverride: event => event.accepted = event.key === Qt.Key_Escape
                    Keys.onEscapePressed: searchBar.close()
                    Keys.onReturnPressed: event => view.step(event.modifiers & Qt.ShiftModifier ? -1 : 1)
                    Keys.onEnterPressed: event => view.step(event.modifiers & Qt.ShiftModifier ? -1 : 1)
                }
                Label {
                    Layout.minimumWidth: 70
                    horizontalAlignment: Text.AlignRight
                    text: !field.text ? "" : view.found.currentResult >= 0 && view.found.count > 0
                        ? `${view.found.currentResult + 1} of ${view.found.count}` : `${view.found.count} matches`
                }
                Tool { name: "go-up"; tip: "Previous match (Shift+Enter)"; onClicked: view.step(-1) }
                Tool { name: "go-down"; tip: "Next match (Enter)"; onClicked: view.step(1) }
                Tool { name: "window-close"; tip: "Close (Esc)"; onClicked: searchBar.close() }
            }
        }
    }

    component Tool: ToolButton {
        property string name
        property string tip
        property string label
        icon.name: name
        text: label
        display: label ? AbstractButton.TextBesideIcon : AbstractButton.IconOnly
        focusPolicy: Qt.NoFocus
        ToolTip.visible: hovered
        ToolTip.text: tip
    }
}
