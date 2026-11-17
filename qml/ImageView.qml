// SPDX-License-Identifier: GPL-3.0-or-later
// A picture, fitted to the window. Wheel zooms around the pointer, dragging pans, double-click fits again.
import QtQuick

Flickable {
    id: view
    property alias source: image.source
    property real zoom: 0 // 0 = fit to the window
    readonly property real fitScale: image.implicitWidth > 0
        ? Math.min(1, width / image.implicitWidth, height / image.implicitHeight) : 1 // small pictures stay 1:1
    readonly property real scale: zoom || fitScale

    contentWidth: Math.max(width, image.implicitWidth * scale)
    contentHeight: Math.max(height, image.implicitHeight * scale)
    boundsBehavior: Flickable.StopAtBounds
    interactive: zoom > 0

    onSourceChanged: zoom = 0

    AnimatedImage { // also plays GIF, WebP and APNG animations
        id: image
        asynchronous: true
        autoTransform: true // photos taken sideways stand upright
        smooth: true
        mipmap: true // no shimmer when a big photo is shrunk
        width: implicitWidth * view.scale
        height: implicitHeight * view.scale
        x: (view.contentWidth - width) / 2
        y: (view.contentHeight - height) / 2
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
        onWheel: event => view.zoomAt(Math.pow(1.0015, event.angleDelta.y), point.position.x, point.position.y)
    }
    PinchHandler {
        target: null
        onScaleChanged: delta => view.zoomAt(delta, centroid.position.x, centroid.position.y)
    }
    TapHandler {
        onDoubleTapped: view.zoom = view.zoom ? 0 : 1
    }
}
