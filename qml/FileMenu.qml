// SPDX-License-Identifier: GPL-3.0-or-later
// The file menu: right-click anywhere in the window. Text views add their Copy / Select All on top.
// Keys are hints only: the window's Shortcuts do the work, and an Action shortcut would clash with them.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window
import org.kde.kirigami as Kirigami
import com.quickpeek

Menu {
    id: menu
    required property var win // Main.qml's window: sideOpen, note(), trash()
    required property Peek peek
    property Item view // the preview; a text view has selectAll() and copySelection()
    readonly property bool textView: !!view && typeof view.selectAll === "function"
    property var apps: []
    signal renameRequested()

    Desktop { id: desktop }

    onAboutToShow: {
        apps = JSON.parse(desktop.apps(peek.path))
        // set each time: clicking a checkable item replaces a binding with its own toggle
        sideItem.checked = win.sideOpen
        fullItem.checked = win.visibility === Window.FullScreen
        for (let i = 0; i < count; i++) {
            const item = itemAt(i)
            if (item.subMenu === openWith)
                item.visible = apps.length > 0
        }
    }

    Entry { visible: menu.textView; iconName: "edit-copy"; text: "Copy"; hint: "Ctrl+C"; enabled: menu.textView && menu.view.hasSelection; onTriggered: menu.view.copySelection() }
    Entry { visible: menu.textView; iconName: "edit-select-all"; text: "Select All"; hint: "Ctrl+A"; onTriggered: menu.view.selectAll() }
    MenuSeparator { visible: menu.textView; height: visible ? implicitHeight : 0 }

    Entry { iconName: "document-open"; text: "Open"; hint: "Return"; onTriggered: { peek.openExternal(); win.close() } }
    Menu {
        id: openWith
        title: "Open With"
        icon.name: "system-run"
        Instantiator {
            model: menu.apps
            MenuItem {
                required property var modelData
                text: modelData[0]
                icon.name: modelData[1]
                onTriggered: { desktop.openWith(modelData[2], peek.path); win.close() }
            }
            onObjectAdded: (index, item) => openWith.insertItem(index, item)
            onObjectRemoved: (index, item) => openWith.removeItem(item)
        }
    }
    Entry { iconName: "document-open-folder"; text: "Show in Folder"; onTriggered: desktop.showInFolder(peek.path) }
    MenuSeparator {}
    Entry {
        iconName: "edit-copy"
        text: "Copy"
        hint: "Ctrl+C"
        onTriggered: {
            if (!menu.view?.copySelection?.()) { // selected text first, else the file
                peek.copyFile()
                win.note("copied")
            }
        }
    }
    Entry { iconName: "edit-copy-path"; text: "Copy Path"; hint: "Ctrl+Shift+C"; onTriggered: { peek.copyPath(); win.note("path copied") } }
    MenuSeparator {}
    Entry { iconName: "edit-rename"; text: "Rename…"; hint: "F2"; onTriggered: menu.renameRequested() }
    Entry { iconName: "user-trash"; text: "Move to Trash"; hint: "Del"; onTriggered: win.trash() }
    Entry {
        visible: peek.kind === "image" && desktop.canSetWallpaper()
        iconName: "viewimage"
        text: "Set as Wallpaper"
        onTriggered: desktop.setWallpaper(peek.path)
    }
    MenuSeparator {}
    Entry { id: sideItem; iconName: "sidebar-expand-right"; text: "Details and Tools"; hint: "I"; checkable: true; onTriggered: win.sideOpen = !win.sideOpen }
    Entry {
        iconName: "view-fullscreen"
        text: "Full Screen"
        hint: "F11"
        id: fullItem
        checkable: true
        onTriggered: win.visibility === Window.FullScreen ? win.showNormal() : win.showFullScreen()
    }
    MenuSeparator {}
    Entry { iconName: "window-close"; text: "Close"; hint: "Space"; onTriggered: win.close() }

    // A menu item with its key written on the right, like a QAction's shortcut in the Python app
    component Entry: MenuItem {
        id: entry
        property string iconName
        property string hint
        icon.name: iconName
        contentItem: RowLayout {
            spacing: Kirigami.Units.smallSpacing
            Item { // the check box column, as the style's own MenuItem draws it
                Layout.preferredWidth: (entry.ListView.view && entry.ListView.view.hasCheckables) || entry.checkable
                    ? entry.indicator.width : Kirigami.Units.smallSpacing
            }
            Kirigami.Icon {
                source: entry.iconName
                Layout.preferredWidth: Kirigami.Units.iconSizes.small
                Layout.preferredHeight: Kirigami.Units.iconSizes.small
            }
            Label {
                Layout.fillWidth: true
                text: entry.text
                color: entry.pressed ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
            }
            Label {
                text: entry.hint
                leftPadding: Kirigami.Units.largeSpacing
                color: entry.pressed ? Kirigami.Theme.highlightedTextColor : Kirigami.Theme.textColor
                opacity: 0.7
            }
        }
    }
}
