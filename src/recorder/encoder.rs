//! ffmpeg process plumbing for the recorder.
//!
//! Frames reach ffmpeg in one of two ways:
//! - as raw `bgr0` video on stdin (locked-window capture and the Wayland
//!   portal path), already fitted to the final canvas size, or
//! - via ffmpeg's own `x11grab` input (full-screen capture on X11).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// The H.264-ish encoder available in the local ffmpeg build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    X264,
    OpenH264,
    Mpeg4,
}

/// Find ffmpeg and pick the best available video encoder.
pub fn detect_codec() -> Result<VideoCodec, String> {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| {
            "ffmpeg is not installed — install it (e.g. `sudo apt install ffmpeg`) to record"
                .to_string()
        })?;
    Ok(pick_codec(&String::from_utf8_lossy(&out.stdout)))
}

fn pick_codec(encoders: &str) -> VideoCodec {
    let has = |name: &str| {
        encoders
            .lines()
            .any(|l| l.split_whitespace().nth(1) == Some(name))
    };
    if has("libx264") {
        VideoCodec::X264
    } else if has("libopenh264") {
        VideoCodec::OpenH264
    } else {
        VideoCodec::Mpeg4
    }
}

/// Everything that decides how the output file is encoded.
#[derive(Debug, Clone)]
pub struct EncodeSpec {
    pub fps: u32,
    pub audio: bool,
    pub codec: VideoCodec,
    pub output: PathBuf,
}

fn s(v: &str) -> String {
    v.to_string()
}

/// Input arguments for raw BGRx frames of a fixed size piped on stdin.
pub fn raw_input_args(width: u32, height: u32, fps: u32) -> Vec<String> {
    vec![
        s("-f"),
        s("rawvideo"),
        s("-thread_queue_size"),
        s("512"),
        s("-pixel_format"),
        s("bgr0"),
        s("-video_size"),
        format!("{width}x{height}"),
        s("-framerate"),
        fps.to_string(),
        s("-i"),
        s("pipe:0"),
    ]
}

/// Input arguments for grabbing a screen rectangle with `x11grab`.
pub fn x11grab_input_args(
    display: &str,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    fps: u32,
    cursor: bool,
) -> Vec<String> {
    vec![
        s("-f"),
        s("x11grab"),
        s("-thread_queue_size"),
        s("512"),
        s("-draw_mouse"),
        (if cursor { "1" } else { "0" }).to_string(),
        s("-framerate"),
        fps.to_string(),
        s("-video_size"),
        format!("{width}x{height}"),
        s("-i"),
        format!("{display}+{x},{y}"),
    ]
}

/// A video filter that fits any input into a fixed `w`×`h` frame, keeping the
/// aspect ratio and padding the rest (the "aspect lock").
pub fn fit_filter(w: u32, h: u32) -> String {
    format!(
        "scale={w}:{h}:force_original_aspect_ratio=decrease:flags=lanczos,\
         pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:black,setsar=1"
    )
}

/// A video filter that only trims odd pixel rows/columns (yuv420p needs even sizes).
pub fn even_filter() -> String {
    "crop=trunc(iw/2)*2:trunc(ih/2)*2,setsar=1".to_string()
}

/// Audio input + everything after the inputs: filters, codecs and the file.
pub fn output_args(spec: &EncodeSpec, filter: Option<String>) -> Vec<String> {
    let mut args = Vec::new();
    if spec.audio {
        // PulseAudio (or PipeWire's pulse server) default source.
        args.extend([
            s("-f"),
            s("pulse"),
            s("-thread_queue_size"),
            s("1024"),
            s("-i"),
            s("default"),
        ]);
    }
    if let Some(filter) = filter {
        args.extend([s("-vf"), filter]);
    }
    match spec.codec {
        VideoCodec::X264 => args.extend([
            s("-c:v"),
            s("libx264"),
            s("-preset"),
            s("veryfast"),
            s("-crf"),
            s("20"),
        ]),
        VideoCodec::OpenH264 => args.extend([s("-c:v"), s("libopenh264"), s("-b:v"), s("8M")]),
        VideoCodec::Mpeg4 => args.extend([s("-c:v"), s("mpeg4"), s("-q:v"), s("3")]),
    }
    args.extend([s("-pix_fmt"), s("yuv420p"), s("-r"), spec.fps.to_string()]);
    if spec.audio {
        args.extend([s("-c:a"), s("aac"), s("-b:a"), s("160k")]);
    } else {
        args.push(s("-an"));
    }
    args.extend([
        s("-movflags"),
        s("+faststart"),
        s("-y"),
        spec.output.to_string_lossy().to_string(),
    ]);
    args
}

/// A running ffmpeg process whose stderr is collected for error reporting.
pub struct Ffmpeg {
    pub child: Child,
    stderr: Arc<Mutex<String>>,
}

