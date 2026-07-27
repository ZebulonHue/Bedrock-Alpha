//! A local video file, blurred, playing behind the UI with its audio.
//!
//! The app never bundles or copies the media itself — only a path the user
//! sets in Settings. Decoding goes through the system's `ffmpeg`, which is
//! not guaranteed to be installed: every failure here is soft. No ffmpeg, a
//! bad path, or a file `ffmpeg` cannot open all mean "no background", logged
//! once, never a crash and never a panic.
//!
//! Two `ffmpeg` processes run per playback: one streams blurred RGBA frames
//! to a pipe, the other streams raw PCM to a pipe for `rodio`. They are
//! started with the same `-ss` seek so they begin at the same instant; there
//! is no ongoing lock-step beyond that, which is enough for ambience and not
//! attempted for anything needing frame-accurate audio/video sync.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};

/// One decoded video frame, RGBA8, at [`FRAME_W`]x[`FRAME_H`].
pub struct Frame {
    pub rgba: Vec<u8>,
}

/// Frame size for the background: small on purpose. It is shown heavily
/// blurred and scaled up to fill the window, so decoding at full resolution
/// would spend CPU on detail nobody will see.
const FRAME_W: u32 = 480;
const FRAME_H: u32 = 270;
const FRAME_FPS: u32 = 12;

/// One track in the compilation, as given by the user: a name and the
/// timestamp it starts at. Used only to pick a random point to start
/// playback — never to identify or reproduce the audio itself.
struct Track {
    #[allow(dead_code)]
    name: &'static str,
    start_secs: f64,
}

/// Start times taken from the video's own description. The last track's end
/// is bounded by the file's real duration via `ffprobe`, not guessed.
const TRACKS: &[Track] = &[
    Track { name: "Infinite Amethyst", start_secs: 0.0 },
    Track { name: "Comforting Memories", start_secs: 166.0 },
    Track { name: "Otherside", start_secs: 338.0 },
    Track { name: "Aerie", start_secs: 699.0 },
    Track { name: "Left to Bloom", start_secs: 816.0 },
    Track { name: "Creator", start_secs: 1018.0 },
    Track { name: "Infinite Amethyst", start_secs: 1165.0 },
    Track { name: "Comforting Memories", start_secs: 1332.0 },
    Track { name: "Otherside", start_secs: 1500.0 },
    Track { name: "Aerie", start_secs: 1862.0 },
    Track { name: "Left to Bloom", start_secs: 1979.0 },
    Track { name: "Creator", start_secs: 2183.0 },
    Track { name: "Infinite Amethyst", start_secs: 2329.0 },
    Track { name: "Comforting Memories", start_secs: 2495.0 },
    Track { name: "Otherside", start_secs: 2664.0 },
    Track { name: "Aerie", start_secs: 3025.0 },
    Track { name: "Left to Bloom", start_secs: 3143.0 },
    Track { name: "Creator", start_secs: 3347.0 },
    Track { name: "Otherside", start_secs: 3487.0 },
];

/// Pick a random point within a random track's span.
///
/// Landing exactly on a track's first sample every time would make "random"
/// mean "the same handful of intros" in practice; a point somewhere across
/// the track reflects what the user actually asked for.
fn random_start(total_duration: f64) -> f64 {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let i = rng.gen_range(0..TRACKS.len());
    let start = TRACKS[i].start_secs;
    let end = TRACKS.get(i + 1).map_or(total_duration, |t| t.start_secs);
    if end > start {
        rng.gen_range(start..end)
    } else {
        start
    }
}

/// Locate `ffmpeg`/`ffprobe` on PATH, or `None` if either is missing.
///
/// Checked once at startup rather than per-failure so a missing ffmpeg
/// produces one clear log line instead of one per attempted spawn.
fn find_tool(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(&exe))
            .find(|candidate| candidate.is_file())
    })
}

