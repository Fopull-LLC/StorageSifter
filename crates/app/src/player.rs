//! In-app audio playback.
//!
//! Pure-Rust decoding (Symphonia, via rodio) and output through cpal/ALSA, which
//! is present on every Linux desktop — so this adds no heavyweight system
//! dependency and the binary stays self-contained. Video is intentionally not
//! handled here (it needs a much heavier media stack); see the media view.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Whether a file is in-app playable audio, externally-played video, or neither.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Audio,
    Video,
}

/// Classify a file by extension.
pub fn media_kind(name: &str) -> Option<MediaKind> {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "mp3" | "flac" | "wav" | "ogg" | "oga" | "m4a" | "aac" | "aiff" | "aif" | "alac"
        | "mka" => Some(MediaKind::Audio),
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "wmv" | "flv" | "m4v" | "mpeg" | "mpg" | "ts"
        | "m2ts" | "3gp" | "ogv" => Some(MediaKind::Video),
        _ => None,
    }
}

/// A playing (or paused) audio track. Dropping it stops playback and releases
/// the output device.
pub struct AudioPlayer {
    // The stream + handle must outlive the sink for sound to keep playing.
    _stream: OutputStream,
    _handle: OutputStreamHandle,
    sink: Sink,
    duration: Option<Duration>,
}

impl AudioPlayer {
    /// Open `path` and begin playing. Returns a human-readable error on failure
    /// (no output device, unsupported codec, unreadable file, …).
    pub fn open(path: &Path) -> Result<AudioPlayer, String> {
        let (stream, handle) =
            OutputStream::try_default().map_err(|e| format!("No audio output device ({e})"))?;
        let sink = Sink::try_new(&handle).map_err(|e| format!("Audio device error ({e})"))?;
        let file = File::open(path).map_err(|e| format!("Couldn't open file ({e})"))?;
        let decoder = Decoder::new(BufReader::new(file))
            .map_err(|e| format!("Couldn't decode this audio ({e})"))?;
        // Probe the duration ourselves — rodio 0.20's `total_duration` is wrong.
        let duration = probe_duration(path);
        sink.append(decoder);
        sink.play();
        Ok(AudioPlayer {
            _stream: stream,
            _handle: handle,
            sink,
            duration,
        })
    }

    pub fn toggle_pause(&self) {
        if self.sink.is_paused() {
            self.sink.play();
        } else {
            self.sink.pause();
        }
    }

    pub fn is_paused(&self) -> bool {
        self.sink.is_paused()
    }

    /// Current playback position.
    pub fn position(&self) -> Duration {
        self.sink.get_pos()
    }

    /// Total track length, if the decoder could determine it.
    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    /// Jump to `pos` (best effort; ignored if the format can't seek).
    pub fn seek(&self, pos: Duration) {
        let _ = self.sink.try_seek(pos);
    }

    /// True once the track has played to the end.
    pub fn finished(&self) -> bool {
        self.sink.empty()
    }
}

/// Read a track's total duration via Symphonia (accurate `Time` → `Duration`
/// conversion, which rodio's own `total_duration` gets wrong). `None` if the
/// container doesn't record it.
fn probe_duration(path: &Path) -> Option<Duration> {
    let file = File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let track = probed.format.default_track()?;
    let tb = track.codec_params.time_base?;
    let frames = track.codec_params.n_frames?;
    let time = tb.calc_time(frames);
    Some(Duration::from_secs_f64(time.seconds as f64 + time.frac))
}
