// SPDX-License-Identifier: GPL-3.0-or-later
// Plain text, highlighted code, or rendered Markdown/HTML, with a find box (Ctrl+F) over the top-right corner.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.quickpeek

Control {
    id: view
    property string text
    property string name // file name: picks the code language
    property url baseUrl // Markdown/HTML load relative images from here
    property string format: "plain" // "plain" or "html" (Markdown arrives as HTML)
    readonly property bool dark: palette.window.hslLightness < 0.5
    readonly property bool code: format === "plain" && highlighter.html !== ""

    padding: 0

    background: Rectangle { color: view.code && highlighter.background ? highlighter.background : view.palette.base }

    Highlighter { id: highlighter }

    function highlight() {
        if (format === "plain")
            highlighter.highlight(text, name, dark)
    }
    onTextChanged: Qt.callLater(highlight)
    onNameChanged: Qt.callLater(highlight)
    onDarkChanged: Qt.callLater(highlight)
    Component.onCompleted: highlight()

    /// Ctrl+C: copies the selected text. False when nothing is selected, so the window copies the file.
    function copySelection() {
        if (!area.selectedText)
            return false
        area.copy()
        return true
    }

    // For the right-click menu's Copy / Select All
    readonly property bool hasSelection: area.selectedText !== ""
    function selectAll() { area.selectAll() }

    contentItem: Flickable {
        id: flick
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: ScrollBar {}
        ScrollBar.horizontal: ScrollBar {}

        TextArea.flickable: TextArea {
            id: area
            text: view.code ? highlighter.html : view.text
            textFormat: view.format === "html" || view.code ? TextEdit.RichText : TextEdit.PlainText
            baseUrl: view.baseUrl
            font.family: view.format === "plain" ? "monospace" : Qt.application.font.family
            wrapMode: view.format === "plain" ? TextEdit.NoWrap : TextEdit.Wrap
            color: view.palette.text
            readOnly: true
            selectByMouse: true
            persistentSelection: true // the current match stays highlighted while typing in the find box
            activeFocusOnPress: false // keep Space and the arrow keys on the window
            background: null
            onLinkActivated: link => Qt.openUrlExternally(link)
            onTextChanged: Qt.callLater(finder.find, 0) // highlighted text replaced the plain one, or another file
            HoverHandler { cursorShape: area.hoveredLink ? Qt.PointingHandCursor : Qt.IBeamCursor }
        }
    }

    Shortcut { sequence: StandardKey.Find; onActivated: finder.open() }

    Pane {
        id: finder
        property var matches: [] // start positions of the matches
        property int current: -1
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.margins: 10
        anchors.topMargin: 50 // below the floating side-panel button
        anchors.rightMargin: 12 + (flick.ScrollBar.vertical.visible ? flick.ScrollBar.vertical.width : 0)
        padding: 4
        visible: false
        background: Rectangle {
            color: view.palette.window
            radius: 6
            border.color: Qt.alpha(view.palette.mid, 0.6)
        }

        function open() {
            visible = true
            field.forceActiveFocus()
            field.selectAll()
        }

        function close() {
            field.clear() // clears the highlight too
            visible = false
            view.forceActiveFocus() // the window's keys work again
        }

        /// delta 0: a new search from the top; 1 / -1: next / previous match, wrapping around.
        function find(delta) {
            const needle = field.text
            if (delta === 0) {
                matches = []
                if (needle) {
                    // Case-insensitive; positions match the document's
                    const pattern = new RegExp(needle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi")
                    const haystack = area.getText(0, area.length), found = []
                    for (let m; (m = pattern.exec(haystack));)
                        found.push(m.index)
                    matches = found
                }
                current = matches.length ? 0 : -1
            } else if (matches.length) {
                current = (current + delta + matches.length) % matches.length
            }
            if (current < 0) {
                area.deselect()
                return
            }
            const start = matches[current]
            area.select(start, start + needle.length)
            const rect = area.positionToRectangle(start) // scroll the match into view
            flick.contentY = Math.max(0, Math.min(flick.contentHeight - flick.height, rect.y - flick.height / 3))
            flick.contentX = Math.max(0, Math.min(flick.contentWidth - flick.width, rect.x - flick.width / 2))
        }

        RowLayout {
            spacing: 2
            TextField {
                id: field
                placeholderText: "Find"
                Layout.preferredWidth: 220
                onTextChanged: finder.find(0)
                // Keys the window's shortcuts would take otherwise
                Keys.onShortcutOverride: event => event.accepted = [Qt.Key_Escape, Qt.Key_Return, Qt.Key_Enter,
                    Qt.Key_Up, Qt.Key_Down].includes(event.key)
                Keys.onEscapePressed: finder.close()
                Keys.onReturnPressed: event => finder.find(event.modifiers & Qt.ShiftModifier ? -1 : 1)
                Keys.onEnterPressed: event => finder.find(event.modifiers & Qt.ShiftModifier ? -1 : 1)
                Keys.onUpPressed: finder.find(-1)
                Keys.onDownPressed: finder.find(1)
            }
            Label {
                Layout.minimumWidth: 70
                horizontalAlignment: Text.AlignRight
                text: !field.text ? "" : finder.matches.length ? finder.matches.length + " matches" : "no matches"
            }
            FindButton { iconName: "go-up"; tip: "Previous match (Shift+Enter)"; onClicked: finder.find(-1) }
            FindButton { iconName: "go-down"; tip: "Next match (Enter)"; onClicked: finder.find(1) }
            FindButton { iconName: "window-close"; tip: "Close (Esc)"; onClicked: finder.close() }
        }
    }

    component FindButton: ToolButton {
        property string iconName
        property string tip
        icon.name: iconName
        focusPolicy: Qt.NoFocus
        ToolTip.visible: hovered
        ToolTip.text: tip
    }
}