fn probe_duration(ffprobe: &Path, media: &Path) -> Option<f64> {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(media)
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

/// A running (or failed-to-start) background playback session.
pub struct BackgroundMedia {
    frame_rx: Receiver<Frame>,
    latest: Option<Frame>,
    _video_child: Option<Child>,
    _audio_child: Option<Child>,
    /// Kept alive for the duration of playback; dropping it stops audio.
    _audio_stream: Option<rodio::OutputStream>,
    /// Lets [`Self::set_volume`] adjust playback already in progress.
    sink: Option<rodio::Sink>,
}

impl BackgroundMedia {
    /// Frame width/height, for callers building a texture to blit into.
    pub const FRAME_SIZE: [u32; 2] = [FRAME_W, FRAME_H];

    /// Start playing `media` from a random point. Returns `None` and logs the
    /// reason on any failure — no ffmpeg, no ffprobe, a path that does not
    /// exist, a file ffmpeg cannot read.
    pub fn start(media: &Path, play_audio: bool, blur_sigma: f32, volume: f32) -> Option<Self> {
        if !media.is_file() {
            tracing::warn!(
                "background media path does not exist: {} — background disabled",
                media.display()
            );
            return None;
        }
        let Some(ffmpeg) = find_tool("ffmpeg") else {
            tracing::warn!(
                "ffmpeg not found on PATH — background video disabled. \
                 Install it (e.g. `winget install ffmpeg`) to enable it."
            );
            return None;
        };
        let Some(ffprobe) = find_tool("ffprobe") else {
            tracing::warn!("ffprobe not found on PATH — background video disabled.");
            return None;
        };
        let Some(duration) = probe_duration(&ffprobe, media) else {
            tracing::warn!(
                "ffprobe could not read {} — background disabled",
                media.display()
            );
            return None;
        };

        let start_at = random_start(duration);
        tracing::info!(
            "Background media: {} at {:.0}s of {:.0}s",
            media.display(),
            start_at,
            duration
        );

        let (tx, rx): (SyncSender<Frame>, Receiver<Frame>) = sync_channel(4);
        let video_child = spawn_video(&ffmpeg, media, start_at, tx, blur_sigma);

        let mut audio_child = None;
        let mut audio_stream = None;
        let mut sink = None;
        if play_audio {
            match spawn_audio(&ffmpeg, media, start_at, volume) {
                Some((child, stream, s)) => {
                    audio_child = Some(child);
                    audio_stream = Some(stream);
                    sink = Some(s);
                }
                None => tracing::warn!("background audio failed to start; video continues muted"),
            }
        }

        Some(Self {
            frame_rx: rx,
            latest: None,
            _video_child: video_child,
            _audio_child: audio_child,
            _audio_stream: audio_stream,
            sink,
        })
    }

    /// Change volume live -- no restart, unlike blur or the file itself,
    /// since `Sink::set_volume` takes effect immediately on already-playing
    /// audio.
    pub fn set_volume(&self, volume: f32) {
        if let Some(sink) = &self.sink {
            sink.set_volume(volume.clamp(0.0, 1.0));
        }
    }

    /// Pull the newest decoded frame, if one has arrived since the last call.
    /// Keeps showing the previous frame between decodes rather than flicker.
    pub fn latest_frame(&mut self) -> Option<&Frame> {
        while let Ok(frame) = self.frame_rx.try_recv() {
            self.latest = Some(frame);
        }
        self.latest.as_ref()
    }
}

/// Drain a child's stderr on a background thread and log it once the pipe
/// closes (the process exited). ffmpeg is chatty on success at `-loglevel
/// error`, so a non-empty result here means something worth seeing.
fn log_stderr_on_exit(label: &'static str, stderr: Option<std::process::ChildStderr>) {
    let Some(mut stderr) = stderr else { return };
    std::thread::Builder::new()
        .name(format!("background-{label}-stderr"))
        .spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            let text = text.trim();
            if !text.is_empty() {
                tracing::warn!("ffmpeg ({label}) reported: {text}");
            }
        })
        .ok();
}

