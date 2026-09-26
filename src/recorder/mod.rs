//! RustCast's built-in screen recorder.
//!
//! Two kinds of recordings:
//!
//! - **Locked window** — the recording is locked onto one application window.
//!   It follows the window wherever it moves, other windows dragged on top of it
//!   never appear, and it keeps recording while the window is minimized (see
//!   [`x11_capture`]). With *aspect lock* on, every frame is fitted into a fixed
//!   canvas so resizing the window never changes the video's dimensions.
//! - **Full screen** — a whole monitor.
//!
//! On X11 (and for XWayland windows) capture is done directly; on a Wayland
//! session full-screen and native-Wayland windows go through the desktop's
//! screen-cast portal ([`portal`]). All paths encode with ffmpeg.

pub mod encoder;
pub mod frame;
pub mod indicator;
pub mod portal;
pub mod x11_capture;

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

use crate::app::Message;
use crate::app::tile::ExtSender;
use crate::config::{RecorderConfig, Theme};
use crate::platform::linux::x11::{self, Rect};

use encoder::{EncodeSpec, Ffmpeg};
use frame::{Canvas, RectI, blit_fit, even};
use portal::PortalSource;
use x11_capture::{Grab, WindowCapture};

/// What to record.
#[derive(Debug, Clone, PartialEq)]
pub enum RecordTarget {
    /// Lock onto one X11/XWayland window.
    Window { xid: u32, title: String },
    /// A monitor rectangle, grabbed directly (X11 sessions).
    Monitor { name: String, rect: Rect },
    /// Let the user pick in the system dialog (Wayland sessions).
    Portal(PortalSource),
}

impl RecordTarget {
    pub fn label(&self) -> String {
        match self {
            RecordTarget::Window { title, .. } => title.clone(),
            RecordTarget::Monitor { name, .. } => format!("Full screen ({name})"),
            RecordTarget::Portal(PortalSource::Window) => "Window".to_string(),
            RecordTarget::Portal(PortalSource::Monitor) => "Full screen".to_string(),
        }
    }
}

/// A snapshot of the running recording, for the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub label: String,
    /// The locked window, when this is a locked-window recording (other
    /// windows can then be added to it).
    pub locked: Option<u32>,
    /// Windows added on top of the locked window: (xid, title).
    pub layers: Vec<(u32, String)>,
    /// `None` while still starting (e.g. the portal dialog is open).
    pub elapsed: Option<Duration>,
    pub output: PathBuf,
}

/// Live, shared settings of a locked recording that can change mid-recording.
#[derive(Default)]
struct Scene {
    layers: Mutex<Vec<(u32, String)>>,
    pip: AtomicBool,
}

struct Active {
    id: u64,
    stop: Arc<AtomicBool>,
    locked: Option<u32>,
    scene: Arc<Scene>,
    label: String,
    started: Option<Instant>,
    output: PathBuf,
    indicator: Option<Child>,
}

static ACTIVE: Lazy<Mutex<Option<Active>>> = Lazy::new(|| Mutex::new(None));
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub fn is_recording() -> bool {
    ACTIVE.lock().map(|a| a.is_some()).unwrap_or(false)
}

pub fn status() -> Option<Status> {
    let guard = ACTIVE.lock().ok()?;
    let active = guard.as_ref()?;
    Some(Status {
        label: active.label.clone(),
        locked: active.locked,
        layers: active
            .scene
            .layers
            .lock()
            .map(|l| l.clone())
            .unwrap_or_default(),
        elapsed: active.started.map(|s| s.elapsed()),
        output: active.output.clone(),
    })
}

/// Ask the running recording (if any) to stop. It finishes asynchronously and
/// reports back through a notification and [`Message::RecorderChanged`].
pub fn stop() {
    if let Ok(guard) = ACTIVE.lock()
        && let Some(active) = guard.as_ref()
    {
        active.stop.store(true, Ordering::Relaxed);
    }
}

/// Bring another window into the running locked recording. It is drawn on
/// top of the locked window (captured occlusion-proof, like the locked one).
pub fn add_window(xid: u32, title: String) -> Result<(), String> {
    let guard = ACTIVE.lock().map_err(|_| "recorder state poisoned")?;
    let active = guard.as_ref().ok_or("nothing is being recorded")?;
    if active.locked.is_none() {
        return Err("full-screen recordings already include every window".to_string());
    }
    if active.locked == Some(xid) {
        return Ok(());
    }
    let mut layers = active.scene.layers.lock().map_err(|_| "poisoned")?;
    if !layers.iter().any(|(id, _)| *id == xid) {
        layers.push((xid, title));
    }
    Ok(())
}

