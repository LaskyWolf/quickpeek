// SPDX-License-Identifier: GPL-3.0-or-later
// A font's family name and a sample line at several sizes. Shows the file icon when Qt can't load the font.
import QtQuick
import QtQuick.Controls

Item {
    id: view
    property alias source: loader.source
    property string icon // shown when the font doesn't load

    // ponytail: Qt keeps every FontLoader font registered until the app quits; remove them via a C++ helper if memory grows
    FontLoader { id: loader }

    ScrollView {
        anchors.fill: parent
        visible: loader.status === FontLoader.Ready
        contentWidth: availableWidth // wrap long lines instead of scrolling sideways

        Column {
            width: parent.width
            padding: 16
            spacing: 12

            component Sample: Label {
                width: parent.width - 32
                font.family: loader.name
                wrapMode: Text.Wrap
            }

            Sample { text: loader.name; font.pointSize: 24; font.bold: true }
            Repeater {
                model: [12, 18, 24, 36, 48, 72]
                Sample { text: "The quick brown fox jumps over the lazy dog"; font.pointSize: modelData }
            }
            Sample {
                text: "ABCDEFGHIJKLMNOPQRSTUVWXYZ\nabcdefghijklmnopqrstuvwxyz\n0123456789 !?&@€$%()"
                font.pointSize: 20
            }
        }
    }

    ToolButton {
        anchors.centerIn: parent
        visible: loader.status === FontLoader.Error
        icon.name: view.icon
        icon.width: 128
        icon.height: 128
        enabled: false
        flat: true
        background: null
    }
}
