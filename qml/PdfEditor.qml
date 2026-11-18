// SPDX-License-Identifier: GPL-3.0-or-later
// Draw on a PDF: rectangles, ellipses, arrows, a pen that follows tablet pressure, a highlighter and text.
// Saving writes the drawings into the file as real annotations (rust/pdfedit.rs), so other PDF apps can move or delete them.
// While it shows, it takes every key: the window's shortcuts (Space, arrows, Del…) sleep.
import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts
import QtQuick.Pdf
import QtQuick.Window
import com.quickpeek

FocusScope {
    id: editor
    required property PdfDocument document
    required property string path
    property int startPage: 0
    signal done()

    readonly property var colors: ["#e53935", "#1e88e5", "#43a047", "#fdd835", "#212121"]
    readonly property var widths: [["Thin", 1.5], ["Medium", 3], ["Thick", 6]] // points; pairs, as a map would sort its names
    readonly property var sizes: [["Small", 10], ["Normal", 14], ["Large", 20], ["Huge", 28]] // points
    readonly property var tools: [
        ["select", "edit-select", "Select and move (S) — double-click text to change it"],
        ["rect", "draw-rectangle", "Rectangle (R) — Shift for a square"],
        ["ellipse", "draw-ellipse", "Ellipse (O) — Shift for a circle"],
        ["arrow", "draw-arrow", "Arrow (A)"],
        ["pen", "draw-freehand", "Pen (P) — follows a tablet pen's pressure"],
        ["highlight", "draw-highlight", "Highlight text (H) — drag across the words"],
        ["text", "draw-text", "Text (T) — click to write, click a text to change it"],
    ]
    readonly property real gap: 12 // points around and between pages

    property string tool: "rect"
    property color color: colors[0]
    property real lineWidth: widths[1][1]
    property real textSize: sizes[1][1]
    property real zoom: 1 // pixels per point
    property bool fitting: true
    property var drawings: [] // oldest first, so later ones paint on top. Points are PDF points on the page as shown.
    property var history: [] // [["add" or "delete", [drawings]]], for Undo
    property var selected: []
    property var drawing: null // the one being drawn
    property var moving: null // where the last step of a move ended, in `area` pixels
    property rect band // the Select tool's rubber band, in `area` pixels
    property real pressure: -1 // how hard a tablet pen presses; -1 without one
    property string saved: "[]" // the drawings at the last save
    property int fileRevision: 0 // a save puts the drawings into the file: render its pages again
    property string status
    onStatusChanged: if (status) statusTimer.restart()
    property Item typing: null // the text box taking the keys
    readonly property int page: { flick.contentY; flick.height; zoom; return currentPage() }

    PdfAnnotator { id: writer }
    PdfSelection { id: words; document: editor.document; visible: false; enabled: false } // the highlighter snaps to words

    Component.onCompleted: {
        fit()
        goTo(startPage)
        forceActiveFocus()
    }

    // ---- pages

    function fit() {
        fitting = true
        setZoom(flick.width / (document.maxPageWidth + 2 * gap))
    }

    function setZoom(value) {
        const factor = Math.min(Math.max(value, 0.1), 8) / zoom
        // everything grows with the zoom, so the same spot stays in the middle
        const middle = flick.contentY + flick.height / 2
        zoom *= factor
        flick.contentY = Math.max(middle * factor - flick.height / 2, 0)
    }

    function zoomBy(factor) {
        fitting = false
        setZoom(zoom * factor)
    }

    function goTo(index) {
        if (index < 0 || index >= document.pageCount)
            return
        let top = gap / 2
        for (let i = 0; i < index; i++)
            top += document.pagePointSize(i).height + gap
        flick.contentY = Math.min(top * zoom, Math.max(flick.contentHeight - flick.height, 0))
    }

    function currentPage() {
        const middle = flick.contentY + flick.height / 2 - column.y
        let best = 0
        for (let i = 0; i < pages.count; i++) {
            const item = pages.itemAt(i)
            if (item && item.y <= middle)
                best = i
        }
        return best
    }

    /// The page under a point of `area`, and the point on it; null between pages.
    function pageAt(x, y) {
        for (let i = 0; i < pages.count; i++) {
            const item = pages.itemAt(i)
            const p = item.mapFromItem(area, x, y)
            if (p.x >= 0 && p.y >= 0 && p.x <= item.width && p.y <= item.height)
                return { page: i, x: p.x / zoom, y: p.y / zoom }
        }
        return null
    }

    /// A point of `area` on page `index`, kept on the page: outside it would be cut away.
    function pointOn(index, x, y) {
        const item = pages.itemAt(index)
        const p = item.mapFromItem(area, x, y)
        return [Math.min(Math.max(p.x, 0), item.width) / zoom, Math.min(Math.max(p.y, 0), item.height) / zoom]
    }

    // ---- drawings

    function changed() {
        ink.requestPaint()
    }

    function setTool(name) {
        finishTyping()
        tool = name
        selected = []
        changed()
    }

    /// [left, top, right, bottom] around a drawing, in points. `tight`: without the line's width.
    function bounds(d, tight) {
        if (d.kind === "text")
            return [d.item.px, d.item.py, d.item.px + d.item.width / zoom, d.item.py + d.item.height / zoom]
        const pad = tight || d.kind === "highlight" ? 0 : d.kind === "marker" ? 6 : d.kind === "arrow" ? d.width * 3 + 5 : d.width
        const xs = [], ys = []
        if (d.rect)
            xs.push(d.rect[0], d.rect[2]), ys.push(d.rect[1], d.rect[3])
        for (const p of d.points ?? [])
            xs.push(p[0]), ys.push(p[1])
        for (const b of d.boxes ?? [])
            xs.push(b[0], b[2]), ys.push(b[1], b[3])
        return [Math.min(...xs) - pad, Math.min(...ys) - pad, Math.max(...xs) + pad, Math.max(...ys) + pad]
    }

    function drawingAt(hit) {
        for (let i = drawings.length - 1; i >= 0; i--) {
            const d = drawings[i], b = bounds(d)
            if (d.page === hit.page && hit.x >= b[0] - 2 && hit.x <= b[2] + 2 && hit.y >= b[1] - 2 && hit.y <= b[3] + 2)
                return d
        }
        return null
    }

    function moveBy(d, dx, dy) {
        if (d.item) {
            d.item.px += dx
            d.item.py += dy
        }
        if (d.rect)
            d.rect = [d.rect[0] + dx, d.rect[1] + dy, d.rect[2] + dx, d.rect[3] + dy]
        if (d.points)
            d.points = d.points.map(p => [p[0] + dx, p[1] + dy])
        if (d.boxes)
            d.boxes = d.boxes.map(b => [b[0] + dx, b[1] + dy, b[2] + dx, b[3] + dy])
    }

    /// Puts a drawing on its page, or takes it off.
    function show(d, on) {
        if (on)
            drawings.push(d)
        else
            drawings.splice(drawings.indexOf(d), 1)
        if (d.item)
            d.item.visible = on
    }

    function undo() {
        const step = history.pop()
        if (!step)
            return
        for (const d of step[1])
            show(d, step[0] === "delete")
        selected = []
        changed()
    }

    function deleteSelected() {
        if (!selected.length)
            return
        for (const d of selected)
            show(d, false)
        history.push(["delete", selected])
        selected = []
        changed()
    }

    /// Removes an empty text box without a trace in Undo.
    function discard(d) {
        if (drawings.includes(d))
            drawings.splice(drawings.indexOf(d), 1)
        history = history.filter(step => !step[1].includes(d))
        selected = selected.filter(s => s !== d)
        d.item.destroy()
        changed()
    }

    /// Applies a new color, line width or text size to the selected drawings.
    function restyle(newColor, width, size) {
        for (const d of selected) {
            if (d.item) {
                if (newColor)
                    d.item.color = newColor
                if (size)
                    d.item.size = size
            } else {
                if (newColor)
                    d.color = newColor.toString()
                if (width && ["rect", "ellipse", "arrow", "pen"].includes(d.kind))
                    d.width = width
            }
        }
        changed()
    }

    function addText(hit) {
        // the first line sits on the click
        const item = textBox.createObject(pages.itemAt(hit.page), { px: hit.x - 2, py: hit.y - textSize * 0.75, size: textSize, color: color })
        const d = { kind: "text", page: hit.page, item }
        item.drawing = d
        drawings.push(d)
        history.push(["add", [d]])
        startTyping(item)
    }

    function startTyping(item) {
        finishTyping()
        typing = item
        item.forceActiveFocus()
    }

    /// Ends the text box being typed in. False if there was none.
    function finishTyping() {
        const item = typing
        if (!item)
            return false
        typing = null
        item.focus = false // else the editor would hand the keys straight back to it
        item.deselect()
        editor.forceActiveFocus()
        if (!item.text.trim()) // an empty box is a click that wrote nothing
            discard(item.drawing)
        return true
    }

    function pageHasText(index) {
        words.page = index
        words.selectAll()
        return words.text.trim() !== ""
    }

    /// The line boxes covering the words from the one nearest `from` to the one nearest `to`, like a text selection.
    function wordBoxes(index, from, to) {
        words.page = index
        words.from = Qt.point(from[0], from[1])
        words.to = Qt.point(to[0], to[1])
        const boxes = []
        for (const polygon of words.geometry) {
            // ponytail: a QPolygonF reaches JavaScript opaque; its text form "QPolygonF(QPointF(41,52), …)" is
            // the only way in. A C++ helper returning rectangles is the upgrade if Qt changes that text.
            const numbers = String(polygon).replace(/QPolygonF|QPointF/g, "").match(/-?\d+(\.\d+)?(e-?\d+)?/g)?.map(Number) ?? []
            const xs = numbers.filter((_, i) => i % 2 === 0), ys = numbers.filter((_, i) => i % 2 === 1)
            if (xs.length)
                boxes.push([Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)])
        }
        return boxes
    }

    function press(mouse) {
        if (typing) {
            const b = bounds(typing.drawing), hit = pageAt(mouse.x, mouse.y)
            if (hit?.page === typing.drawing.page && hit.x >= b[0] && hit.x <= b[2] && hit.y >= b[1] && hit.y <= b[3]) {
                mouse.accepted = false // a click in the text being typed places the cursor
                return
            }
            finishTyping() // a click elsewhere ends typing; the next click starts a new text
            return
        }
        const hit = pageAt(mouse.x, mouse.y)
        if (tool === "select") {
            const d = hit && drawingAt(hit)
            if (!d) {
                selected = []
                band = Qt.rect(mouse.x, mouse.y, 0, 0)
            } else if (!selected.includes(d)) {
                selected = mouse.modifiers & Qt.ShiftModifier ? selected.concat([d]) : [d]
            }
            if (d)
                moving = Qt.point(mouse.x, mouse.y)
            changed()
            return
        }
        if (!hit)
            return
        if (tool === "text") {
            const d = drawingAt(hit)
            if (d?.kind === "text") { // change an existing text: the click places the cursor
                const p = d.item.mapFromItem(area, mouse.x, mouse.y)
                startTyping(d.item)
                d.item.cursorPosition = d.item.positionAt(p.x, p.y)
                return
            }
            addText(hit)
            return
        }
        const start = [hit.x, hit.y]
        let kind = tool
        if (kind === "highlight" && !pageHasText(hit.page))
            kind = "marker" // a scanned page has no words: a wide see-through stroke instead
        drawing = { kind, page: hit.page, color: color.toString(), width: lineWidth, start }
        if (kind === "rect" || kind === "ellipse")
            drawing.rect = [hit.x, hit.y, hit.x, hit.y]
        else if (kind === "highlight")
            drawing.boxes = []
        else
            drawing.points = [start]
        if (kind === "pen" && pressure >= 0)
            drawing.pressures = [pressure]
        drawings.push(drawing)
    }

    function drag(mouse) {
        if (moving) {
            const dx = (mouse.x - moving.x) / zoom, dy = (mouse.y - moving.y) / zoom
            for (const d of selected)
                moveBy(d, dx, dy)
            moving = Qt.point(mouse.x, mouse.y)
        } else if (tool === "select" && !drawing) {
            band = Qt.rect(band.x, band.y, mouse.x - band.x, mouse.y - band.y)
        }
        const d = drawing
        if (!d) {
            changed()
            return
        }
        const p = pointOn(d.page, mouse.x, mouse.y)
        if (d.kind === "rect" || d.kind === "ellipse") {
            let w = p[0] - d.start[0], h = p[1] - d.start[1]
            if (mouse.modifiers & Qt.ShiftModifier) { // a square, a circle
                const side = Math.max(Math.abs(w), Math.abs(h))
                w = Math.sign(w || 1) * side
                h = Math.sign(h || 1) * side
            }
            const x = d.start[0], y = d.start[1]
            d.rect = [Math.min(x, x + w), Math.min(y, y + h), Math.max(x, x + w), Math.max(y, y + h)]
        } else if (d.kind === "arrow") {
            d.points = [d.start, p]
        } else if (d.kind === "highlight") {
            d.boxes = wordBoxes(d.page, d.start, p)
        } else {
            d.points.push(p)
            if (d.pressures)
                d.pressures.push(pressure >= 0 ? pressure : 0.5)
        }
        changed()
    }

    function release() {
        const d = drawing
        drawing = null
        moving = null
        if (band.width || band.height) {
            const r = Qt.rect(Math.min(band.x, band.x + band.width), Math.min(band.y, band.y + band.height),
                              Math.abs(band.width), Math.abs(band.height))
            selected = drawings.filter(d => {
                const b = bounds(d), origin = pages.itemAt(d.page).mapToItem(area, 0, 0)
                return origin.x + b[0] * zoom < r.x + r.width && origin.x + b[2] * zoom > r.x
                    && origin.y + b[1] * zoom < r.y + r.height && origin.y + b[3] * zoom > r.y
            })
            band = Qt.rect(0, 0, 0, 0)
        }
        if (d) {
            const b = bounds(d, true)
            const empty = d.kind === "highlight" ? !d.boxes.length : b[2] - b[0] + b[3] - b[1] < 3
            if (empty)
                drawings.splice(drawings.indexOf(d), 1) // a click without dragging draws nothing
            else
                history.push(["add", [d]])
        }
        changed()
    }

    // ---- painting

    function arrowHead(start, end, width) {
        const back = Math.atan2(start[1] - end[1], start[0] - end[0]), length = width * 3 + 5
        return [28, -28].map(turn => {
            const angle = back + turn * Math.PI / 180
            return [end[0] + length * Math.cos(angle), end[1] + length * Math.sin(angle)]
        })
    }

    /// Line width factor: a light touch draws at 0.15×, a full press at 2× the chosen width.
    function pressureWidth(p) {
        return 0.15 + 1.85 * p
    }

    function paint(ctx) {
        ctx.reset()
        for (const d of drawings) {
            const item = pages.itemAt(d.page)
            const origin = item.mapToItem(ink, 0, 0)
            if (origin.y > ink.height || origin.y + item.height < 0)
                continue
            ctx.save()
            ctx.translate(origin.x, origin.y)
            ctx.scale(zoom, zoom)
            if (d.kind !== "text") // a text box draws itself
                draw(ctx, d)
            if (selected.includes(d)) {
                const b = bounds(d)
                ctx.globalAlpha = 1
                ctx.lineWidth = 1 / zoom
                ctx.strokeStyle = palette.highlight
                ctx.strokeRect(b[0], b[1], b[2] - b[0], b[3] - b[1])
            }
            ctx.restore()
        }
        if (band.width || band.height) {
            const corner = area.mapToItem(ink, band.x, band.y)
            ctx.lineWidth = 1
            ctx.strokeStyle = palette.highlight
            ctx.fillStyle = Qt.alpha(palette.highlight, 0.15)
            ctx.fillRect(corner.x, corner.y, band.width, band.height)
            ctx.strokeRect(corner.x, corner.y, band.width, band.height)
        }
    }

    function draw(ctx, d) {
        ctx.strokeStyle = d.color
        ctx.fillStyle = d.color
        ctx.lineWidth = d.width
        ctx.lineCap = "round"
        ctx.lineJoin = "round"
        const r = d.rect, p = d.points
        if (d.kind === "rect") {
            ctx.strokeRect(r[0], r[1], r[2] - r[0], r[3] - r[1])
        } else if (d.kind === "ellipse") {
            ctx.beginPath()
            ctx.ellipse(r[0], r[1], r[2] - r[0], r[3] - r[1])
            ctx.stroke()
        } else if (d.kind === "highlight") {
            ctx.globalAlpha = 0.4
            for (const b of d.boxes)
                ctx.fillRect(b[0], b[1], b[2] - b[0], b[3] - b[1])
        } else if (d.kind === "arrow") {
            ctx.beginPath()
            ctx.moveTo(p[0][0], p[0][1])
            ctx.lineTo(p[1][0], p[1][1])
            for (const side of arrowHead(p[0], p[1], d.width)) {
                ctx.moveTo(p[1][0], p[1][1])
                ctx.lineTo(side[0], side[1])
            }
            ctx.stroke()
        } else if (d.pressures) { // each piece as wide as the pen was pressed there
            for (let i = 1; i < p.length; i++) {
                ctx.lineWidth = d.width * pressureWidth((d.pressures[i - 1] + d.pressures[i]) / 2)
                ctx.beginPath()
                ctx.moveTo(p[i - 1][0], p[i - 1][1])
                ctx.lineTo(p[i][0], p[i][1])
                ctx.stroke()
            }
        } else { // pen, or the highlighter on a page without text
            if (d.kind === "marker") {
                ctx.globalAlpha = 0.4
                ctx.lineWidth = 12
                ctx.lineCap = "butt"
            }
            ctx.beginPath()
            ctx.moveTo(p[0][0], p[0][1])
            for (const point of p.slice(1))
                ctx.lineTo(point[0], point[1])
            ctx.stroke()
        }
    }

    // ---- saving

    /// Every drawing as plain data for rust/pdfedit.rs: page, kind, geometry in points, style.
    function annotations() {
        return drawings.filter(d => d.kind !== "text" || d.item.text.trim()).map(d => {
            const c = Qt.color(d.item ? d.item.color : d.color)
            const out = { page: d.page, kind: d.kind, color: [c.r, c.g, c.b], width: d.width ?? 0 }
            if (d.item)
                Object.assign(out, { rect: bounds(d), text: d.item.text, size: d.item.size })
            else
                Object.assign(out, { rect: d.rect, points: d.points, pressures: d.pressures, boxes: d.boxes })
            return out
        })
    }

    /// Writes the drawings into `target`: the open PDF itself, or a new file.
    function save(target) {
        finishTyping()
        const list = annotations()
        if (!list.length) {
            status = "Nothing drawn yet"
            return false
        }
        const error = writer.save(path, JSON.stringify(list), target)
        if (error) {
            status = "Save failed: " + error
            return false
        }
        if (target === path) { // the drawings are now part of the PDF: its pages show them
            for (const d of drawings)
                d.item?.destroy()
            drawings = []
            history = []
            selected = []
            saved = "[]"
            fileRevision++
        } else {
            saved = state()
        }
        status = "Saved " + target.split("/").pop()
        changed()
        return true
    }

    /// The drawings, to tell whether anything changed since the last save. Without a text box's size:
    /// that follows the zoom's font rounding.
    function state() {
        return JSON.stringify(annotations().map(a => a.text === undefined ? a : Object.assign(a, { rect: a.rect.slice(0, 2) })))
    }

    /// True when it's fine to leave: nothing new since the last save. Otherwise it asks, and `then` runs
    /// once you chose Save or Discard.
    function leave(then) {
        finishTyping()
        if (state() === saved)
            return true
        ask.then = then
        ask.open()
        return false
    }

    function localPath(url) {
        return decodeURIComponent(url.toString().replace(/^file:\/\//, ""))
    }

    Keys.onShortcutOverride: event => event.accepted = true
    Keys.onPressed: event => {
        const plain = !(event.modifiers & (Qt.ControlModifier | Qt.AltModifier | Qt.MetaModifier))
        const letter = plain && tools.find(t => t[2].includes(`(${event.text.toUpperCase()})`))
        if (event.matches(StandardKey.SaveAs) || event.key === Qt.Key_S && event.modifiers === (Qt.ControlModifier | Qt.ShiftModifier))
            saveAs.open()
        else if (event.matches(StandardKey.Save))
            save(path)
        else if (event.matches(StandardKey.Undo))
            undo()
        else if (event.key === Qt.Key_Delete || event.key === Qt.Key_Backspace)
            deleteSelected()
        else if (event.matches(StandardKey.ZoomIn) || event.key === Qt.Key_Equal && event.modifiers & Qt.ControlModifier)
            zoomBy(1.25)
        else if (event.matches(StandardKey.ZoomOut))
            zoomBy(1 / 1.25)
        else if (event.key === Qt.Key_PageUp)
            goTo(page - 1)
        else if (event.key === Qt.Key_PageDown)
            goTo(page + 1)
        else if (event.key === Qt.Key_Up || event.key === Qt.Key_Down)
            flick.contentY = Math.min(Math.max(flick.contentY + (event.key === Qt.Key_Up ? -60 : 60), 0), Math.max(flick.contentHeight - flick.height, 0))
        else if (event.key === Qt.Key_Escape)
            editor.done()
        else if (letter)
            setTool(letter[0])
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.rightMargin: 44 // room for the floating side-panel button
            Layout.margins: 4
            spacing: 2

            // the drawing controls wrap onto a second line in a narrow window
            Flow {
                Layout.fillWidth: true
                Layout.preferredWidth: 0
                Layout.preferredHeight: implicitHeight
                spacing: 2

                Repeater {
                    model: editor.tools
                    Tool {
                        required property var modelData
                        name: modelData[1]
                        tip: modelData[2]
                        checked: editor.tool === modelData[0]
                        onClicked: editor.setTool(modelData[0])
                    }
                }
                Separator {}
                Repeater {
                    model: editor.colors
                    ToolButton {
                        required property string modelData
                        checked: Qt.colorEqual(editor.color, modelData)
                        focusPolicy: Qt.NoFocus
                        ToolTip.visible: hovered
                        ToolTip.text: "Color"
                        onClicked: {
                            editor.color = modelData
                            editor.restyle(Qt.color(modelData))
                        }
                        contentItem: Item {
                            implicitWidth: 16
                            implicitHeight: 16
                            Rectangle {
                                anchors.centerIn: parent
                                width: 14
                                height: 14
                                radius: 7
                                color: modelData
                                border.color: Qt.alpha("gray", 0.67) // the dark swatch stays visible on dark themes
                            }
                        }
                    }
                }
                Separator {}
                // Line width for shapes, text size for text, neither for the highlighter
                readonly property var kinds: editor.selected.length ? editor.selected.map(d => d.kind) : [editor.tool]
                ComboBox {
                    model: editor.widths.map(w => w[0])
                    currentIndex: 1
                    visible: parent.kinds.some(k => ["rect", "ellipse", "arrow", "pen"].includes(k))
                    focusPolicy: Qt.NoFocus
                    ToolTip.visible: hovered
                    ToolTip.text: "Line width"
                    onActivated: {
                        editor.lineWidth = editor.widths[currentIndex][1]
                        editor.restyle(null, editor.lineWidth)
                    }
                }
                ComboBox {
                    model: editor.sizes.map(s => s[0])
                    currentIndex: 1
                    visible: parent.kinds.includes("text")
                    focusPolicy: Qt.NoFocus
                    ToolTip.visible: hovered
                    ToolTip.text: "Text size"
                    onActivated: {
                        editor.textSize = editor.sizes[currentIndex][1]
                        editor.restyle(null, null, editor.textSize)
                    }
                }
                Tool { name: "edit-undo"; tip: "Undo (Ctrl+Z)"; onClicked: editor.undo() }
                Tool { name: "edit-delete"; tip: "Delete selected (Del)"; enabled: editor.selected.length > 0; onClicked: editor.deleteSelected() }
            }

            Label {
                text: `Page ${editor.page + 1} of ${editor.document.pageCount}`
                enabled: false // dimmed: information, not a control
            }
            Tool { name: "zoom-out"; tip: "Zoom out (Ctrl+-)"; onClicked: editor.zoomBy(1 / 1.25) }
            Tool { name: "zoom-in"; tip: "Zoom in (Ctrl++)"; onClicked: editor.zoomBy(1.25) }
            Tool { name: "zoom-fit-width"; tip: "Fit width"; checked: editor.fitting; onClicked: editor.fit() }
            Tool { name: "dialog-ok"; label: "Done"; tip: "Back to reading (Esc)"; onClicked: editor.done() }
            Tool { name: "document-save-as"; tip: "Save as a new PDF… (Ctrl+Shift+S)"; onClicked: saveAs.open() }
            Tool { name: "document-save"; label: "Save"; tip: "Write the drawings into this PDF (Ctrl+S)"; onClicked: editor.save(editor.path) }
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true

            Rectangle { anchors.fill: parent; color: palette.window }

            Flickable {
                id: flick
                anchors.fill: parent
                contentWidth: Math.max(width, column.width + 2 * editor.gap * editor.zoom)
                contentHeight: column.height + 2 * editor.gap * editor.zoom
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: ScrollBar {}
                ScrollBar.horizontal: ScrollBar {}
                onWidthChanged: if (editor.fitting) editor.fit()
                onContentYChanged: ink.requestPaint()
                onContentXChanged: ink.requestPaint()

                Column {
                    id: column
                    x: (flick.contentWidth - width) / 2
                    y: editor.gap * editor.zoom
                    spacing: editor.gap * editor.zoom

                    Repeater {
                        id: pages
                        model: editor.document.pageCount
                        Rectangle {
                            id: sheet
                            required property int index
                            readonly property size points: editor.document.pagePointSize(index)
                            // rendered only near the screen
                            readonly property bool near: column.y + y + height > flick.contentY - flick.height
                                                         && column.y + y < flick.contentY + 2 * flick.height
                            width: points.width * editor.zoom
                            height: points.height * editor.zoom
                            color: "white" // paper until rendered
                            border.color: Qt.alpha(palette.shadow, 0.27)

                            Image {
                                anchors.fill: parent
                                asynchronous: true
                                cache: false
                                // pixels per point, capped at 16 Mpx a page so deep zoom can't eat gigabytes
                                readonly property real density: Math.min(editor.zoom * Screen.devicePixelRatio,
                                                                         Math.sqrt(16e6 / (sheet.points.width * sheet.points.height)))
                                sourceSize: Qt.size(Math.round(sheet.points.width * density), Math.round(sheet.points.height * density))
                                // with the annotations, which QtPdf's own page images leave out
                                source: sheet.near ? `image://pdfpage/${sheet.index}/${encodeURIComponent(editor.path)}/${editor.fileRevision}` : ""
                            }
                        }
                    }
                }

                MouseArea {
                    id: area
                    width: flick.contentWidth
                    height: flick.contentHeight
                    preventStealing: true // a drag draws instead of scrolling
                    cursorShape: editor.tool === "select" ? Qt.ArrowCursor
                               : ["text", "highlight"].includes(editor.tool) ? Qt.IBeamCursor : Qt.CrossCursor
                    onPressed: mouse => editor.press(mouse)
                    onPositionChanged: mouse => editor.drag(mouse)
                    onReleased: editor.release()
                    onDoubleClicked: mouse => { // Select tool: double-click a text to change it
                        const hit = editor.pageAt(mouse.x, mouse.y), d = hit && editor.drawingAt(hit)
                        if (d?.kind === "text")
                            editor.startTyping(d.item)
                    }
                    // ponytail: a tablet pen's pressure, then Qt hands the same touch to onPressed as a mouse click;
                    // not tried with a real tablet
                    PointHandler {
                        acceptedDevices: PointerDevice.Stylus | PointerDevice.Airbrush
                        onPointChanged: editor.pressure = active ? point.pressure : -1
                        onActiveChanged: if (!active) editor.pressure = -1
                    }
                }
            }

            Canvas {
                id: ink
                anchors.fill: flick
                onPaint: editor.paint(getContext("2d"))
                Connections {
                    target: editor
                    function onZoomChanged() { ink.requestPaint() }
                }
            }

            // Ctrl+wheel zooms; without Ctrl the wheel scrolls
            WheelHandler {
                target: null
                acceptedModifiers: Qt.ControlModifier
                onWheel: event => editor.zoomBy(event.angleDelta.y > 0 ? 1.25 : 1 / 1.25)
            }

            // What the last save did, for a few seconds over the top-right corner
            Pane {
                anchors.top: parent.top
                anchors.right: parent.right
                anchors.margins: 12
                anchors.rightMargin: 24 // clear of the scrollbar
                padding: 6
                visible: editor.status !== ""
                Label { text: editor.status }
                Timer { id: statusTimer; interval: 4000; onTriggered: editor.status = "" }
            }
        }
    }

    Component {
        id: textBox
        TextEdit {
            property var drawing
            property real px // position on the page, in points
            property real py
            property real size // points
            readonly property real margin: 2 // points, as rust/pdfedit.rs writes the text
            x: px * editor.zoom
            y: py * editor.zoom
            padding: margin * editor.zoom
            font.family: "Helvetica" // the same shapes as the PDF's built-in Helvetica the saved text uses
            font.pixelSize: size * editor.zoom
            textFormat: TextEdit.PlainText
            selectByMouse: true
            activeFocusOnPress: false // typing starts from the editor's clicks
            // every key while typing: letters, Space, Del and Ctrl+Z belong to the text
            Keys.onShortcutOverride: event => event.accepted = true
            Keys.onEscapePressed: editor.finishTyping() // the first Esc ends the text, the next leaves editing
            onWidthChanged: ink.requestPaint() // the selection frame follows
            onHeightChanged: ink.requestPaint()
            onXChanged: ink.requestPaint()
            onYChanged: ink.requestPaint()

            Rectangle { // a frame while typing shows where the box is
                anchors.fill: parent
                color: "transparent"
                border.color: palette.highlight
                visible: editor.typing === parent
            }
        }
    }

    Dialog {
        id: ask
        property var then
        title: "Unsaved drawings"
        anchors.centerIn: parent
        modal: true
        standardButtons: Dialog.Save | Dialog.Discard | Dialog.Cancel
        Label { text: "Save your drawings into the PDF before leaving?" }
        onAccepted: if (editor.save(editor.path)) Qt.callLater(then)
        onDiscarded: {
            editor.saved = editor.state()
            close()
            Qt.callLater(then)
        }
        onClosed: editor.forceActiveFocus()
    }

    FileDialog {
        id: saveAs
        fileMode: FileDialog.SaveFile
        nameFilters: ["PDF documents (*.pdf)"]
        defaultSuffix: "pdf"
        currentFolder: "file://" + editor.path.slice(0, editor.path.lastIndexOf("/"))
        selectedFile: "file://" + editor.path.replace(/\.pdf$/i, "") + " (edited).pdf"
        onAccepted: {
            editor.save(editor.localPath(selectedFile))
            editor.forceActiveFocus()
        }
        onRejected: editor.forceActiveFocus()
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

    component Separator: Item {
        width: 9
        height: 32
        Rectangle {
            anchors.centerIn: parent
            width: 1
            height: 20
            color: Qt.alpha(palette.windowText, 0.2)
        }
    }
}
