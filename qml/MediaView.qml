// SPDX-License-Identifier: GPL-3.0-or-later
// Video, or audio with cover, tags and waveform, over a player bar.
// Keys: K play/pause, J / L back / forward 5 s, M mute. Space closes the window, so a click pauses.
import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtMultimedia
import com.quickpeek

ColumnLayout {
    id: view
    property url source
    property bool audio
    property string icon // shown for audio without cover art
    readonly property string failure: player.error !== MediaPlayer.NoError ? "The file is damaged or in a format that can't be played." : ""
    readonly property bool playing: player.playbackState === MediaPlayer.PlayingState
    spacing: 0

    function toggle() { playing ? player.pause() : player.play() }
    function clock(ms) { // 1:05 or 1:02:05
        const s = Math.floor(Math.max(ms, 0) / 1000), h = Math.floor(s / 3600)
        const mm = Math.floor(s % 3600 / 60), ss = String(s % 60).padStart(2, "0")
        return h ? h + ":" + String(mm).padStart(2, "0") + ":" + ss : mm + ":" + ss
    }

    SystemPalette { id: colors }

    // Saved in ~/.config/quickpeek/quickpeek.conf, so the volume carries over
    Settings {
        id: settings
        location: StandardPaths.writableLocation(StandardPaths.ConfigLocation) + "/quickpeek/quickpeek.conf"
        property real volume: 1.0
    }

    MediaPlayer {
        id: player
        source: view.source
        autoPlay: true
        videoOutput: video
        audioOutput: AudioOutput { id: output; volume: settings.volume }
        // Qt 6.12's FFmpeg backend freezes when a very short file reaches its end.
        // Looping never reaches the end; pausing when the position jumps back plays it once.
        loops: MediaPlayer.Infinite
        property int lastPos: 0
        onPositionChanged: () => {
            if (position < lastPos && duration - lastPos < 1500) // wrapped at the end, not a seek
                pause()
            lastPos = position
        }
        onSourceChanged: { lastPos = 0; playbackRate = 1 } // each file starts at normal speed
    }

    Shortcut { sequence: "K"; onActivated: view.toggle() }
    Shortcut { sequence: "M"; onActivated: output.muted = !output.muted }
    Shortcut { sequence: "J"; onActivated: player.setPosition(Math.max(player.position - 5000, 0)) }
    Shortcut { sequence: "L"; onActivated: player.setPosition(Math.min(player.position + 5000, player.duration)) }

    Item {
        Layout.fillWidth: true
        Layout.fillHeight: true

        VideoOutput {
            id: video
            anchors.fill: parent
            visible: !view.audio
        }
        TapHandler { onTapped: view.toggle() } // the waveform's MouseArea keeps its clicks for seeking

        ColumnLayout {
            visible: view.audio
            anchors.fill: parent
            anchors.margins: 12
            spacing: 6

            Item { Layout.fillHeight: true }
            Image {
                Layout.alignment: Qt.AlignHCenter
                Layout.maximumWidth: 300
                Layout.maximumHeight: 300
                visible: audioFile.cover !== ""
                source: audioFile.cover
                sourceSize: Qt.size(300, 300)
                fillMode: Image.PreserveAspectFit
            }
            ToolButton {
                Layout.alignment: Qt.AlignHCenter
                visible: audioFile.cover === ""
                icon.name: view.icon
                icon.width: 128
                icon.height: 128
                flat: true
                hoverEnabled: false
                focusPolicy: Qt.NoFocus
                onClicked: view.toggle() // it covers part of the click-to-pause area
            }
            Label {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                font.pointSize: Qt.application.font.pointSize * 1.6
                font.bold: true
                text: player.metaData.stringValue(MediaMetaData.Title)
                    || decodeURIComponent(view.source.toString().split("/").pop().replace(/\.[^.]*$/, ""))
            }
            Label {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                text: [player.metaData.stringValue(MediaMetaData.ContributingArtist)
                           || player.metaData.stringValue(MediaMetaData.AlbumArtist),
                       player.metaData.stringValue(MediaMetaData.AlbumTitle),
                       player.duration ? view.clock(player.duration) : ""].filter(p => p).join("  ·  ")
            }
            Item { Layout.fillHeight: true }

            // The whole track's loudness as bars; the played part in the accent color. Click or drag to seek.
            Item {
                id: wave
                Layout.fillWidth: true
                Layout.preferredHeight: 120
                // The player reports its position only every few tenths of a second: in between,
                // the playhead moves on by the clock, so it glides instead of jumping.
                property real known: player.position
                property real stamp: Date.now()
                property real shown
                onKnownChanged: { stamp = Date.now(); shown = known }
                FrameAnimation {
                    running: view.playing && view.audio
                    onTriggered: wave.shown = Math.min(wave.known + (Date.now() - wave.stamp) * player.playbackRate, player.duration)
                }

                AudioFile { id: audioFile; path: view.audio ? decodeURIComponent(view.source.toString().replace(/^file:\/\//, "")) : "" }
                Bars { anchors.fill: parent; color: Qt.alpha(colors.text, 0.3) }
                Item { // the played part, cut mid-bar: no 3 px jumps
                    width: player.duration ? wave.shown / player.duration * wave.width : 0
                    height: parent.height
                    clip: true
                    Bars { width: wave.width; height: wave.height; color: colors.highlight }
                }
                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    function seek(x) { player.setPosition(Math.round(Math.max(0, Math.min(1, x / width)) * player.duration)) }
                    onPressed: mouse => seek(mouse.x)
                    onPositionChanged: mouse => seek(mouse.x)
                }
            }
        }
    }

    RowLayout {
        Layout.fillWidth: true
        Layout.leftMargin: 8
        Layout.rightMargin: 8
        spacing: 8

        ToolButton {
            icon.name: view.playing ? "media-playback-pause" : "media-playback-start"
            focusPolicy: Qt.NoFocus // Space and the arrows keep flipping files
            ToolTip.visible: hovered
            ToolTip.text: "Play / pause (K)"
            onClicked: view.toggle()
        }
        Label {
            text: view.clock(player.position)
            font.features: { "tnum": 1 } // equal-width digits: the time doesn't wobble while playing
            enabled: false // dimmed: secondary to the timeline
        }
        Slider {
            id: seek
            Layout.fillWidth: true
            focusPolicy: Qt.NoFocus
            to: Math.max(player.duration, 1)
            onMoved: player.setPosition(value)
            Binding on value { when: !seek.pressed; value: player.position } // don't yank the handle while dragging
        }
        Label {
            text: view.clock(player.duration)
            font.features: { "tnum": 1 }
            enabled: false
        }
        ToolButton {
            visible: !view.audio
            text: player.playbackRate + "×"
            focusPolicy: Qt.NoFocus
            ToolTip.visible: hovered
            ToolTip.text: "Playback speed"
            onClicked: speedMenu.popup()
            Menu {
                id: speedMenu
                Instantiator {
                    model: [0.5, 0.75, 1, 1.25, 1.5, 2]
                    MenuItem { text: modelData + "×"; onTriggered: player.playbackRate = modelData }
                    onObjectAdded: (index, item) => speedMenu.insertItem(index, item)
                    onObjectRemoved: (index, item) => speedMenu.removeItem(item)
                }
            }
        }
        ToolButton {
            // sliders feel linear to the ear only on a logarithmic scale
            readonly property real level: output.muted ? 0 : volume.value
            icon.name: level === 0 ? "audio-volume-muted" : level < 0.34 ? "audio-volume-low"
                     : level < 0.67 ? "audio-volume-medium" : "audio-volume-high"
            focusPolicy: Qt.NoFocus
            ToolTip.visible: hovered
            ToolTip.text: output.muted ? "Unmute (M)" : "Mute (M)"
            onClicked: output.muted = !output.muted
        }
        Slider {
            id: volume
            Layout.preferredWidth: 90
            focusPolicy: Qt.NoFocus
            // QtAudio::convertVolume between the linear and logarithmic scales; QML doesn't have it
            value: 1 - Math.exp(-settings.volume * Math.log(100))
            onMoved: {
                settings.volume = value > 0.999 ? 1 : -Math.log(1 - value) / Math.log(100)
                output.muted = false
            }
            ToolTip.visible: hovered
            ToolTip.text: "Volume"
        }
    }

    component Bars: Canvas {
        property color color
        property var peaks: audioFile.peaks
        onColorChanged: requestPaint()
        onPeaksChanged: requestPaint()
        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()
        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()
            if (!peaks.length)
                return
            ctx.fillStyle = color
            const bars = Math.max(Math.floor(width / 3), 1), middle = height / 2 // 2 px bar + 1 px gap
            for (let i = 0; i < bars; i++) {
                const half = Math.max(peaks[Math.floor(i * peaks.length / bars)] * middle, 1)
                ctx.fillRect(i * 3, middle - half, 2, half * 2)
            }
        }
    }
}