/// Take an added window out of the running recording again.
pub fn remove_window(xid: u32) {
    if let Ok(guard) = ACTIVE.lock()
        && let Some(active) = guard.as_ref()
        && let Ok(mut layers) = active.scene.layers.lock()
    {
        layers.retain(|(id, _)| *id != xid);
    }
}

/// Switch the added-windows layout of the running recording.
pub fn set_picture_in_picture(on: bool) {
    if let Ok(guard) = ACTIVE.lock()
        && let Some(active) = guard.as_ref()
    {
        active.scene.pip.store(on, Ordering::Relaxed);
    }
}

/// Windows that can be locked onto: every normal window except RustCast's own.
pub fn recordable_windows() -> Vec<x11::ClientWindow> {
    let me = std::process::id();
    x11::client_windows()
        .into_iter()
        .filter(|w| w.pid != Some(me) && !w.class.to_lowercase().contains("rustcast"))
        .filter(|w| !w.title.trim().is_empty() || !w.class.trim().is_empty())
        .collect()
}

/// Human-friendly name for a window: its title, or its class if untitled.
pub fn window_label(w: &x11::ClientWindow) -> String {
    if w.title.trim().is_empty() {
        w.class
            .split_whitespace()
            .last()
            .unwrap_or("Window")
            .to_string()
    } else {
        w.title.trim().to_string()
    }
}

fn notify(summary: &str, body: &str) {
    let (summary, body) = (summary.to_string(), body.to_string());
    std::thread::spawn(move || {
        let _ = std::process::Command::new("notify-send")
            .args(["-a", "RustCast", "-i", "media-record", &summary, &body])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
}

fn tell_ui(sender: &Option<ExtSender>) {
    if let Some(s) = sender {
        let _ = s.0.clone().try_send(Message::RecorderChanged);
    }
}

fn display_path(path: &std::path::Path) -> String {
    let p = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && p.starts_with(&home) => format!("~{}", &p[home.len()..]),
        _ => p,
    }
}

/// Start recording `target`. Returns immediately; the recording runs on its
/// own thread. Fails fast (with a user-facing message) when something required
/// is missing or a recording is already running.
pub fn start(
    target: RecordTarget,
    cfg: RecorderConfig,
    theme: Theme,
    sender: Option<ExtSender>,
) -> Result<(), String> {
    if is_recording() {
        return Err("a recording is already running".to_string());
    }
    let codec = encoder::detect_codec()?;
    let dir = cfg.output_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;

    let label = target.label();
    let output = encoder::output_path(&dir, &label);
    let spec = EncodeSpec {
        fps: cfg.fps(),
        audio: cfg.record_audio,
        codec,
        output: output.clone(),
    };

    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let stop_flag = Arc::new(AtomicBool::new(false));
    let scene = Arc::new(Scene::default());
    scene.pip.store(cfg.picture_in_picture, Ordering::Relaxed);
    let locked = match &target {
        RecordTarget::Window { xid, .. } => Some(*xid),
        _ => None,
    };
    {
        let mut guard = ACTIVE.lock().map_err(|_| "recorder state poisoned")?;
        *guard = Some(Active {
            id,
            stop: stop_flag.clone(),
            locked,
            scene: scene.clone(),
            label: label.clone(),
            started: None,
            output: output.clone(),
            indicator: None,
        });
    }
    tell_ui(&sender);

    let on_started = {
        let (label, sender, show) = (label.clone(), sender.clone(), cfg.show_indicator);
        move || {
            let indicator = if show {
                indicator::spawn(&label, &theme, locked.is_some())
            } else {
                None
            };
            let watch_pid = indicator.as_ref().map(|c| c.id());
            if let Ok(mut guard) = ACTIVE.lock()
                && let Some(active) = guard.as_mut().filter(|a| a.id == id)
            {
                active.started = Some(Instant::now());
                active.indicator = indicator;
            }
            if let Some(pid) = watch_pid {
                watch_indicator(id, pid);
            }
            log::info!("Recorder: recording started ({label})");
            tell_ui(&sender);
        }
    };

    std::thread::Builder::new()
        .name("rustcast-recorder".to_string())
        .spawn(move || {
            let result = match &target {
                RecordTarget::Window { xid, .. } => {
                    record_window(*xid, &cfg, spec, stop_flag, scene, on_started)
                }
                RecordTarget::Monitor { rect, .. } => {
                    record_monitor(*rect, &cfg, spec, stop_flag, on_started)
                }
                RecordTarget::Portal(source) => {
                    portal::run(*source, &cfg, spec, stop_flag, on_started)
                }
            };
            finish(id, result, &sender);
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// When the indicator subprocess exits (its Stop button), stop the recording.
fn watch_indicator(id: u64, pid: u32) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let Ok(mut guard) = ACTIVE.lock() else { return };
            let Some(active) = guard.as_mut().filter(|a| a.id == id) else {
                return;
            };
            let exited = active
                .indicator
                .as_mut()
                .filter(|c| c.id() == pid)
                .map(|c| !matches!(c.try_wait(), Ok(None)))
                .unwrap_or(true);
            if exited {
                active.stop.store(true, Ordering::Relaxed);
                return;
            }
        }
    });
}

