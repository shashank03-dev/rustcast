//! Wayland recording through `xdg-desktop-portal` (ScreenCast).
//!
//! On a Wayland session RustCast (running through XWayland) cannot read the
//! pixels of native-Wayland windows or of the real screen. The portal asks the
//! compositor instead: the user picks a window or monitor in the system dialog
//! and the compositor streams it over PipeWire. GNOME/mutter renders a *window*
//! stream from the window's own surface, so overlapping windows never appear in
//! it, and when the window is minimized the stream simply stops updating — the
//! last frame is repeated (`keepalive-time` + `videorate`) instead of going black.
//!
//! Pipeline: `pipewiresrc` (gst-launch-1.0) → fitted BGRx frames on a pipe →
//! ffmpeg (the same encoder settings as every other recording).

use std::os::fd::{AsRawFd, OwnedFd};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ashpd::desktop::PersistMode;
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};

use super::encoder::{EncodeSpec, Ffmpeg, output_args, raw_input_args};
use super::frame::even;
use crate::config::RecorderConfig;

/// What the system picker should offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalSource {
    Window,
    Monitor,
}

/// True when RustCast runs inside a Wayland session (see `main`, which stashes
/// the real `WAYLAND_DISPLAY` before forcing the X11 backend).
pub fn is_wayland_session() -> bool {
    std::env::var_os("RUSTCAST_WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t.eq_ignore_ascii_case("wayland"))
}

fn has_gst_element(name: &str) -> bool {
    Command::new("gst-inspect-1.0")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check the external GStreamer pieces before bothering the user with a dialog.
pub fn check_dependencies() -> Result<(), String> {
    if Command::new("gst-launch-1.0")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_err()
    {
        return Err(
            "gst-launch-1.0 is missing — install gstreamer1.0-tools and gstreamer1.0-pipewire"
                .to_string(),
        );
    }
    if !has_gst_element("pipewiresrc") {
        return Err(
            "the GStreamer PipeWire plugin is missing — install gstreamer1.0-pipewire".into(),
        );
    }
    Ok(())
}

struct PortalStream {
    node: u32,
    size: Option<(i32, i32)>,
    fd: OwnedFd,
}

async fn open_stream(
    source: PortalSource,
    cursor: bool,
) -> Result<
    (
        Screencast,
        ashpd::desktop::Session<Screencast>,
        PortalStream,
    ),
    String,
> {
    let proxy = Screencast::new()
        .await
        .map_err(|e| format!("screen-cast portal unavailable: {e}"))?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(|e| format!("portal session failed: {e}"))?;

    let modes = proxy.available_cursor_modes().await.unwrap_or_default();
    let cursor_mode = if cursor && modes.contains(CursorMode::Embedded) {
        CursorMode::Embedded
    } else {
        CursorMode::Hidden
    };
    let kind = match source {
        PortalSource::Window => SourceType::Window,
        PortalSource::Monitor => SourceType::Monitor,
    };
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(cursor_mode)
                .set_sources(ashpd::enumflags2::BitFlags::from_flag(kind))
                .set_multiple(false)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .map_err(|e| format!("portal source selection failed: {e}"))?;

    let streams = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(|e| format!("portal start failed: {e}"))?
        .response()
        .map_err(|_| "recording cancelled".to_string())?;
    let stream = streams
        .streams()
        .first()
        .ok_or_else(|| "nothing was selected".to_string())?;
    let (node, size) = (stream.pipe_wire_node_id(), stream.size());

    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(|e| format!("cannot open PipeWire remote: {e}"))?;
    Ok((proxy, session, PortalStream { node, size, fd }))
}

/// Spawn `gst-launch-1.0` reading the PipeWire node (with the portal fd at
/// fd 3) and writing fitted `width`×`height` BGRx frames to stdout.
fn spawn_gst(stream: &PortalStream, width: u32, height: u32, fps: u32) -> Result<Child, String> {
    use std::os::unix::process::CommandExt;

    let caps =
        format!("video/x-raw,format=BGRx,width={width},height={height},pixel-aspect-ratio=1/1");
    let rate = format!("video/x-raw,framerate={fps}/1");
    let mut cmd = Command::new("gst-launch-1.0");
    cmd.args(["-e", "-q", "pipewiresrc", "fd=3"])
        .arg(format!("path={}", stream.node))
        .args([
            "do-timestamp=true",
            "keepalive-time=100",
            "always-copy=true",
            "!",
            "videoconvert",
            "!",
            "videoscale",
            "add-borders=true",
            "!",
        ])
        .arg(caps)
        .args(["!", "videorate", "!"])
        .arg(rate)
        .args(["!", "fdsink", "fd=1", "sync=false"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let raw = stream.fd.as_raw_fd();
    // SAFETY: only async-signal-safe libc calls between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            if raw == 3 {
                let flags = libc::fcntl(3, libc::F_GETFD);
                libc::fcntl(3, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
            } else if libc::dup2(raw, 3) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd.spawn()
        .map_err(|e| format!("failed to start gst-launch-1.0: {e}"))
}

/// Record through the portal until `stop` is set or the stream ends.
pub fn run(
    source: PortalSource,
    cfg: &RecorderConfig,
    spec: EncodeSpec,
    stop: Arc<AtomicBool>,
    on_started: impl FnOnce(),
) -> Result<(), String> {
    check_dependencies()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let (_proxy, session, stream) = runtime.block_on(open_stream(source, cfg.show_cursor))?;

    let (width, height) = if cfg.aspect_lock {
        cfg.output_size()
    } else {
        stream
            .size
            .map(|(w, h)| (even(w.max(2) as u32), even(h.max(2) as u32)))
            .unwrap_or_else(|| cfg.output_size())
    };
    let fps = cfg.fps();

    let mut gst = spawn_gst(&stream, width, height, fps)?;
    let frames = gst.stdout.take().ok_or("gst-launch-1.0 has no stdout")?;
    let ffmpeg = match Ffmpeg::spawn(
        raw_input_args(width, height, fps),
        output_args(&spec, None),
        Stdio::from(frames),
    ) {
        Ok(f) => f,
        Err(e) => {
            let _ = gst.kill();
            return Err(e);
        }
    };
    on_started();

    let mut ffmpeg = ffmpeg;
    let mut ended_early = false;
    while !stop.load(Ordering::Relaxed) {
        if matches!(gst.try_wait(), Ok(Some(_))) || matches!(ffmpeg.child.try_wait(), Ok(Some(_))) {
            ended_early = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    // EOS through the pipeline: gst closes the pipe, ffmpeg finalizes the MP4.
    // SAFETY: plain kill(2) on our own child.
    unsafe { libc::kill(gst.id() as i32, libc::SIGINT) };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while matches!(gst.try_wait(), Ok(None)) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = gst.kill();
    let _ = gst.wait();
    let result = ffmpeg.finish(Duration::from_secs(20));

    let _ = runtime.block_on(session.close());
    drop(stream);

    match result {
        Ok(()) if ended_early => {
            log::info!("Recorder: portal stream ended (source closed)");
            Ok(())
        }
        other => other,
    }
}