fn spawn_video(
    ffmpeg: &Path,
    media: &Path,
    start_at: f64,
    tx: SyncSender<Frame>,
    blur_sigma: f32,
) -> Option<Child> {
    // Blur in ffmpeg itself rather than in a shader: `gblur` is a single
    // well-tested filter, and doing it here means the renderer only ever
    // handles an already-finished image.
    let mut child = Command::new(ffmpeg)
        .args(["-ss", &start_at.to_string(), "-i"])
        .arg(media)
        .args([
            "-loglevel",
            "error",
            "-an",
            "-vf",
            &format!("scale={FRAME_W}:{FRAME_H},gblur=sigma={blur_sigma},fps={FRAME_FPS}"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| tracing::warn!("failed to start ffmpeg (video): {e}"))
        .ok()?;
    log_stderr_on_exit("video", child.stderr.take());

    let mut stdout = child.stdout.take()?;
    let frame_bytes = (FRAME_W * FRAME_H * 4) as usize;
    std::thread::Builder::new()
        .name("background-video-decode".into())
        .spawn(move || {
            let mut buf = vec![0u8; frame_bytes];
            loop {
                if stdout.read_exact(&mut buf).is_err() {
                    // EOF (file ended) or the process was torn down when
                    // playback stopped. Either way, nothing more to send.
                    break;
                }
                if tx.send(Frame { rgba: buf.clone() }).is_err() {
                    break;
                }
            }
        })
        .ok();

    Some(child)
}

fn spawn_audio(
    ffmpeg: &Path,
    media: &Path,
    start_at: f64,
    volume: f32,
) -> Option<(Child, rodio::OutputStream, rodio::Sink)> {
    const SAMPLE_RATE: u32 = 48_000;
    const CHANNELS: u16 = 2;

    let mut child = Command::new(ffmpeg)
        .args(["-ss", &start_at.to_string(), "-i"])
        .arg(media)
        .args([
            "-loglevel",
            "error",
            "-vn",
            "-f",
            "s16le",
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-ac",
            &CHANNELS.to_string(),
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| tracing::warn!("failed to start ffmpeg (audio): {e}"))
        .ok()?;
    log_stderr_on_exit("audio", child.stderr.take());

    let stdout = child.stdout.take()?;
    let source = RawPcmSource {
        reader: std::io::BufReader::with_capacity(8192, stdout),
        channels: CHANNELS,
        sample_rate: SAMPLE_RATE,
    };
    let (stream, handle) = rodio::OutputStream::try_default()
        .map_err(|e| tracing::warn!("no audio output device: {e}"))
        .ok()?;
    // A `Sink` rather than `play_raw` directly: it is the only way rodio
    // offers to change volume on audio that is already playing, which is
    // what a live volume slider needs.
    let sink = rodio::Sink::try_new(&handle)
        .map_err(|e| tracing::warn!("failed to create audio sink: {e}"))
        .ok()?;
    sink.set_volume(volume.clamp(0.0, 1.0));
    sink.append(source);

    Some((child, stream, sink))
}

/// Raw little-endian i16 PCM straight off ffmpeg's pipe, as a `rodio::Source`.
///
/// Simpler and more robust than wrapping the stream in a synthetic WAV header
/// for `rodio::Decoder`: that path failed here with "Unrecognized format",
/// and debugging a container decoder's opinion of a hand-built header is far
/// more fragile than just being the source directly -- the format is already
/// known exactly, since this process asked ffmpeg to produce it.
struct RawPcmSource<R> {
    reader: std::io::BufReader<R>,
    channels: u16,
    sample_rate: u32,
}

impl<R: Read> Iterator for RawPcmSource<R> {
    type Item = i16;
    fn next(&mut self) -> Option<i16> {
        let mut buf = [0u8; 2];
        self.reader.read_exact(&mut buf).ok()?;
        Some(i16::from_le_bytes(buf))
    }
}

impl<R: Read + Send> rodio::Source for RawPcmSource<R> {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

impl Drop for BackgroundMedia {
    fn drop(&mut self) {
        // Processes are killed rather than left to exit on their own: ffmpeg
        // writing to a pipe nobody reads any more otherwise blocks instead of
        // exiting, leaking a process every time the background is changed.
        if let Some(child) = &mut self._video_child {
            let _ = child.kill();
        }
        if let Some(child) = &mut self._audio_child {
            let _ = child.kill();
        }
    }
}

/// Whether ffmpeg and ffprobe are both on PATH, so the Settings panel can
/// grey out the background-media field instead of letting the user pick a
/// file and only finding out it silently did nothing.
pub fn ffmpeg_available() -> bool {
    find_tool("ffmpeg").is_some() && find_tool("ffprobe").is_some()
}