fn finish(id: u64, result: Result<(), String>, sender: &Option<ExtSender>) {
    let active = ACTIVE.lock().ok().and_then(|mut g| {
        if g.as_ref().is_some_and(|a| a.id == id) {
            g.take()
        } else {
            None
        }
    });
    let Some(mut active) = active else { return };
    if let Some(mut child) = active.indicator.take() {
        let _ = child.kill();
        let _ = child.wait();
    }

    let saved = active
        .output
        .metadata()
        .map(|m| m.len() > 0)
        .unwrap_or(false);
    match result {
        Ok(()) if saved => {
            log::info!("Recorder: saved {}", active.output.display());
            notify("Recording saved", &display_path(&active.output));
        }
        Ok(()) => {
            let _ = std::fs::remove_file(&active.output);
        }
        Err(e) => {
            log::warn!("Recorder: {e}");
            if !saved {
                let _ = std::fs::remove_file(&active.output);
            }
            if e != "recording cancelled" {
                notify("Recording failed", &e);
            }
        }
    }
    tell_ui(sender);
}

/// A window added on top of the locked one.
struct Layer {
    xid: u32,
    cap: WindowCapture,
    buf: Vec<u8>,
    /// Last good frame's root position and size.
    geom: Option<(i32, i32, u32, u32)>,
}

/// Corner radius / shadow of added windows, relative to the canvas size so it
/// looks the same at any output resolution.
fn layer_style(canvas: &Canvas) -> (f32, f32) {
    let unit = canvas.width.min(canvas.height) as f32 / 1080.0;
    ((12.0 * unit).max(4.0), (22.0 * unit).max(6.0))
}

/// Draw the added windows over the locked window's image. `base` is the locked
/// window's root geometry and `area` where its image sits in the canvas.
fn compose_layers(
    canvas: &mut Canvas,
    layers: &[Layer],
    base: (i32, i32, u32, u32),
    area: RectI,
    pip: bool,
) {
    let (radius, blur) = layer_style(canvas);
    let visible: Vec<&Layer> = layers.iter().filter(|l| l.geom.is_some()).collect();
    let rects: Vec<RectI> = if pip {
        let sizes: Vec<(u32, u32)> = visible
            .iter()
            .map(|l| l.geom.map(|g| (g.2, g.3)).unwrap_or((1, 1)))
            .collect();
        frame::pip_slots(area, &sizes)
    } else {
        // Real position relative to the locked window, at the same scale.
        let scale = area.w as f64 / base.2.max(1) as f64;
        visible
            .iter()
            .map(|l| {
                let (x, y, w, h) = l.geom.expect("filtered above");
                RectI::new(
                    area.x + ((x - base.0) as f64 * scale).round() as i32,
                    area.y + ((y - base.1) as f64 * scale).round() as i32,
                    (w as f64 * scale).round().max(1.0) as i32,
                    (h as f64 * scale).round().max(1.0) as i32,
                )
            })
            .collect()
    };
    for (layer, dst) in visible.iter().zip(rects) {
        let (_, _, w, h) = layer.geom.expect("filtered above");
        frame::draw_shadow(canvas, dst, area, radius, blur, 0.45);
        frame::blit_into(
            &layer.buf,
            w,
            h,
            w as usize * frame::BPP,
            canvas,
            dst,
            area,
            radius,
        );
    }
}