impl Ffmpeg {
    /// Spawn `ffmpeg` with the given input and output arguments. `stdin` is
    /// what ffmpeg reads from (a pipe for raw frames or for the `q` command).
    pub fn spawn(input: Vec<String>, output: Vec<String>, stdin: Stdio) -> Result<Self, String> {
        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostats"])
            .args(input)
            .args(output)
            .stdin(stdin)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to start ffmpeg: {e}"))?;

        let stderr = Arc::new(Mutex::new(String::new()));
        if let Some(mut pipe) = child.stderr.take() {
            let sink = stderr.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 1024];
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut s) = sink.lock() {
                        s.push_str(&String::from_utf8_lossy(&buf[..n]));
                        // Only the tail matters for error messages.
                        if s.len() > 4096 {
                            let cut = s.len() - 2048;
                            let cut = (cut..s.len()).find(|&i| s.is_char_boundary(i)).unwrap_or(0);
                            s.drain(..cut);
                        }
                    }
                }
            });
        }
        Ok(Ffmpeg { child, stderr })
    }

    /// The last error output ffmpeg printed, trimmed.
    pub fn error_output(&self) -> String {
        self.stderr
            .lock()
            .map(|s| s.trim().lines().last().unwrap_or("").to_string())
            .unwrap_or_default()
    }

    /// Wait for ffmpeg to exit, killing it after `timeout`. Returns an error
    /// message if it failed.
    pub fn finish(mut self, timeout: std::time::Duration) -> Result<(), String> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => {
                    // Give the stderr reader a moment to collect the message.
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    let msg = self.error_output();
                    return Err(if msg.is_empty() {
                        format!("ffmpeg exited with {status}")
                    } else {
                        format!("ffmpeg: {msg}")
                    });
                }
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return Err("ffmpeg did not finish in time".to_string());
                }
            }
        }
    }
}

/// `YYYY-MM-DD_HH-MM-SS` in local time, for file names.
pub fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    // SAFETY: localtime_r writes into the provided struct only.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// Turn a window title into a short, filesystem-safe file name fragment.
pub fn slug(label: &str) -> String {
    let mut out = String::new();
    for ch in label.chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.chars().count() >= 32 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "recording".to_string()
    } else {
        out
    }
}

/// The output path for a new recording of `label` inside `dir`.
pub fn output_path(dir: &Path, label: &str) -> PathBuf {
    dir.join(format!("RustCast-{}-{}.mp4", timestamp(), slug(label)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_best_available_encoder() {
        let listing = " V....D libx264              libx264 H.264\n V....D mpeg4   MPEG-4";
        assert_eq!(pick_codec(listing), VideoCodec::X264);
        assert_eq!(
            pick_codec(" V....D libopenh264  OpenH264\n"),
            VideoCodec::OpenH264
        );
        assert_eq!(pick_codec(" V....D mpeg4   MPEG-4"), VideoCodec::Mpeg4);
    }

    #[test]
    fn raw_input_describes_frame_geometry() {
        let args = raw_input_args(1280, 720, 30);
        let joined = args.join(" ");
        assert!(joined.contains("-pixel_format bgr0"));
        assert!(joined.contains("-video_size 1280x720"));
        assert!(joined.ends_with("-i pipe:0"));
    }

    #[test]
    fn x11grab_input_targets_display_offset() {
        let args = x11grab_input_args(":1", 1920, 0, 2560, 1440, 60, false);
        assert_eq!(args.last().unwrap(), ":1+1920,0");
        assert!(args.join(" ").contains("-draw_mouse 0"));
    }

    #[test]
    fn output_args_toggle_audio_and_filter() {
        let spec = EncodeSpec {
            fps: 30,
            audio: false,
            codec: VideoCodec::X264,
            output: PathBuf::from("/tmp/out.mp4"),
        };
        let args = output_args(&spec, Some(fit_filter(1920, 1080)));
        assert!(args.contains(&"-an".to_string()));
        assert!(!args.contains(&"pulse".to_string()));
        assert!(args.iter().any(|a| a.starts_with("scale=1920:1080")));
        assert_eq!(args.last().unwrap(), "/tmp/out.mp4");

        let args = output_args(
            &EncodeSpec {
                audio: true,
                ..spec
            },
            None,
        );
        assert!(args.contains(&"pulse".to_string()));
        assert!(args.contains(&"aac".to_string()));
        assert!(!args.contains(&"-vf".to_string()));
    }

    #[test]
    fn slug_is_filesystem_safe() {
        assert_eq!(slug("Firefox — GitHub/Home"), "firefox-github-home");
        assert_eq!(slug("   "), "recording");
        assert!(slug(&"a".repeat(100)).len() <= 32);
    }

    #[test]
    fn timestamp_has_expected_shape() {
        let ts = timestamp();
        assert_eq!(ts.len(), 19);
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[10..11], "_");
    }
}
