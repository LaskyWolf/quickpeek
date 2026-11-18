// SPDX-License-Identifier: GPL-3.0-or-later
// The QuickPeek window: the preview, previous/next arrows, and the file bar at the bottom.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window
import org.kde.kirigami as Kirigami
import com.quickpeek

ApplicationWindow {
    id: win
    title: peek.path.split("/").pop()
    minimumWidth: 640
    minimumHeight: 420
    width: Screen.desktopAvailableWidth * 0.6
    height: Screen.desktopAvailableHeight * 0.7

    Peek {
        id: peek
        onReceived: (path, token) => {
            if (path === "--quit") {
                Qt.quit()
            } else if (win.visible && path === peek.path) {
                win.close() // Space on the same file again closes, like Sushi
            } else {
                peek.open(path)
                peek.activate(token)
                win.showAndRaise()
            }
        }
    }

    function showAndRaise() {
        show()
        raise()
        requestActivate()
    }

    function note(text) {
        noteLabel.text = text
        noteTimer.restart()
    }

    Component.onCompleted: {
        peek.start()
        showAndRaise()
        // For tests: quickpeek-window FILE --snapshot=OUT.png saves the window and quits
        const snapshot = Qt.application.arguments.find(a => a.startsWith("--snapshot="))
        if (snapshot)
            snapshotTimer.file = snapshot.slice(11)
    }

    Timer {
        id: snapshotTimer
        property string file
        running: file !== ""
        interval: 1500
        onTriggered: root.grabToImage(result => { result.saveToFile(file); Qt.quit() })
    }

    // Stay running hidden: the next file shows up instantly instead of starting the app again
    onClosing: close => {
        close.accepted = false
        if (preview.item?.setEditing?.(false, () => win.close()) === false)
            return // it asks about unsaved drawings first
        hide()
        peek.clear() // free the preview's memory while hidden
    }

    Shortcut { sequences: ["Space", "Q"]; onActivated: win.close() }
    Shortcut { sequence: "Escape"; onActivated: win.visibility === Window.FullScreen ? win.showNormal() : win.close() }
    Shortcut { sequence: "Return"; onActivated: { peek.openExternal(); win.close() } }
    Shortcut { sequences: ["Left", "Up"]; onActivated: peek.step(-1) }
    Shortcut { sequences: ["Right", "Down"]; onActivated: peek.step(1) }
    Shortcut {
        sequences: [StandardKey.Copy]
        onActivated: {
            if (!preview.item?.copySelection?.()) { // selected text first, else the file
                peek.copyFile()
                win.note("copied")
            }
        }
    }
    Shortcut { sequence: "Ctrl+Shift+C"; onActivated: { peek.copyPath(); win.note("path copied") } }
    Shortcut { sequence: "F2"; onActivated: renameDialog.open() }
    Shortcut { sequences: [StandardKey.Delete]; onActivated: win.trash() }
    Shortcut { sequence: "F11"; onActivated: win.visibility === Window.FullScreen ? win.showNormal() : win.showFullScreen() }

    function trash() {
        const name = title
        if (!peek.trash())
            win.note("couldn't move “" + name + "” to the Trash")
        else if (peek.kind === "")
            win.close()
        else
            win.note("“" + name + "” moved to the Trash")
    }

    ColumnLayout {
        id: root
        anchors.fill: parent
        spacing: 0

        Item {
            id: previewArea
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true

            Loader {
                id: preview
                anchors.fill: parent
                sourceComponent: ({ image: imageView, text: textView, font: fontView, audio: mediaView, video: mediaView, sheet: sheetView, pdf: pdfView, folder: folderView, archive: archiveView, markdown: markdownView, html: htmlView, other: iconView })[peek.kind] ?? null
            }

            // Decoded ahead on worker threads, so flipping to the neighbours is instant
            Repeater {
                model: [peek.previous, peek.next]
                Image { visible: false; asynchronous: true; source: modelData }
            }

            NavArrow { anchors.left: parent.left; iconName: "go-previous"; tip: "Previous file (←)"; onClicked: peek.step(-1) }
            NavArrow { anchors.right: parent.right; iconName: "go-next"; tip: "Next file (→)"; onClicked: peek.step(1) }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.margins: 8
            spacing: 8

            // Drag it into another app to hand over the file
            ToolButton {
                icon.name: peek.icon
                icon.width: 32
                icon.height: 32
                flat: true
                focusPolicy: Qt.NoFocus
                ToolTip.visible: hovered
                ToolTip.text: "Drag the file into another app"
                Drag.dragType: Drag.Automatic
                Drag.supportedActions: Qt.CopyAction | Qt.LinkAction // never Move: dropping must not take the file away
                Drag.mimeData: ({ "text/uri-list": peek.source.toString() })
                Drag.active: dragHandler.active
                DragHandler { id: dragHandler; target: null }
            }

            Label {
                Layout.fillWidth: true
                textFormat: Text.RichText
                elide: Text.ElideRight
                text: peek.details + (noteLabel.text ? "  ·  <i>" + noteLabel.text + "</i>" : "")
                Label { id: noteLabel; visible: false }
                Timer { id: noteTimer; interval: 2000; onTriggered: noteLabel.text = "" }
            }

            Button {
                text: "Open"
                icon.name: "document-open"
                focusPolicy: Qt.NoFocus // clicking it must not steal Space and the arrow keys
                onClicked: { peek.openExternal(); win.close() }
            }
        }
    }

    component NavArrow: AbstractButton {
        property string iconName
        property string tip
        anchors.verticalCenter: parent.verticalCenter
        width: 56
        height: 112 // a generous target, while the drawn circle stays small
        visible: peek.count > 1
        focusPolicy: Qt.NoFocus
        opacity: hovered ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: 150 } }
        ToolTip.visible: hovered
        ToolTip.text: tip
        contentItem: Item {
            Rectangle {
                anchors.centerIn: parent
                width: 40
                height: 40
                radius: 20
                color: Qt.alpha(palette.window, 0.88)
                border.color: Qt.alpha(palette.mid, 0.47)
                Kirigami.Icon {
                    anchors.centerIn: parent
                    width: 20
                    height: 20
                    source: iconName
                }
            }
        }
        background: null
    }

    Component { id: imageView; ImageView { source: peek.source } }
    Component { id: sheetView; SheetView { path: peek.path } }
    Component { id: folderView; FolderView { path: peek.path; onOpened: path => peek.open(path) } }
    Component { id: archiveView; ArchiveView { path: peek.path } }

    Component { id: fontView; FontView { source: peek.source; icon: peek.icon } }
    Component { id: mediaView; MediaView { source: peek.source; audio: peek.kind === "audio"; icon: peek.icon } }

    Component { id: textView; TextView { text: peek.text; name: win.title } }
    Component { id: markdownView; TextView { text: peek.text; format: "markdown"; baseUrl: peek.source } }
    Component { id: htmlView; TextView { text: peek.text; format: "html"; baseUrl: peek.source } }
    Component { id: pdfView; PdfView { source: peek.source } }

    Component {
        id: iconView
        Item {
            Kirigami.Icon {
                anchors.centerIn: parent
                width: 128
                height: 128
                source: peek.icon
            }
        }
    }

    Dialog {
        id: renameDialog
        title: "Rename"
        anchors.centerIn: parent
        modal: true
        standardButtons: Dialog.Ok | Dialog.Cancel
        onAboutToShow: {
            const name = win.title
            const dot = name.lastIndexOf(".")
            nameField.text = name
            nameField.select(0, dot > 0 ? dot : name.length) // like Dolphin: the extension stays unselected
            nameField.forceActiveFocus()
        }
        onAccepted: {
            const error = peek.rename(nameField.text)
            if (error)
                win.note(error)
        }
        TextField { id: nameField; implicitWidth: 360; onAccepted: renameDialog.accept() }
    }
}