/// Keep `layers` in sync with the scene's list, ordered bottom-to-top.
fn sync_layers(layers: &mut Vec<Layer>, wanted: &[(u32, String)]) {
    layers.retain(|l| wanted.iter().any(|(id, _)| *id == l.xid));
    for (xid, title) in wanted {
        if layers.iter().any(|l| l.xid == *xid) {
            continue;
        }
        match WindowCapture::new(*xid) {
            Ok(cap) => {
                log::info!("Recorder: added {title} to the recording");
                layers.push(Layer {
                    xid: *xid,
                    cap,
                    buf: Vec::new(),
                    geom: None,
                });
            }
            Err(e) => log::warn!("Recorder: cannot add {title}: {e}"),
        }
    }
    let stacking = x11::stacking_order();
    layers.sort_by_key(|l| {
        stacking
            .iter()
            .position(|w| *w == l.xid)
            .unwrap_or(usize::MAX)
    });
}

/// Locked-window recording loop.
fn record_window(
    xid: u32,
    cfg: &RecorderConfig,
    spec: EncodeSpec,
    stop: Arc<AtomicBool>,
    scene: Arc<Scene>,
    on_started: impl FnOnce(),
) -> Result<(), String> {
    let mut cap = WindowCapture::new(xid)?;
    let (cw, ch) = if cfg.aspect_lock {
        cfg.output_size()
    } else {
        let (w, h) = cap
            .content_size()
            .ok_or_else(|| "that window no longer exists".to_string())?;
        (even(w), even(h))
    };
    let mut canvas = Canvas::new(cw, ch);
    let fps = cfg.fps();

    let mut ffmpeg = Ffmpeg::spawn(
        encoder::raw_input_args(canvas.width, canvas.height, fps),
        encoder::output_args(&spec, None),
        Stdio::piped(),
    )?;
    let mut stdin = ffmpeg.child.stdin.take().ok_or("ffmpeg has no stdin")?;
    on_started();

    let mut buf = Vec::new();
    let mut base: Option<(i32, i32, u32, u32)> = None;
    let mut layers: Vec<Layer> = Vec::new();
    let mut synced: Vec<(u32, String)> = Vec::new();
    let started = Instant::now();
    let frame_time = Duration::from_secs_f64(1.0 / fps as f64);
    let mut written: u64 = 0;
    let mut tick: u64 = 0;
    let mut failure = None;

    while !stop.load(Ordering::Relaxed) {
        tick += 1;
        match cap.grab(&mut buf, cfg.show_cursor) {
            Grab::Frame {
                x,
                y,
                width,
                height,
            } => base = Some((x, y, width, height)),
            Grab::Unavailable => {
                // Keep the last good frame on screen (never black) and, if the
                // window was minimized, ghost it so it keeps rendering.
                if cfg.keep_recording_when_minimized && cap.is_minimized() {
                    cap.ghost();
                }
            }
            Grab::Gone => {
                log::info!("Recorder: locked window closed — finishing recording");
                break;
            }
        }
        if cap.is_ghosted() {
            cap.tick_ghost();
        }

        // Added windows: pick up changes, then grab each one.
        let wanted = scene.layers.lock().map(|l| l.clone()).unwrap_or_default();
        if wanted != synced || tick.is_multiple_of((fps as u64).max(1)) {
            sync_layers(&mut layers, &wanted);
            synced = wanted;
        }
        let mut closed = Vec::new();
        for layer in layers.iter_mut() {
            match layer.cap.grab(&mut layer.buf, cfg.show_cursor) {
                Grab::Frame {
                    x,
                    y,
                    width,
                    height,
                } => layer.geom = Some((x, y, width, height)),
                Grab::Unavailable => {
                    if cfg.keep_recording_when_minimized && layer.cap.is_minimized() {
                        layer.cap.ghost();
                    }
                }
                Grab::Gone => closed.push(layer.xid),
            }
            if layer.cap.is_ghosted() {
                layer.cap.tick_ghost();
            }
        }
        for xid in closed {
            remove_window(xid);
        }

        if let Some(b) = base {
            blit_fit(&buf, b.2, b.3, b.2 as usize * frame::BPP, &mut canvas);
            if !layers.is_empty() {
                let (ax, ay, aw, ah) = frame::fit_rect(b.2, b.3, canvas.width, canvas.height);
                let area = RectI::new(ax as i32, ay as i32, aw as i32, ah as i32);
                compose_layers(
                    &mut canvas,
                    &layers,
                    b,
                    area,
                    scene.pip.load(Ordering::Relaxed),
                );
            }
        }

        // Emit as many frames as wall-clock time calls for, so the video stays
        // in real time even if a grab was slow.
        let due = (started.elapsed().as_secs_f64() * fps as f64) as u64 + 1;
        let mut burst = 0;
        while written < due && burst < fps * 2 {
            if let Err(e) = stdin.write_all(&canvas.data) {
                failure = Some(e);
                break;
            }
            written += 1;
            burst += 1;
        }
        if written < due {
            written = due; // drop the backlog rather than spiral
        }
        if failure.is_some() {
            break;
        }

        let next = frame_time.mul_f64(written as f64);
        if let Some(wait) = next.checked_sub(started.elapsed()) {
            std::thread::sleep(wait.min(frame_time));
        }
    }

    drop(stdin);
    drop(layers);
    drop(cap); // un-redirects, and re-minimizes a ghosted window
    let finished = ffmpeg.finish(Duration::from_secs(30));
    match (failure, finished) {
        (_, Err(e)) => Err(e),
        (Some(e), Ok(())) => Err(format!("writing frames failed: {e}")),
        (None, Ok(())) => Ok(()),
    }
}

