// Copyright (C) 2022 The Qt Company Ltd.
// SPDX-License-Identifier: LicenseRef-Qt-Commercial OR LGPL-3.0-only OR GPL-2.0-only OR GPL-3.0-only
// QtPdf's PdfMultiPageView (Qt 6.12), copied as Qt's docs suggest, with QuickPeek's changes:
// - pages come from image://pdfpage (cpp/helpers.cpp), which draws annotations; QtPdf's PdfPageImage leaves them out
// - no horizontal scrollbar unless the pages are wider than the view
// - clicks and keys never take the focus from the window: dragging the mouse selects text, the wheel scrolls

pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Pdf
import QtQuick.Shapes

Item {
    required property PdfDocument document
    property int revision // bump it after the file changed on disk: the pages render again
    readonly property string path: decodeURIComponent(String(document.source).replace(/^file:\/\//, ""))

    property string selectedText

    function selectAll() {
        const currentItem = tableView.itemAtCell(tableView.cellAtPos(root.width / 2, root.height / 2))
        const pdfSelection = currentItem?.selection as PdfSelection
        pdfSelection?.selectAll()
    }

    function copySelectionToClipboard() {
        const currentItem = tableView.itemAtCell(tableView.cellAtPos(root.width / 2, root.height / 2))
        const pdfSelection = currentItem?.selection as PdfSelection
        pdfSelection?.copyToClipboard()
    }

    // --------------------------------
    // page navigation

    property alias currentPage: pageNavigator.currentPage

    property alias backEnabled: pageNavigator.backAvailable

    property alias forwardEnabled: pageNavigator.forwardAvailable

    function back() { pageNavigator.back() }

    function forward() { pageNavigator.forward() }

    function goToPage(page) {
        if (page === pageNavigator.currentPage)
            return
        goToLocation(page, Qt.point(-1, -1), 0)
    }

    function goToLocation(page, location, zoom) {
        if (tableView.rows === 0) {
            // save this request for later
            tableView.pendingRow = page
            tableView.pendingLocation = location
            tableView.pendingZoom = zoom
            return
        }
        if (zoom > 0) {
            pageNavigator.jumping = true // don't call pageNavigator.update() because we will jump() instead
            root.renderScale = zoom
            pageNavigator.jumping = false
        }
        pageNavigator.jump(page, location, zoom) // actually jump
    }

    property int currentPageRenderingStatus: Image.Null

    // --------------------------------
    // page scaling

    property real renderScale: 1

    property real pageRotation: 0

    function resetScale() { root.renderScale = 1 }

    function scaleToWidth(width, height) {
        root.renderScale = width / (tableView.rot90 ? tableView.firstPagePointSize.height : tableView.firstPagePointSize.width)
    }

    function scaleToPage(width, height) {
        const windowAspect = width / height
        const pageAspect = tableView.firstPagePointSize.width / tableView.firstPagePointSize.height
        if (tableView.rot90) {
            if (windowAspect > pageAspect) {
                root.renderScale = height / tableView.firstPagePointSize.width
            } else {
                root.renderScale = width / tableView.firstPagePointSize.height
            }
        } else {
            if (windowAspect > pageAspect) {
                root.renderScale = height / tableView.firstPagePointSize.height
            } else {
                root.renderScale = width / tableView.firstPagePointSize.width
            }
        }
    }

    // --------------------------------
    // text search

    property alias searchModel: searchModel

    property alias searchString: searchModel.searchString

    function searchBack() { --searchModel.currentResult }

    function searchForward() { ++searchModel.currentResult }

    id: root
    PdfStyle { id: style }
    TableView {
        id: tableView
        property real minScale: 0.1
        property real maxScale: 10
        property point jumpLocationMargin: Qt.point(10, 10)  // px away from viewport edges
        anchors.fill: parent
        anchors.leftMargin: 2
        pointerNavigationEnabled: false
        acceptedButtons: Qt.NoButton
        model: root.document ? root.document.pageCount : 0
        rowSpacing: 6
        property real rotationNorm: Math.round((360 + (root.pageRotation % 360)) % 360)
        property bool rot90: rotationNorm == 90 || rotationNorm == 270
        onRot90Changed: forceLayout()
        onHeightChanged: forceLayout()
        onWidthChanged: forceLayout()
        property size firstPagePointSize: root.document?.status === PdfDocument.Ready ? root.document.pagePointSize(0) : Qt.size(1, 1)
        property real pageHolderWidth: Math.max(tableView.width, ((rot90 ? root.document?.maxPageHeight : root.document?.maxPageWidth) ?? 0) * root.renderScale)
        columnWidthProvider: function(col) { return root.document ? pageHolderWidth : 0 }
        rowHeightProvider: function(row) { return (rot90 ? root.document.pagePointSize(row).width : root.document.pagePointSize(row).height) * root.renderScale }

        // delayed-jump feature in case the user called goToPage() or goToLocation() too early
        property int pendingRow: -1
        property point pendingLocation
        property real pendingZoom: -1
        onRowsChanged: {
            if (rows > 0 && tableView.pendingRow >= 0) {
                root.goToLocation(tableView.pendingRow, tableView.pendingLocation, tableView.pendingZoom)
                tableView.pendingRow = -1
                tableView.pendingLocation = Qt.point(-1, -1)
                tableView.pendingZoom = -1
            }
        }

        delegate: Rectangle {
            id: pageHolder
            required property int index
            color: "transparent"
            property alias selection: selection
            Rectangle {
                id: paper
                width: image.width
                height: image.height
                rotation: root.pageRotation
                anchors.centerIn: pinch.active ? undefined : parent
                property size pagePointSize: root.document.pagePointSize(pageHolder.index)
                property real pageScale: image.paintedWidth / pagePointSize.width
                Image {
                    id: image
                    asynchronous: true
                    cache: false
                    source: `image://pdfpage/${pageHolder.index}/${encodeURIComponent(root.path)}/${root.revision}`
                    // pixels per point, capped at 16 Mpx a page so deep zoom can't eat gigabytes
                    readonly property real density: Math.min(root.renderScale * Screen.devicePixelRatio,
                                                             Math.sqrt(16e6 / (paper.pagePointSize.width * paper.pagePointSize.height)))
                    sourceSize: Qt.size(Math.round(paper.pagePointSize.width * density), Math.round(paper.pagePointSize.height * density))
                    fillMode: Image.PreserveAspectFit
                    width: paper.pagePointSize.width * root.renderScale
                    height: paper.pagePointSize.height * root.renderScale
                    property real renderScale: root.renderScale
                    onRenderScaleChanged: {
                        paper.scale = 1
                        searchHighlights.update()
                    }
                    onStatusChanged: {
                        if (pageHolder.index === pageNavigator.currentPage)
                            root.currentPageRenderingStatus = status
                    }
                }
                Shape {
                    anchors.fill: parent
                    visible: image.status === Image.Ready
                    onVisibleChanged: searchHighlights.update()
                    ShapePath {
                        strokeWidth: -1
                        fillColor: style.pageSearchResultsColor
                        scale: Qt.size(paper.pageScale, paper.pageScale)
                        PathMultiline {
                            id: searchHighlights
                            function update() {
                                // paths could be a binding, but we need to be able to "kick" it sometimes
                                paths = searchModel.boundingPolygonsOnPage(pageHolder.index)
                            }
                        }
                    }
                    Connections {
                        target: searchModel
                        // whenever the highlights on the _current_ page change, they actually need to change on _all_ pages
                        // (usually because the search string has changed)
                        function onCurrentPageBoundingPolygonsChanged() { searchHighlights.update() }
                    }
                    ShapePath {
                        strokeWidth: -1
                        fillColor: style.selectionColor
                        scale: Qt.size(paper.pageScale, paper.pageScale)
                        PathMultiline {
                            paths: selection.geometry
                        }
                    }
                }
                Shape {
                    anchors.fill: parent
                    visible: image.status === Image.Ready && searchModel.currentPage === pageHolder.index
                    ShapePath {
                        strokeWidth: style.currentSearchResultStrokeWidth
                        strokeColor: style.currentSearchResultStrokeColor
                        fillColor: "transparent"
                        scale: Qt.size(paper.pageScale, paper.pageScale)
                        PathMultiline {
                            paths: searchModel.currentResultBoundingPolygons
                        }
                    }
                }
                PinchHandler {
                    id: pinch
                    minimumScale: tableView.minScale / root.renderScale
                    maximumScale: Math.max(1, tableView.maxScale / root.renderScale)
                    minimumRotation: root.pageRotation
                    maximumRotation: root.pageRotation
                    onActiveChanged:
                        if (active) {
                            paper.z = 10
                        } else {
                            paper.z = 0
                            const centroidInPoints = Qt.point(pinch.centroid.position.x / root.renderScale,
                                                            pinch.centroid.position.y / root.renderScale)
                            const centroidInFlickable = tableView.mapFromItem(paper, pinch.centroid.position.x, pinch.centroid.position.y)
                            const newSourceWidth = image.sourceSize.width * paper.scale
                            const ratio = newSourceWidth / image.sourceSize.width
                            if (ratio > 1.1 || ratio < 0.9) {
                                const centroidOnPage = Qt.point(centroidInPoints.x * root.renderScale * ratio, centroidInPoints.y * root.renderScale * ratio)
                                paper.scale = 1
                                pinch.persistentScale = 1
                                paper.x = 0
                                paper.y = 0
                                root.renderScale *= ratio
                                tableView.forceLayout()
                                if (tableView.rotationNorm == 0) {
                                    tableView.contentX = pageHolder.x + tableView.originX + centroidOnPage.x - centroidInFlickable.x
                                    tableView.contentY = pageHolder.y + tableView.originY + centroidOnPage.y - centroidInFlickable.y
                                } else if (tableView.rotationNorm == 90) {
                                    tableView.contentX = pageHolder.x + tableView.originX + image.height - centroidOnPage.y - centroidInFlickable.x
                                    tableView.contentY = pageHolder.y + tableView.originY + centroidOnPage.x - centroidInFlickable.y
                                } else if (tableView.rotationNorm == 180) {
                                    tableView.contentX = pageHolder.x + tableView.originX + image.width - centroidOnPage.x - centroidInFlickable.x
                                    tableView.contentY = pageHolder.y + tableView.originY + image.height - centroidOnPage.y - centroidInFlickable.y
                                } else if (tableView.rotationNorm == 270) {
                                    tableView.contentX = pageHolder.x + tableView.originX + centroidOnPage.y - centroidInFlickable.x
                                    tableView.contentY = pageHolder.y + tableView.originY + image.width - centroidOnPage.x - centroidInFlickable.y
                                }
                                tableView.returnToBounds()
                            }
                        }
                    grabPermissions: PointerHandler.CanTakeOverFromAnything
                }
                DragHandler {
                    id: textSelectionDrag
                    acceptedDevices: PointerDevice.Mouse | PointerDevice.Stylus
                    target: null
                }
                TapHandler {
                    id: mouseClickHandler
                    acceptedDevices: PointerDevice.Mouse | PointerDevice.Stylus
                }
                TapHandler {
                    id: touchTapHandler
                    acceptedDevices: PointerDevice.TouchScreen
                    onTapped: {
                        selection.clear()
                        selection.forceActiveFocus()
                    }
                }
                Repeater {
                    model: PdfLinkModel {
                        id: linkModel
                        document: root.document
                        page: pageHolder.index
                    }
                    delegate: PdfLinkDelegate {
                        x: rectangle.x * paper.pageScale
                        y: rectangle.y * paper.pageScale
                        width: rectangle.width * paper.pageScale
                        height: rectangle.height * paper.pageScale
                        visible: image.status === Image.Ready
                        onTapped:
                            (link) => {
                                if (link.page >= 0)
                                    root.goToLocation(link.page, link.location, link.zoom)
                                else
                                    Qt.openUrlExternally(url)
                            }
                    }
                }
                PdfSelection {
                    id: selection
                    anchors.fill: parent
                    document: root.document
                    page: pageHolder.index
                    renderScale: image.renderScale
                    from: textSelectionDrag.centroid.pressPosition
                    to: textSelectionDrag.centroid.position
                    hold: !textSelectionDrag.active && !mouseClickHandler.pressed
                    onTextChanged: root.selectedText = text
                    focus: true
                }
            }
        }
        ScrollBar.vertical: ScrollBar {
            id: vscroll
            property bool moved: false
            onPositionChanged: moved = true
            onPressedChanged: if (pressed) {
                // When the user starts scrolling, push the location where we came from so the user can go "back" there
                const cell = tableView.cellAtPos(root.width / 2, root.height / 2)
                const currentItem = tableView.itemAtCell(cell)
                const currentLocation = currentItem
                                      ? Qt.point((tableView.contentX - currentItem.x + tableView.jumpLocationMargin.x) / root.renderScale,
                                                 (tableView.contentY - currentItem.y + tableView.jumpLocationMargin.y) / root.renderScale)
                                      : Qt.point(0, 0) // maybe the delegate wasn't loaded yet
                pageNavigator.jump(cell.y, currentLocation, root.renderScale)
            }
            onActiveChanged: if (!active ) {
                // When the scrollbar stops moving, tell navstack where we are, so as to update currentPage etc.
                const cell = tableView.cellAtPos(root.width / 2, root.height / 2)
                const currentItem = tableView.itemAtCell(cell)
                const currentLocation = currentItem
                                      ? Qt.point((tableView.contentX - currentItem.x + tableView.jumpLocationMargin.x) / root.renderScale,
                                                 (tableView.contentY - currentItem.y + tableView.jumpLocationMargin.y) / root.renderScale)
                                      : Qt.point(0, 0) // maybe the delegate wasn't loaded yet
                pageNavigator.update(cell.y, currentLocation, root.renderScale)
            }
        }
        ScrollBar.horizontal: ScrollBar { }
    }
    onRenderScaleChanged: {
        // if pageNavigator.jumped changes the scale, don't turn around and update the stack again;
        // and don't force layout either, because positionViewAtCell() will do that
        if (pageNavigator.jumping)
            return
        // page size changed: TableView needs to redo layout to avoid overlapping delegates or gaps between them
        tableView.forceLayout()
        const cell = tableView.cellAtPos(root.width / 2, root.height / 2)
        const currentItem = tableView.itemAtCell(cell)
        if (currentItem) {
            const currentLocation = Qt.point((tableView.contentX - currentItem.x + tableView.jumpLocationMargin.x) / root.renderScale,
                                             (tableView.contentY - currentItem.y + tableView.jumpLocationMargin.y) / root.renderScale)
            pageNavigator.update(cell.y, currentLocation, renderScale)
        }
    }
    PdfPageNavigator {
        id: pageNavigator
        property bool jumping: false
        property int previousPage: 0
        onJumped: function(current) {
            jumping = true
            if (current.zoom > 0)
                root.renderScale = current.zoom
            const pageSize = root.document.pagePointSize(current.page)
            if (current.location.y < 0) {
                // invalid to indicate that a specific location was not needed,
                // so attempt to position the new page just as the current page is
                const previousPageDelegate = tableView.itemAtCell(0, previousPage)
                const currentYOffset = previousPageDelegate
                                     ? tableView.contentY - previousPageDelegate.y
                                     : 0
                tableView.positionViewAtRow(current.page, Qt.AlignTop, currentYOffset)
            } else if (current.rectangles.length > 0) {
                // jump to a search result and position the covered area within the viewport
                pageSize.width *= root.renderScale
                pageSize.height *= root.renderScale
                const rectPts = current.rectangles[0]
                const rectPx = Qt.rect(rectPts.x * root.renderScale - tableView.jumpLocationMargin.x,
                                       rectPts.y * root.renderScale - tableView.jumpLocationMargin.y,
                                       rectPts.width * root.renderScale + tableView.jumpLocationMargin.x * 2,
                                       rectPts.height * root.renderScale + tableView.jumpLocationMargin.y * 2)
                tableView.positionViewAtCell(0, current.page, TableView.Contain, Qt.point(0, 0), rectPx)
            } else {
                // jump to a page and position the given location relative to the top-left corner of the viewport
                pageSize.width *= root.renderScale
                pageSize.height *= root.renderScale
                const rectPx = Qt.rect(current.location.x * root.renderScale - tableView.jumpLocationMargin.x,
                                       current.location.y * root.renderScale - tableView.jumpLocationMargin.y,
                                       tableView.jumpLocationMargin.x * 2, tableView.jumpLocationMargin.y * 2)
                tableView.positionViewAtCell(0, current.page, TableView.AlignLeft | TableView.AlignTop, Qt.point(0, 0), rectPx)
            }
            jumping = false
            previousPage = current.page
        }

        property url documentSource: root.document.source
        onDocumentSourceChanged: {
            pageNavigator.clear()
            root.resetScale()
            tableView.contentX = 0
            tableView.contentY = 0
        }
    }
    PdfSearchModel {
        id: searchModel
        document: root.document === undefined ? null : root.document
        onCurrentResultChanged: pageNavigator.jump(currentResultLink)
    }
}
