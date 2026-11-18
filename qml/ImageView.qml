// SPDX-License-Identifier: GPL-3.0-or-later
// A picture, fitted to the window. Wheel zooms around the pointer, dragging pans, double-click fits again.
// The toolbar's AI tools: Subject (S) picks what to keep and copies or saves it cut out; Text (T) finds the
// lines of text, click one to see and copy its paragraph.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.quickpeek

ColumnLayout {
    id: view
    property alias source: image.source
    property string path // the file: the cut-out is saved next to it, the found text is cached by it
    property string mode // "", "subject" or "text": one click mode at a time
    property int block: -1 // the clicked paragraph of found text
    readonly property var lines: tools.lines ? JSON.parse(tools.lines) : []
    readonly property bool ai: tools.available()
    readonly property bool picked: tools.overlay > 0
    spacing: 0

    ImageTools { id: tools }

    onSourceChanged: {
        flick.zoom = 0
        mode = ""
        tools.reset()
    }
    onModeChanged: {
        block = -1
        if (mode === "subject")
            tools.startSubject(file())
        else
            tools.stopSubject()
        if (mode === "text")
            tools.readText(file(), path)
    }

    function file() { return decodeURIComponent(source.toString().replace(/^file:\/\//, "")) }

    function toggle(name) {
        if (!ai)
            tools.status = "Install onnxruntime-cpu to use the AI tools"
        else
            mode = mode === name ? "" : name
    }

    function blockText(n) { return lines.filter(l => l.block === n).map(l => l.text).join("\n") }

    /// Ctrl+C: copies the clicked paragraph, else the subject. False when neither, so the window copies the file.
    function copySelection() {
        if (mode === "text" && block >= 0) {
            tools.copyText(blockText(block))
            return true
        }
        if (mode === "subject" && picked) {
            tools.copySubject()
            return true
        }
        return false
    }

    Shortcut { sequences: ["+", "="]; onActivated: flick.zoomAt(1.25, flick.width / 2, flick.height / 2) }
    Shortcut { sequence: "-"; onActivated: flick.zoomAt(1 / 1.25, flick.width / 2, flick.height / 2) }
    Shortcut { sequence: "1"; onActivated: flick.zoom = 1 }
    Shortcut { sequence: "0"; onActivated: flick.zoom = 0 }
    Shortcut { sequence: "S"; enabled: subjectButton.enabled; onActivated: view.toggle("subject") }
    Shortcut { sequence: "T"; onActivated: view.toggle("text") }
    Shortcut { sequences: [StandardKey.Undo]; enabled: view.mode === "subject"; onActivated: tools.pick(0, 0, "undo", reach.value) }

    RowLayout {
        Layout.fillWidth: true
        Layout.rightMargin: 44 // room for the floating side-panel button
        Layout.margins: 4
        Layout.leftMargin: 12
        spacing: 2

        Label {
            Layout.fillWidth: true
            text: tools.status
            elide: Text.ElideRight
            clip: true
            enabled: false // dimmed: a status, not a control
        }
        Label { text: "Less"; enabled: false; visible: view.picked }
        Slider {
            id: reach
            visible: view.picked
            from: -8
            to: 8
            stepSize: 1
            implicitWidth: 110
            focusPolicy: Qt.NoFocus
            ToolTip.visible: hovered
            ToolTip.text: "Select less ⟷ more"
            onMoved: refit.restart()
            Timer { id: refit; interval: 60; onTriggered: tools.pick(0, 0, "reach", reach.value) } // slider moves come fast: redo the mask once
        }
        Label { text: "More"; enabled: false; visible: view.picked }
        Tool { name: "edit-copy"; label: "Copy"; tip: "Copy the subject (Ctrl+C)"; visible: view.picked; onClicked: tools.copySubject() }
        Tool { name: "document-save"; label: "Save"; tip: "Save the subject as a PNG next to the picture"; visible: view.picked && view.path !== ""; onClicked: tools.saveSubject(view.path) }
        Tool {
            name: "edit-copy"
            label: "Copy all"
            tip: "Copy all the text found"
            visible: view.mode === "text" && view.lines.length > 0
            onClicked: {
                const blocks = [...new Set(view.lines.map(l => l.block))]
                tools.copyText(blocks.map(view.blockText).join("\n\n"))
                tools.status = "Text copied"
            }
        }
        Tool {
            id: subjectButton
            name: "tools-wizard"
            label: "Subject"
            tip: view.ai ? "Pick a subject: click what to keep (S)" : "Install onnxruntime-cpu to pick subjects"
            checked: view.mode === "subject"
            enabled: image.frameCount <= 1 // an animation's frames move under the mask
            onClicked: view.toggle("subject")
        }
        Tool {
            name: "edit-select-text"
            label: "Text"
            tip: view.ai ? "Find text in this image, then click it to copy (T)" : "Install onnxruntime-cpu to find text"
            checked: view.mode === "text"
            onClicked: view.toggle("text")
        }
        Tool { name: "zoom-out"; tip: "Zoom out (-)"; onClicked: flick.zoomAt(1 / 1.25, flick.width / 2, flick.height / 2) }
        Label {
            text: Math.round(flick.scale * 100) + "%"
            horizontalAlignment: Text.AlignHCenter
            Layout.minimumWidth: 44
        }
        Tool { name: "zoom-in"; tip: "Zoom in (+)"; onClicked: flick.zoomAt(1.25, flick.width / 2, flick.height / 2) }
        Tool { name: "zoom-original"; tip: "Actual size (1)"; checked: flick.zoom === 1; onClicked: flick.zoom = 1 }
        Tool { name: "zoom-fit-best"; tip: "Fit window (0)"; checked: flick.zoom === 0; onClicked: flick.zoom = 0 }
    }

    Flickable {
        id: flick
        Layout.fillWidth: true
        Layout.fillHeight: true
        clip: true
        property real zoom: 0 // 0 = fit to the window
        readonly property real fitScale: image.implicitWidth > 0
            ? Math.min(1, width / image.implicitWidth, height / image.implicitHeight) : 1 // small pictures stay 1:1
        readonly property real scale: zoom || fitScale

        contentWidth: Math.max(width, image.implicitWidth * scale)
        contentHeight: Math.max(height, image.implicitHeight * scale)
        boundsBehavior: Flickable.StopAtBounds
        interactive: zoom > 0

        AnimatedImage { // also plays GIF, WebP and APNG animations
            id: image
            asynchronous: true
            autoTransform: true // photos taken sideways stand upright
            smooth: true
            mipmap: true // no shimmer when a big photo is shrunk
            width: implicitWidth * flick.scale
            height: implicitHeight * flick.scale
            x: (flick.contentWidth - width) / 2
            y: (flick.contentHeight - height) / 2

            // The picked subject: everything else dimmed, the subject outlined
            Image {
                anchors.fill: parent
                visible: view.mode === "subject" && view.picked
                source: view.picked ? "image://aioverlay/" + tools.overlay : ""
                cache: false
                smooth: true
            }

            // Found text: a highlight on each line, brighter on the clicked paragraph
            Repeater {
                model: view.mode === "text" ? view.lines : []
                Rectangle {
                    readonly property real pad: Math.max(2, modelData.h * 0.12)
                    x: (modelData.x - pad) * flick.scale
                    y: (modelData.y - pad) * flick.scale
                    width: (modelData.w + 2 * pad) * flick.scale
                    height: (modelData.h + 2 * pad) * flick.scale
                    radius: modelData.h * 0.25 * flick.scale
                    color: Qt.alpha(palette.highlight, modelData.block === view.block ? 0.47 : 0.16)
                    border.color: Qt.alpha(palette.highlight, 0.67)
                    border.width: 1.5
                }
            }

            // A soft band of the accent color sweeping down the picture while a model reads it
            Rectangle {
                visible: tools.busy
                width: parent.width
                height: parent.height * 0.12
                gradient: Gradient {
                    GradientStop { position: 0; color: "transparent" }
                    GradientStop { position: 0.7; color: Qt.alpha(palette.highlight, 0.35) }
                    GradientStop { position: 1; color: "transparent" }
                }
                NumberAnimation on y {
                    running: tools.busy
                    loops: Animation.Infinite
                    from: -image.height * 0.12
                    to: image.height
                    duration: 1400
                    easing.type: Easing.InOutSine
                }
            }
        }

        function zoomAt(factor, px, py) {
            const old = scale
            const next = Math.max(fitScale / 4, Math.min(32, old * factor))
            // Keep the point under the pointer still
            const cx = (contentX + px - image.x) / old, cy = (contentY + py - image.y) / old
            zoom = next
            contentX = Math.max(0, Math.min(contentWidth - width, cx * next + image.x - px))
            contentY = Math.max(0, Math.min(contentHeight - height, cy * next + image.y - py))
        }

        WheelHandler {
            acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
            onWheel: event => flick.zoomAt(Math.pow(1.0015, event.angleDelta.y), point.position.x, point.position.y)
        }
        PinchHandler {
            target: null
            onScaleChanged: delta => flick.zoomAt(delta, centroid.position.x, centroid.position.y)
        }
        HoverHandler { cursorShape: view.mode === "subject" ? Qt.CrossCursor : view.mode === "text" ? Qt.PointingHandCursor : Qt.ArrowCursor }
        TapHandler {
            id: tap
            onDoubleTapped: if (view.mode === "") flick.zoom = flick.zoom ? 0 : 1
            onTapped: eventPoint => {
                const p = image.mapFromItem(tap.parent, eventPoint.position)
                view.clicked(p.x / flick.scale, p.y / flick.scale, tap.point.modifiers)
            }
        }
    }

    /// A click on the picture, in picture pixels
    function clicked(x, y, modifiers) {
        if (mode === "subject") {
            tools.pick(x, y, modifiers & Qt.ShiftModifier ? "add" : modifiers & Qt.ControlModifier ? "remove" : "new", reach.value)
        } else if (mode === "text") {
            const line = lines.find(l => x >= l.x && x < l.x + l.w && y >= l.y && y < l.y + l.h)
            block = line ? line.block : -1 // a click beside the text closes it
        }
    }

    // The clicked paragraph, to read and copy
    Pane {
        id: card
        parent: flick
        visible: view.mode === "text" && view.block >= 0
        readonly property var rect: {
            const boxes = view.lines.filter(l => l.block === view.block)
            if (!boxes.length)
                return Qt.rect(0, 0, 0, 0)
            const left = Math.min(...boxes.map(l => l.x)), top = Math.min(...boxes.map(l => l.y))
            const right = Math.max(...boxes.map(l => l.x + l.w)), bottom = Math.max(...boxes.map(l => l.y + l.h))
            return Qt.rect(left, top, right - left, bottom - top)
        }
        // the paragraph's corners in the visible part of the picture
        readonly property real textLeft: image.x + rect.x * flick.scale - flick.contentX
        readonly property real textTop: image.y + rect.y * flick.scale - flick.contentY
        readonly property real textBottom: textTop + rect.height * flick.scale
        x: Math.max(8, Math.min(flick.width - width - 8, textLeft))
        y: textBottom + height + 16 < flick.height ? textBottom + 8 : Math.max(8, textTop - height - 8) // below it, else above
        width: Math.min(440, flick.width - 16)
        padding: 10
        background: Rectangle {
            color: palette.window
            border.color: Qt.alpha(palette.windowText, 0.2)
            radius: 6
        }
        RowLayout {
            anchors.fill: parent
            spacing: 8
            Label {
                Layout.fillWidth: true
                text: view.block >= 0 ? view.blockText(view.block) : ""
                wrapMode: Text.Wrap
                maximumLineCount: 12
                elide: Text.ElideRight
            }
            Tool {
                Layout.alignment: Qt.AlignTop
                name: "edit-copy"
                label: "Copy"
                tip: "Copy this text (Ctrl+C)"
                onClicked: {
                    tools.copyText(view.blockText(view.block))
                    tools.status = "Text copied"
                }
            }
        }
    }

    component Tool: ToolButton {
        property string name
        property string tip
        property string label
        icon.name: name
        text: label
        display: label && view.width > 820 ? AbstractButton.TextBesideIcon : AbstractButton.IconOnly // a narrow window keeps the icons
        focusPolicy: Qt.NoFocus
        ToolTip.visible: hovered
        ToolTip.text: tip
    }
}
