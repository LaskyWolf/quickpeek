// SPDX-License-Identifier: GPL-3.0-or-later
//! Cover art and waveform of an audio file, read with ffmpeg on a worker thread.

use core::pin::Pin;
use cxx_qt::Threading;
use cxx_qt_lib::{QByteArray, QByteArrayBase64Options, QList, QString};
use std::process::Command;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qlist.h");
        type QList_f64 = cxx_qt_lib::QList<f64>;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, path)]
        #[qproperty(QString, cover)] // data: URL of the embedded picture, empty when there is none
        #[qproperty(QList_f64, peaks)] // loudness of each slice, 0..1; empty until ffmpeg is done
        type AudioFile = super::AudioFileRust;
    }

    impl cxx_qt::Threading for AudioFile {}
    impl cxx_qt::Initialize for AudioFile {}
}

use qobject::AudioFile;

const PEAKS: usize = 600;

/// "audio" or "video" for files MediaView plays.
pub fn kind(path: &str) -> Option<&'static str> {
    let mime = crate::helpers::mime_info(path).name;
    ["audio", "video"].into_iter().find(|kind| mime.starts_with(&format!("{kind}/")))
}

#[derive(Default)]
pub struct AudioFileRust {
    path: QString,
    cover: QString,
    peaks: QList<f64>,
}

fn ffmpeg(path: &str, args: &[&str]) -> Vec<u8> {
    Command::new("ffmpeg")
        .args(["-v", "error", "-i", path])
        .args(args)
        .arg("-")
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default() // no ffmpeg: no cover, no waveform
}

/// Loudest sample in each of `count` slices of mono s16le audio, scaled so the loudest is 1.
fn peaks(raw: &[u8], count: usize) -> Vec<f64> {
    let samples: Vec<i32> = raw.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs() as i32).collect();
    let step = (samples.len() / count).max(1);
    let peaks: Vec<i32> = samples.chunks(step).map(|c| *c.iter().max().unwrap()).collect();
    let loudest = peaks.iter().copied().max().unwrap_or(0).max(1) as f64;
    peaks.into_iter().map(|p| p as f64 / loudest).collect()
}

fn load(thread: cxx_qt::CxxQtThread<AudioFile>, path: String) {
    // Cover first: it takes a moment, the waveform decodes the whole track
    let png = ffmpeg(&path, &["-an", "-map", "0:v:0?", "-frames:v", "1", "-c:v", "png", "-f", "image2pipe"]);
    let wanted = path.clone();
    let _ = thread.queue(move |file| {
        if file.path().to_string() == wanted && !png.is_empty() {
            let base64 = QByteArray::from(png.as_slice()).to_base64(QByteArrayBase64Options::default());
            let url = format!("data:image/png;base64,{}", String::from_utf8_lossy(base64.as_slice()));
            file.set_cover(QString::from(url.as_str()));
        }
    });
    let raw = ffmpeg(&path, &["-ac", "1", "-ar", "8000", "-f", "s16le"]);
    let peaks = peaks(&raw, PEAKS);
    let _ = thread.queue(move |file| {
        if file.path().to_string() == path {
            file.set_peaks(QList::from(&peaks));
        }
    });
}

impl cxx_qt::Initialize for AudioFile {
    fn initialize(self: Pin<&mut Self>) {
        self.on_path_changed(|mut file| {
            file.as_mut().set_cover(QString::default());
            file.as_mut().set_peaks(QList::default());
            let (thread, path) = (file.qt_thread(), file.path().to_string());
            std::thread::spawn(move || load(thread, path));
        })
        .release();
    }
}

#[cfg(test)]
mod tests {
    use super::peaks;

    #[test]
    fn peaks_are_scaled_slice_maxima() {
        let raw: Vec<u8> = [100i16, -200, 50, 400, i16::MIN, 0].iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(peaks(&raw, 3), vec![200.0 / 32768.0, 400.0 / 32768.0, 1.0]);
        assert!(peaks(&[], 3).is_empty());
        assert_eq!(peaks(&[0, 0], 3), vec![0.0]); // silence doesn't divide by zero
    }
}
