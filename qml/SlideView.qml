// SPDX-License-Identifier: GPL-3.0-or-later
// A presentation one slide at a time: ↑ / ↓, Page Up / Down or the wheel move between slides.
// Slides that come in with a Morph transition play it: each object glides to its new place, size and turn
// while its look cross-fades; objects that come or go fade. The pictures come from rust/office.rs.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Pdf

ColumnLayout {
    id: view
    property alias source: doc.source
    property string deck // JSON, see deck_json in rust/office.rs
    readonly property var info: deck ? JSON.parse(deck) : null
    // the PDF doesn't match the slides we read: no Morph, just pages
    readonly property var slides: info && info.slides.length === doc.pageCount ? info.slides : null
    property int index: 0
    property real t: 0 // how far the Morph has played
    property var draws: [] // the objects it moves: {o: object, from, to: box, fade}
    property int from: 0 // the slides it goes between
    property int to: 0
    readonly property size page: doc.status === PdfDocument.Ready ? doc.pagePointSize(0) : Qt.size(4, 3)
    spacing: 0

    PdfDocument { id: doc }

    NumberAnimation { id: morph; target: view; property: "t"; from: 0; to: 1; easing.type: Easing.InOutCubic }

    /// ↑ / ↓ from the window: always slides here.
    function stepVertical(delta) {
        go(delta)
        return true
    }

    /// Move `delta` slides; a Morph plays when the slide coming in (or, going back, the one leaving) has one.
    function go(delta) {
        const target = Math.min(Math.max(index + delta, 0), doc.pageCount - 1)
        if (target === index)
            return
        morph.stop()
        const start = index
        index = target
        if (Math.abs(target - start) === 1 && canMorph(start, target)) {
            from = start
            to = target
            draws = plan(start, target)
            morph.duration = slides[Math.max(start, target)].duration || 2000
            morph.restart()
        }
    }

    function canMorph(a, b) {
        return slides !== null && slides[Math.max(a, b)].morph && slides[a].bg !== "" && slides[b].bg !== ""
    }

    // Objects pair up by name. Leaving objects first, then the arriving slide's objects in its own stacking order.
    function plan(a, b) {
        const firstByName = list => {
            const out = {}
            list.forEach((o, n) => { if (!(o.name in out)) out[o.name] = n })
            return out
        }
        const mine = firstByName(slides[a].objects), theirs = firstByName(slides[b].objects)
        const out = []
        for (const o of slides[a].objects)
            if (!(o.name in theirs))
                out.push({ o: o, from: o.box, to: o.box, fade: "out" })
        for (const o of slides[b].objects) {
            if (o.name in mine) {
                // both looks travel together; each is solid for half the way so nothing dims midway
                const old = slides[a].objects[mine[o.name]]
                out.push({ o: old, from: old.box, to: o.box, fade: "outHalf" })
                out.push({ o: o, from: old.box, to: o.box, fade: "inHalf" })
            } else {
                out.push({ o: o, from: o.box, to: o.box, fade: "in" })
            }
        }
        return out.filter(d => d.o.sprite !== "")
    }

    function fade(kind, t) {
        return { out: 1 - t, in: t, outHalf: Math.min(1, 2 * (1 - t)), inHalf: Math.min(1, 2 * t) }[kind]
    }

    // Moves a picture drawn at box `own` to where the object is at `t` between `from` and `to`:
    // around its center, stretched to the new size and turned to the new angle. Boxes: [x, y, w, h, degrees] in EMU.
    function motion(own, from, to, t) {
        const s = slideArea.width / info.width // pixels per EMU
        const now = from.map((v, i) => v + (to[i] - v) * t)
        const rad = Math.PI / 180
        const c1 = Math.cos(-own[4] * rad), s1 = Math.sin(-own[4] * rad)
        const c2 = Math.cos(now[4] * rad), s2 = Math.sin(now[4] * rad)
        const sx = own[2] ? now[2] / own[2] : 1, sy = own[3] ? now[3] / own[3] : 1
        // rotate(now) · scale · rotate(−own)
        const a = c2 * sx * c1 - s2 * sy * s1, b = -c2 * sx * s1 - s2 * sy * c1
        const c = s2 * sx * c1 + c2 * sy * s1, d = -s2 * sx * s1 + c2 * sy * c1
        const ox = (own[0] + own[2] / 2) * s, oy = (own[1] + own[3] / 2) * s
        const nx = (now[0] + now[2] / 2) * s, ny = (now[1] + now[3] / 2) * s
        return Qt.matrix4x4(a, b, 0, nx - a * ox - b * oy, c, d, 0, ny - c * ox - d * oy, 0, 0, 1, 0, 0, 0, 0, 1)
    }

    Shortcut { sequences: [StandardKey.MoveToPreviousPage]; onActivated: view.go(-1) }
    Shortcut { sequences: [StandardKey.MoveToNextPage]; onActivated: view.go(1) }
    Shortcut { sequence: "Home"; onActivated: view.go(-view.index) }
    Shortcut { sequence: "End"; onActivated: view.go(doc.pageCount - 1 - view.index) }

    RowLayout {
        Layout.fillWidth: true
        Layout.rightMargin: 44 // room for the floating side-panel button
        Layout.margins: 4
        Layout.leftMargin: 12
        Layout.minimumHeight: 32
        Label {
            text: doc.pageCount > 0 ? `Slide ${view.index + 1} of ${doc.pageCount}` + (view.slides?.[view.index].morph ? "  ·  Morph" : "") : ""
            enabled: false // dimmed: information, not a control
        }
    }

    Item {
        id: area
        Layout.fillWidth: true
        Layout.fillHeight: true
        clip: true

        // As big as fits, centered
        Item {
            id: slideArea
            readonly property real scale: Math.min((area.width - 24) / Math.max(view.page.width, 1), (area.height - 24) / Math.max(view.page.height, 1))
            width: view.page.width * scale
            height: view.page.height * scale
            anchors.centerIn: parent

            PdfPageImage {
                anchors.fill: parent
                document: doc
                currentFrame: view.index
                sourceSize.width: width * Screen.devicePixelRatio
                visible: !morph.running
            }

            Item {
                anchors.fill: parent
                visible: morph.running
                Image { anchors.fill: parent; source: morph.running ? view.info.dir + "/" + view.slides[view.from].bg : "" }
                Image { anchors.fill: parent; source: morph.running ? view.info.dir + "/" + view.slides[view.to].bg : ""; opacity: view.t }
                Repeater {
                    model: morph.running ? view.draws : []
                    Item {
                        required property var modelData
                        width: slideArea.width
                        height: slideArea.height
                        opacity: view.fade(modelData.fade, view.t)
                        transform: Matrix4x4 { matrix: view.motion(modelData.o.box, modelData.from, modelData.to, view.t) }
                        Image {
                            source: view.info.dir + "/" + modelData.o.sprite
                            x: modelData.o.crop[0] * parent.width
                            y: modelData.o.crop[1] * parent.height
                            width: modelData.o.crop[2] * parent.width
                            height: modelData.o.crop[3] * parent.height
                            smooth: true
                        }
                    }
                }
            }
        }

        WheelHandler {
            property real turned: 0
            onWheel: event => {
                turned += event.angleDelta.y
                while (Math.abs(turned) >= 120) { // one notch, one slide; touchpads send many small steps
                    view.go(turned > 0 ? -1 : 1)
                    turned -= turned > 0 ? 120 : -120
                }
            }
        }
    }
}