/// Full-monitor recording on X11 via ffmpeg's `x11grab`.
fn record_monitor(
    rect: Rect,
    cfg: &RecorderConfig,
    spec: EncodeSpec,
    stop: Arc<AtomicBool>,
    on_started: impl FnOnce(),
) -> Result<(), String> {
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string());
    let filter = if cfg.aspect_lock {
        let (w, h) = cfg.output_size();
        encoder::fit_filter(w, h)
    } else {
        encoder::even_filter()
    };
    let mut ffmpeg = Ffmpeg::spawn(
        encoder::x11grab_input_args(
            &display,
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            cfg.fps(),
            cfg.show_cursor,
        ),
        encoder::output_args(&spec, Some(filter)),
        Stdio::piped(),
    )?;
    let mut stdin = ffmpeg.child.stdin.take();
    on_started();

    while !stop.load(Ordering::Relaxed) {
        if let Ok(Some(_)) = ffmpeg.child.try_wait() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // `q` asks ffmpeg to stop grabbing and finalize the file.
    if let Some(stdin) = stdin.as_mut() {
        let _ = stdin.write_all(b"q");
        let _ = stdin.flush();
    }
    drop(stdin);
    ffmpeg.finish(Duration::from_secs(30))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_labels_are_descriptive() {
        let w = RecordTarget::Window {
            xid: 1,
            title: "Firefox".to_string(),
        };
        assert_eq!(w.label(), "Firefox");
        let m = RecordTarget::Monitor {
            name: "DP-1".to_string(),
            rect: Rect {
                x: 0,
                y: 0,
                w: 10,
                h: 10,
            },
        };
        assert_eq!(m.label(), "Full screen (DP-1)");
        assert_eq!(
            RecordTarget::Portal(PortalSource::Monitor).label(),
            "Full screen"
        );
    }

    #[test]
    fn window_label_falls_back_to_class() {
        let w = x11::ClientWindow {
            id: 1,
            title: "  ".to_string(),
            class: "gnome-terminal Gnome-terminal".to_string(),
            pid: None,
            minimized: false,
        };
        assert_eq!(window_label(&w), "Gnome-terminal");
    }
}

/// End-to-end check against a real X server. Run with e.g.
/// `Xvfb :99 &` plus two overlapping windows, then
/// `DISPLAY=:99 RUSTCAST_TEST_BASE=<xid> RUSTCAST_TEST_TOP=<xid> RUSTCAST_TEST_PROBE=x,y
///  cargo test recorder::live -- --ignored`
/// where the probe point (relative to the base window) is covered by the top window.
#[cfg(test)]
mod live {
    use super::*;

    fn env_u32(name: &str) -> u32 {
        let v = std::env::var(name).unwrap_or_else(|_| panic!("{name} not set"));
        u32::from_str_radix(v.trim_start_matches("0x"), 16)
            .or_else(|_| v.parse())
            .unwrap()
    }

