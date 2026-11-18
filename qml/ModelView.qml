// SPDX-License-Identifier: GPL-3.0-or-later
// A 3D model to turn around, or to fly through with W A S D. Wireframe, spin, floor grid, animations.
// While flying every key goes to the flight, so Escape lands instead of closing the window.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick3D
import QtQuick3D.Helpers
import QtQuick3D.AssetUtils
import com.quickpeek

ColumnLayout {
    id: view
    property alias path: backend.path
    property bool flying: false
    property int lift: 0 // while flying: +1 E held (up), -1 Q held (down), along the world's vertical axis
    property real baseSpeed: 1 // fly speed that suits the model's size: see fit()
    property real speedFactor: 1 // the wheel while flying: up faster, down slower
    // ponytail: these stay as you left them while you flip between models, not after another kind of file
    property bool wireframe: false
    property bool spin: false
    property bool grid: true
    property real gridStep: 1
    property var timelines: []
    property var animations: [] // their names
    property bool playing: true
    spacing: 0

    SystemPalette { id: colors }

    Model {
        id: backend
        onPathChanged: { view.flying = false; view.timelines = []; view.animations = [] }
    }

    // Loading blocks for a moment: draw the empty view first, so the window never sits on the old preview
    property url pending: backend.model
    onPendingChanged: { loader.source = ""; waitFrame.enabled = true }
    Connections {
        id: waitFrame
        target: view.Window.window
        enabled: false
        function onFrameSwapped() { waitFrame.enabled = false; loader.source = view.pending }
    }

    onFlyingChanged: {
        lift = 0
        if (flying) { // start flying from wherever the orbit camera is looking
            flyCam.position = camera.scenePosition
            flyCam.rotation = camera.sceneRotation
            flyKeys.forceActiveFocus()
        } else { // land: orbit around the point the fly camera was looking at, from the same spot
            const distance = camera.z
            orbit.position = flyCam.position.plus(flyCam.forward.times(distance))
            orbit.rotation = flyCam.rotation
            camera.position = Qt.vector3d(0, 0, distance)
        }
    }

    Shortcut { sequence: "Home"; onActivated: view.recenter() }
    Shortcut { sequence: "P"; enabled: view.timelines.length > 0; onActivated: view.play(!view.playing) }

    RowLayout {
        Layout.fillWidth: true
        Layout.rightMargin: 52 // room for the floating side-panel button
        Layout.margins: 4
        Layout.leftMargin: 12
        spacing: 2

        Label {
            Layout.fillWidth: true
            text: backend.stats
            elide: Text.ElideRight
            enabled: false // dimmed: information, not a control
        }
        ComboBox {
            visible: view.animations.length > 1
            model: view.animations
            focusPolicy: Qt.NoFocus
            ToolTip.visible: hovered
            ToolTip.text: "Animation"
            onActivated: index => { animationIndex = index; view.play(true) }
            property int animationIndex: 0
            id: animationBox
        }
        Tool {
            visible: view.timelines.length > 0
            name: view.playing ? "media-playback-pause" : "media-playback-start"
            label: view.playing ? "Pause" : "Play"
            tip: (view.playing ? "Pause" : "Play") + " the animation (P)"
            onClicked: view.play(!view.playing)
        }
        Tool { name: "zoom-fit-best"; label: "Recenter"; tip: "Back to the starting view (Home)"; onClicked: view.recenter() }
        Tool { name: "transform-move"; label: "Fly"; tip: "Walk inside the model"; checked: view.flying; onClicked: view.flying = !view.flying }
        Tool { name: "draw-polyline"; label: "Wireframe"; tip: "Show the triangles"; checked: view.wireframe; onClicked: view.wireframe = !view.wireframe }
        Tool { name: "object-rotate-right"; label: "Spin"; tip: "Turn the model slowly"; checked: view.spin; onClicked: view.spin = !view.spin }
        Tool { name: "view-grid"; label: "Grid"; tip: "Floor grid"; checked: view.grid; onClicked: view.grid = !view.grid }
    }

    Item {
        id: stage
        Layout.fillWidth: true
        Layout.fillHeight: true
        clip: true

        View3D {
            id: view3d
            anchors.fill: parent
            environment: SceneEnvironment {
                backgroundMode: SceneEnvironment.Color
                clearColor: colors.window
                antialiasingMode: SceneEnvironment.MSAA
                antialiasingQuality: SceneEnvironment.High
                debugSettings: DebugSettings { wireframeEnabled: view.wireframe }
                InfiniteGrid { visible: view.grid; gridInterval: view.gridStep }
            }
            camera: view.flying ? flyCam : camera
            Node {
                id: orbit // the controller rotates this node; the camera rides on it
                PerspectiveCamera {
                    id: camera
                    DirectionalLight { // headlight
                        visible: !view.flying
                        brightness: 1.0
                        ambientColor: Qt.rgba(0.35, 0.35, 0.35, 1)
                    }
                }
            }
            PerspectiveCamera {
                id: flyCam
                clipNear: camera.clipNear
                clipFar: camera.clipFar
                DirectionalLight { visible: view.flying; brightness: 1.0; ambientColor: Qt.rgba(0.35, 0.35, 0.35, 1) }
            }
            DirectionalLight { eulerRotation: Qt.vector3d(-50, 40, 0); brightness: 0.5 }
            RuntimeLoader {
                id: loader
                visible: scene.status !== Loader3D.Ready
                opacity: status === RuntimeLoader.Success ? 1 : 0
                Behavior on opacity { NumberAnimation { duration: 200; easing.type: Easing.OutCubic } }
                onBoundsChanged: view.fit()
                onStatusChanged: if (status === RuntimeLoader.Error) console.warn("quickpeek: 3D model:", errorString)
            }
            Loader3D {
                id: scene
                source: backend.scene
                onLoaded: view.collectAnimations()
            }
        }

        // A .blend's own picture while Blender exports it
        Image {
            anchors.fill: parent
            anchors.margins: 24
            visible: loader.status !== RuntimeLoader.Success
            source: backend.thumbnail
            fillMode: Image.PreserveAspectFit
        }

        FrameAnimation {
            running: view.spin && !view.flying
            onTriggered: orbit.eulerRotation.y += frameTime * 30 // degrees per second
        }
        // fit() sizes the clip planes to the model; the controller's own clipping would reset them on every zoom
        OrbitCameraController { anchors.fill: parent; origin: orbit; camera: camera; enabled: !view.flying; automaticClipping: false }
        // Touchpad pinch only zooms. The controller's own pinch also pans by however much the fingers drift,
        // which moves the view a little on every pinch; this one is on top, so it takes the gesture first.
        Item {
            anchors.fill: parent
            PinchHandler {
                target: null
                enabled: !view.flying
                onScaleChanged: delta => camera.z /= delta
            }
        }
        // R / F would move along the camera's own up, which tilts with the view: off, Q / E lift instead.
        // Its keys come from flyKeys, so it never takes the focus itself.
        WasdController {
            id: wasd
            anchors.fill: parent
            controlledObject: flyCam
            enabled: view.flying
            keysEnabled: false
            // shiftSpeed replaces speed while Shift is held (it is not a multiplier): same speed, the wheel sets it
            upSpeed: 0; downSpeed: 0; speed: view.baseSpeed * view.speedFactor; shiftSpeed: speed
            onActiveFocusChanged: if (activeFocus) flyKeys.forceActiveFocus() // its click handler grabs the focus
        }
        FrameAnimation {
            running: view.flying && view.lift !== 0
            onTriggered: flyCam.y += view.lift * 500 * wasd.speed * frameTime // the controller's pace: 5 units × speed per 1/100 s
        }

        // While flying, every key is the flight's: the window's shortcuts wait
        Item {
            id: flyKeys
            Keys.onShortcutOverride: event => event.accepted = view.flying
            Keys.onPressed: event => {
                if (!view.flying || event.isAutoRepeat)
                    return
                if (event.key === Qt.Key_Escape)
                    view.flying = false
                else if (event.key === Qt.Key_E || event.key === Qt.Key_Q)
                    view.lift = event.key === Qt.Key_E ? 1 : -1
                else if (![Qt.Key_Left, Qt.Key_Right, Qt.Key_Up, Qt.Key_Down].includes(event.key)) // W A S D move
                    wasd.handleKeyPress(event)
            }
            Keys.onReleased: event => {
                if (event.isAutoRepeat)
                    return
                if (event.key === Qt.Key_E || event.key === Qt.Key_Q)
                    view.lift = 0
                else
                    wasd.handleKeyRelease(event)
            }
        }

        // The wheel while flying, like Minecraft's spectator mode: each notch is 20% faster or slower
        WheelHandler {
            enabled: view.flying
            target: null
            onWheel: event => view.setSpeed(Math.min(Math.max(view.speedFactor * Math.pow(1.2, event.angleDelta.y / 120), 0.05), 50))
        }
        TapHandler { enabled: view.flying; acceptedButtons: Qt.MiddleButton; onTapped: view.setSpeed(1) }

        Rectangle {
            id: speedNote
            anchors { top: parent.top; horizontalCenter: parent.horizontalCenter; topMargin: 12 }
            width: speedText.implicitWidth + 24
            height: speedText.implicitHeight + 12
            radius: height / 2
            color: Qt.alpha(colors.window, 0.88)
            border.color: Qt.alpha(colors.windowText, 0.3)
            opacity: 0
            Behavior on opacity { NumberAnimation { duration: 200 } }
            Text {
                id: speedText
                anchors.centerIn: parent
                color: colors.windowText
                text: "Speed ×" + (view.speedFactor < 1 ? view.speedFactor.toFixed(2) : view.speedFactor.toFixed(1))
            }
            Timer { id: speedFade; interval: 900; onTriggered: speedNote.opacity = 0 }
        }
        Text {
            anchors { bottom: parent.bottom; horizontalCenter: parent.horizontalCenter; bottomMargin: 8 }
            color: colors.windowText
            opacity: 0.7
            text: view.flying
                  ? "W A S D: move  ·  E / Q: up / down  ·  Wheel: speed  ·  Middle click: normal speed  ·  drag: look  ·  Esc: stop flying"
                  : "Drag: rotate  ·  Ctrl+drag: pan  ·  Wheel: zoom"
        }
        Text {
            anchors.centerIn: parent
            color: colors.windowText
            text: loader.status === RuntimeLoader.Error ? loader.errorString : backend.note
        }
    }

    function setSpeed(factor) { // also the middle button: back to 1
        speedFactor = factor
        speedNote.opacity = 1
        speedFade.restart()
    }

    function recenter() { // land if flying, then frame the whole model again from the starting angle
        flying = false
        fit()
    }

    // balsam makes one Timeline per animation and starts them all; keep only the first running
    function collectAnimations() {
        const found = []
        const walk = node => {
            for (const child of node.data) {
                if (child.endFrame !== undefined && child.animations !== undefined)
                    found.push(child)
                else if (child.data !== undefined)
                    walk(child)
            }
        }
        walk(scene.item)
        timelines = found
        animations = found.map((t, i) => t.objectName || "Animation " + (i + 1))
        animationBox.animationIndex = 0
        play(true)
    }

    function play(on) {
        playing = on
        timelines.forEach((t, i) => {
            const mine = i === animationBox.animationIndex
            t.enabled = mine
            t.animations[0].running = mine
            t.animations[0].paused = mine && !on
        })
    }

    // Center the model and back the camera off far enough to see all of it
    function fit() {
        const min = loader.bounds.minimum, max = loader.bounds.maximum
        const radius = max.minus(min).length() / 2 || 1
        orbit.position = min.plus(max).times(0.5)
        orbit.eulerRotation = Qt.vector3d(-20, 30, 0)
        camera.position = Qt.vector3d(0, 0, radius * 2.6)
        camera.clipNear = radius / 100
        camera.clipFar = radius * 100
        baseSpeed = radius / 150 // fly speed relative to model size
        gridStep = Math.pow(10, Math.floor(Math.log10(radius))) // 1 m lines for a 1-5 m model
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