    /// Record the base window for ~1.5s (optionally with `layer` added) and
    /// return the RGB at the probe point of a decoded frame.
    fn record_and_probe(layer: Option<u32>) -> [u8; 3] {
        let base = env_u32("RUSTCAST_TEST_BASE");
        let probe = std::env::var("RUSTCAST_TEST_PROBE").unwrap();
        let (px, py) = probe.split_once(',').unwrap();
        let (px, py): (u32, u32) = (px.parse().unwrap(), py.parse().unwrap());

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("t.mp4");
        let cfg = RecorderConfig {
            aspect_lock: false,
            show_cursor: false,
            fps: 15,
            ..RecorderConfig::default()
        };
        let spec = EncodeSpec {
            fps: 15,
            audio: false,
            codec: encoder::detect_codec().unwrap(),
            output: out.clone(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let scene = Arc::new(Scene::default());
        if let Some(top) = layer {
            scene.layers.lock().unwrap().push((top, "top".to_string()));
        }
        let s2 = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            s2.store(true, Ordering::Relaxed);
        });
        record_window(base, &cfg, spec, stop, scene, || {}).unwrap();

        // Decode one late frame to raw RGB and read the probe pixel.
        let img = dir.path().join("f.png");
        let ok = std::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-sseof", "-0.3", "-i"])
            .arg(&out)
            .args(["-frames:v", "1", "-y"])
            .arg(&img)
            .status()
            .unwrap();
        assert!(ok.success());
        let rgb = image::open(&img).unwrap().to_rgb8();
        rgb.get_pixel(px, py).0
    }

    #[test]
    #[ignore]
    fn locked_window_ignores_overlap_and_layers_add_windows() {
        let top = env_u32("RUSTCAST_TEST_TOP");
        let plain = record_and_probe(None);
        let layered = record_and_probe(Some(top));
        eprintln!("probe without layer: {plain:?}, with layer: {layered:?}");
        // Base is reddish, top is bluish (see the test setup).
        assert!(plain[0] > plain[2] + 60, "overlap leaked in: {plain:?}");
        assert!(layered[2] > layered[0] + 60, "layer missing: {layered:?}");
    }
}

#[cfg(test)]
mod live_minimize {
    use super::*;

    /// With a window manager running: minimize `RUSTCAST_TEST_BASE` while it
    /// is being recorded (from outside the test) and check the recording kept
    /// real frames. Returns via panic on failure.
    #[test]
    #[ignore]
    fn minimized_locked_window_keeps_recording() {
        let base = u32::from_str_radix(
            std::env::var("RUSTCAST_TEST_BASE")
                .unwrap()
                .trim_start_matches("0x"),
            16,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("m.mp4");
        let cfg = RecorderConfig {
            aspect_lock: true,
            output_width: 640,
            output_height: 360,
            show_cursor: false,
            fps: 15,
            ..RecorderConfig::default()
        };
        let spec = EncodeSpec {
            fps: 15,
            audio: false,
            codec: encoder::detect_codec().unwrap(),
            output: out.clone(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(700));
            let _ = std::process::Command::new("xdotool")
                .args(["windowminimize", &base.to_string()])
                .status();
            std::thread::sleep(Duration::from_millis(1500));
            let st = std::process::Command::new("xprop")
                .args([
                    "-id",
                    &base.to_string(),
                    "WM_STATE",
                    "_NET_WM_WINDOW_OPACITY",
                ])
                .output()
                .unwrap();
            eprintln!("while recording:\n{}", String::from_utf8_lossy(&st.stdout));
            s2.store(true, Ordering::Relaxed);
        });
        record_window(base, &cfg, spec, stop, Arc::new(Scene::default()), || {}).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let st = std::process::Command::new("xprop")
            .args(["-id", &base.to_string(), "WM_STATE"])
            .output()
            .unwrap();
        eprintln!("after stop:\n{}", String::from_utf8_lossy(&st.stdout));

        let img = dir.path().join("f.png");
        assert!(
            std::process::Command::new("ffmpeg")
                .args(["-loglevel", "error", "-sseof", "-0.3", "-i"])
                .arg(&out)
                .args(["-frames:v", "1", "-y"])
                .arg(&img)
                .status()
                .unwrap()
                .success()
        );
        let rgb = image::open(&img).unwrap().to_rgb8();
        let p = rgb.get_pixel(320, 180).0;
        eprintln!("centre pixel after minimize: {p:?}");
        assert!(p[0] > 100, "recording went dark after minimize: {p:?}");
    }
}
