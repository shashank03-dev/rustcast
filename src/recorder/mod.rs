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
pub mod screen_capture;
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
    if is_rustcast_xid(xid) {
        return Err(OWN_WINDOW.to_string());
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

/// The parent pid from a `/proc/<pid>/stat` line. The command name is in
/// parentheses and may itself contain spaces or `)`, so parse after the last `)`.
fn ppid_from_stat(stat: &str) -> Option<u32> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

fn parent_pid(pid: u32) -> Option<u32> {
    ppid_from_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// Whether `pid` is RustCast itself or one of the helper processes it spawns
/// (the REC pill, screenshot thumbnails, …).
fn is_rustcast_pid(pid: u32) -> bool {
    let me = std::process::id();
    pid == me || parent_pid(pid) == Some(me)
}

/// True for windows that belong to RustCast — the launcher, the REC pill, the
/// screenshot thumbnail, file pickers it opens. They are never recorded: not
/// as the locked window, not as added windows, and they are painted out of
/// full-screen recordings.
pub fn is_rustcast_window(w: &x11::ClientWindow) -> bool {
    w.pid.is_some_and(is_rustcast_pid) || w.class.to_lowercase().contains("rustcast")
}

/// [`is_rustcast_window`] for a bare window id. Unknown windows count as not
/// RustCast (they simply fail to record later).
pub fn is_rustcast_xid(xid: u32) -> bool {
    x11::window_info(xid).is_some_and(|w| is_rustcast_window(&w))
}

const OWN_WINDOW: &str = "RustCast's own windows are never recorded";

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_terminate(_signal: libc::c_int) {
    // Only an atomic store: async-signal-safe.
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
}

/// Stop a running recording and wait (bounded) until the file is finalized
/// and every ghosted window is restored. Safe to call when idle.
pub fn shutdown() {
    if !is_recording() {
        return;
    }
    log::info!("Recorder: shutting down — finishing the recording first");
    stop();
    let deadline = Instant::now() + Duration::from_secs(10);
    while is_recording() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// On SIGTERM / SIGHUP / SIGINT (logout, `kill`, Ctrl+C), finish the
/// recording properly before exiting instead of leaving a broken file and an
/// invisible (ghosted) window behind.
pub fn install_shutdown_handler() {
    // SAFETY: the handler only performs an atomic store.
    unsafe {
        for sig in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
            libc::signal(sig, on_terminate as *const () as libc::sighandler_t);
        }
    }
    std::thread::Builder::new()
        .name("rustcast-shutdown".to_string())
        .spawn(|| {
            loop {
                if SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
                    shutdown();
                    std::process::exit(0);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        })
        .ok();
}

/// Windows that can be locked onto: every normal window except RustCast's own.
pub fn recordable_windows() -> Vec<x11::ClientWindow> {
    x11::client_windows()
        .into_iter()
        .filter(|w| !is_rustcast_window(w))
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
    if let RecordTarget::Window { xid, .. } = &target
        && is_rustcast_xid(*xid)
    {
        return Err(OWN_WINDOW.to_string());
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
        // A Wayland full-screen stream is composed by the compositor, which
        // would put the pill in the video — stop with the hotkey instead.
        let portal_screen = matches!(target, RecordTarget::Portal(PortalSource::Monitor));
        let (label, sender, show) = (
            label.clone(),
            sender.clone(),
            cfg.show_indicator && !portal_screen,
        );
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
            if portal_screen {
                notify(
                    "Recording the screen",
                    "Press the recorder hotkey again to stop",
                );
            }
            tell_ui(&sender);
        }
    };

    std::thread::Builder::new()
        .name("rustcast-recorder".to_string())
        .spawn(move || {
            // A panic must not leave the recorder stuck "recording" or a
            // window ghosted: capture structs restore windows on unwind, and
            // the panic becomes an ordinary error here.
            let run = std::panic::AssertUnwindSafe(|| match &target {
                RecordTarget::Window { xid, .. } => {
                    record_window(*xid, &cfg, spec, stop_flag, scene, on_started)
                }
                RecordTarget::Monitor { rect, .. } => {
                    record_monitor(*rect, &cfg, spec, stop_flag, on_started)
                }
                RecordTarget::Portal(source) => {
                    portal::run(*source, &cfg, spec, stop_flag, on_started)
                }
            });
            let result = std::panic::catch_unwind(run).unwrap_or_else(|panic| {
                let msg = panic
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown error");
                Err(format!("recorder crashed: {msg}"))
            });
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
            let Some(child) = active.indicator.as_mut().filter(|c| c.id() == pid) else {
                return;
            };
            match child.try_wait() {
                Ok(None) => {}
                // Stop clicked (the pill exits 0).
                Ok(Some(status)) if status.success() => {
                    active.stop.store(true, Ordering::Relaxed);
                    return;
                }
                // The pill crashed or was killed: keep recording without it.
                _ => {
                    log::warn!("Recorder: indicator exited unexpectedly; recording continues");
                    active.indicator = None;
                    return;
                }
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
        if layers.iter().any(|l| l.xid == *xid) || is_rustcast_xid(*xid) {
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
    let mut pump = FramePump::new(fps);
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
                if cfg.keep_recording_when_minimized
                    && (cap.is_minimized() || cap.other_desktop().is_some())
                {
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
                    if cfg.keep_recording_when_minimized
                        && (layer.cap.is_minimized() || layer.cap.other_desktop().is_some())
                    {
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

        if let Err(e) = pump.emit(&mut stdin, &canvas) {
            failure = Some(e);
            break;
        }
        pump.wait();
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

/// Paces frames to ffmpeg: one frame per tick at most `fps` times a second.
/// ffmpeg stamps each frame with its arrival time and fills gaps (see
/// `encoder::output_args`), so the video lasts exactly as long as the
/// recording even when a capture is slow — nothing is sped up or dropped.
struct FramePump {
    started: Instant,
    frame_time: Duration,
    written: u64,
}

impl FramePump {
    fn new(fps: u32) -> Self {
        FramePump {
            started: Instant::now(),
            frame_time: Duration::from_secs_f64(1.0 / fps.max(1) as f64),
            written: 0,
        }
    }

    fn emit(&mut self, stdin: &mut impl Write, canvas: &Canvas) -> std::io::Result<()> {
        stdin.write_all(&canvas.data)?;
        self.written += 1;
        Ok(())
    }

    /// Sleep until the next frame is due (no sleep when running behind).
    fn wait(&self) {
        let next = self.frame_time.mul_f64(self.written as f64);
        if let Some(wait) = next.checked_sub(self.started.elapsed()) {
            std::thread::sleep(wait.min(self.frame_time));
        }
    }
}

/// Full-monitor recording on X11. RustCast's own windows are painted out of
/// every frame (see [`screen_capture`]).
fn record_monitor(
    rect: Rect,
    cfg: &RecorderConfig,
    spec: EncodeSpec,
    stop: Arc<AtomicBool>,
    on_started: impl FnOnce(),
) -> Result<(), String> {
    // Let the launcher finish hiding so the first (reference) frames are clean.
    std::thread::sleep(Duration::from_millis(250));
    let mut cap = screen_capture::ScreenCapture::new(rect)?;
    let (sw, sh) = cap.size();
    let (cw, ch) = if cfg.aspect_lock {
        cfg.output_size()
    } else {
        (even(sw), even(sh))
    };
    let mut canvas = Canvas::new(cw, ch);
    let fps = cfg.fps();

    // First frame before anything else appears (it seeds the clean background).
    let mut buf = Vec::new();
    cap.grab(&mut buf, cfg.show_cursor)?;
    blit_fit(&buf, sw, sh, sw as usize * frame::BPP, &mut canvas);

    let mut ffmpeg = Ffmpeg::spawn(
        encoder::raw_input_args(canvas.width, canvas.height, fps),
        encoder::output_args(&spec, None),
        Stdio::piped(),
    )?;
    let mut stdin = ffmpeg.child.stdin.take().ok_or("ffmpeg has no stdin")?;
    on_started();

    let mut pump = FramePump::new(fps);
    let mut failure = None;
    let mut warned = false;
    while !stop.load(Ordering::Relaxed) {
        match cap.grab(&mut buf, cfg.show_cursor) {
            Ok(()) => {
                warned = false;
                blit_fit(&buf, sw, sh, sw as usize * frame::BPP, &mut canvas)
            }
            // e.g. the monitor was unplugged: hold the last frame, log once.
            Err(e) if !warned => {
                warned = true;
                log::warn!("Recorder: {e}; repeating the last frame");
            }
            Err(_) => {}
        }
        if let Err(e) = pump.emit(&mut stdin, &canvas) {
            failure = Some(e);
            break;
        }
        pump.wait();
    }
    drop(stdin);
    drop(cap);
    let finished = ffmpeg.finish(Duration::from_secs(30));
    match (failure, finished) {
        (_, Err(e)) => Err(e),
        (Some(e), Ok(())) => Err(format!("writing frames failed: {e}")),
        (None, Ok(())) => Ok(()),
    }
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
    fn parses_parent_pid_even_with_odd_process_names() {
        assert_eq!(ppid_from_stat("1234 (bash) S 42 1234 1234 0"), Some(42));
        assert_eq!(
            ppid_from_stat("99 (my (weird) app) R 7 99 99 0 -1"),
            Some(7)
        );
        assert_eq!(ppid_from_stat("garbage"), None);
    }

    #[test]
    fn recognises_rustcast_windows_by_process_and_class() {
        let win = |pid: Option<u32>, class: &str| x11::ClientWindow {
            id: 1,
            title: "t".to_string(),
            class: class.to_string(),
            pid,
            minimized: false,
        };
        // Our own process (the launcher).
        assert!(is_rustcast_window(&win(Some(std::process::id()), "x")));
        // A helper process we spawned (REC pill / thumbnail).
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .unwrap();
        assert!(is_rustcast_window(&win(Some(child.id()), "x")));
        let _ = child.kill();
        let _ = child.wait();
        // Class match (GTK helpers are "rustcast Rustcast").
        assert!(is_rustcast_window(&win(None, "rustcast Rustcast")));
        // Anything else is fair game.
        assert!(!is_rustcast_window(&win(Some(1), "Navigator firefox")));
        assert!(!is_rustcast_window(&win(None, "")));
    }

    #[test]
    fn frame_pump_paces_to_the_frame_rate() {
        let canvas = Canvas::new(2, 2);
        let mut pump = FramePump::new(50);
        let mut sink = Vec::new();
        let start = Instant::now();
        for _ in 0..5 {
            pump.emit(&mut sink, &canvas).unwrap();
            pump.wait();
        }
        // One frame per emit, and 5 frames at 50 fps take ~100 ms.
        assert_eq!(sink.len(), 5 * canvas.data.len());
        let t = start.elapsed();
        assert!(
            t >= Duration::from_millis(80) && t < Duration::from_millis(400),
            "{t:?}"
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

/// Live end-to-end checks against a real X server. They create their own
/// windows, so they only need an X server with a window manager, e.g.
/// `Xvfb :99 & DISPLAY=:99 openbox &` then
/// `DISPLAY=:99 cargo test recorder::live -- --ignored --test-threads=1`
/// (build the binary first to also test the real REC pill).
#[cfg(test)]
mod live {
    use super::*;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        AtomEnum, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, PropMode, StackMode,
        WindowClass,
    };
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    const RED: u32 = 0xdd2222;
    const BLUE: u32 = 0x2233dd;
    const GREEN: u32 = 0x22cc44;

    struct Win<'a> {
        conn: &'a RustConnection,
        id: u32,
    }

    impl Drop for Win<'_> {
        fn drop(&mut self) {
            let _ = self.conn.destroy_window(self.id);
            let _ = self.conn.flush();
        }
    }

    /// A solid-colour top-level window with the given WM_CLASS, at (x, y).
    fn window<'a>(
        conn: &'a RustConnection,
        x: i16,
        y: i16,
        w: u16,
        h: u16,
        pixel: u32,
        class: &str,
    ) -> Win<'a> {
        let root = conn.setup().roots[0].root;
        let id = conn.generate_id().unwrap();
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            id,
            root,
            x,
            y,
            w,
            h,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new().background_pixel(pixel),
        )
        .unwrap();
        conn.change_property8(
            PropMode::REPLACE,
            id,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            format!("{class}\0{class}\0").as_bytes(),
        )
        .unwrap();
        // USPosition | USSize so the window manager honours x/y.
        let mut hints = [0u32; 18];
        hints[0] = 1 | 2;
        hints[1] = x as u32;
        hints[2] = y as u32;
        hints[3] = w as u32;
        hints[4] = h as u32;
        conn.change_property32(
            PropMode::REPLACE,
            id,
            AtomEnum::WM_NORMAL_HINTS,
            AtomEnum::WM_SIZE_HINTS,
            &hints,
        )
        .unwrap();
        conn.map_window(id).unwrap();
        conn.flush().unwrap();
        Win { conn, id }
    }

    fn raise(conn: &RustConnection, id: u32) {
        let _ = conn.configure_window(id, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        let _ = conn.flush();
    }

    /// Root position of a window's content.
    fn origin(conn: &RustConnection, id: u32) -> (i32, i32) {
        let root = conn.setup().roots[0].root;
        let t = conn
            .translate_coordinates(id, root, 0, 0)
            .unwrap()
            .reply()
            .unwrap();
        (t.dst_x as i32, t.dst_y as i32)
    }

    fn spec(out: &std::path::Path) -> EncodeSpec {
        EncodeSpec {
            fps: 15,
            audio: false,
            codec: encoder::detect_codec().unwrap(),
            output: out.to_path_buf(),
        }
    }

    fn cfg() -> RecorderConfig {
        RecorderConfig {
            aspect_lock: false,
            show_cursor: false,
            fps: 15,
            ..RecorderConfig::default()
        }
    }

    fn stop_after(ms: u64) -> Arc<AtomicBool> {
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(ms));
            s2.store(true, Ordering::Relaxed);
        });
        stop
    }

    fn duration(video: &std::path::Path) -> f64 {
        let out = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "csv=p=0",
            ])
            .arg(video)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    }

    /// RGB of pixel (x, y) in the last frame of `video`.
    fn last_frame_pixel(video: &std::path::Path, x: u32, y: u32) -> [u8; 3] {
        let img = video.with_extension("png");
        assert!(
            std::process::Command::new("ffmpeg")
                .args(["-loglevel", "error", "-sseof", "-0.3", "-i"])
                .arg(video)
                .args(["-frames:v", "1", "-y"])
                .arg(&img)
                .status()
                .unwrap()
                .success()
        );
        image::open(&img).unwrap().to_rgb8().get_pixel(x, y).0
    }

    fn close(p: [u8; 3], rgb: u32) -> bool {
        let want = [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8];
        p.iter().zip(want).all(|(a, b)| a.abs_diff(b) < 40)
    }

    /// Kills the helper process when the test ends (pass or fail).
    struct Pill(std::process::Child);

    impl Drop for Pill {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// The real REC pill (a RustCast helper process), if the binary is built.
    fn real_pill() -> Option<Pill> {
        let exe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/rustcast");
        exe.exists().then(|| {
            Pill(
                std::process::Command::new(exe)
                    .args(["--rec-indicator", "Test", "#000000", "#ffffff", "add"])
                    .env("GDK_BACKEND", "x11")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            )
        })
    }

    fn pill_window(pill: &Pill) -> Option<u32> {
        let child = &pill.0;
        for _ in 0..40 {
            if let Some(w) = x11::client_windows()
                .into_iter()
                .find(|w| w.pid == Some(child.id()))
            {
                return Some(w.id);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    #[test]
    #[ignore]
    fn locked_recording_never_contains_rustcast() {
        let (conn, _) = x11rb::connect(None).expect("needs DISPLAY");
        let base = window(&conn, 60, 60, 400, 300, RED, "lockedapp");
        let fake = window(&conn, 160, 140, 220, 160, GREEN, "rustcast");
        std::thread::sleep(Duration::from_millis(600));
        raise(&conn, fake.id);

        // The real pill, dragged right over the locked window.
        let pill = real_pill();
        if let Some(p) = pill.as_ref().and_then(pill_window) {
            let (bx, by) = origin(&conn, base.id);
            let _ = conn.configure_window(
                p,
                &ConfigureWindowAux::new()
                    .x(bx + 20)
                    .y(by + 20)
                    .stack_mode(StackMode::ABOVE),
            );
            let _ = conn.flush();
            assert!(is_rustcast_xid(p), "the REC pill must count as RustCast");
        }
        std::thread::sleep(Duration::from_millis(400));

        // Guards: never offered, never lockable.
        assert!(is_rustcast_xid(fake.id));
        assert!(!is_rustcast_xid(base.id));
        assert!(recordable_windows().iter().all(|w| w.id != fake.id));
        assert!(recordable_windows().iter().any(|w| w.id == base.id));
        let refused = start(
            RecordTarget::Window {
                xid: fake.id,
                title: "fake".to_string(),
            },
            RecorderConfig::default(),
            crate::config::Theme::default(),
            None,
        );
        assert_eq!(refused, Err(OWN_WINDOW.to_string()));

        // Record the base with the fake RustCast window even forced in as a
        // layer: it must not appear.
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("locked.mp4");
        let scene = Arc::new(Scene::default());
        scene
            .layers
            .lock()
            .unwrap()
            .push((fake.id, "fake".to_string()));
        record_window(base.id, &cfg(), spec(&out), stop_after(1500), scene, || {}).unwrap();

        // Probes (base-relative) under the fake window and under the pill.
        let (bx, by) = origin(&conn, base.id);
        let (fx, fy) = origin(&conn, fake.id);
        let under_fake = ((fx - bx + 60) as u32, (fy - by + 60) as u32);
        for (x, y) in [under_fake, (40, 30)] {
            let p = last_frame_pixel(&out, x, y);
            assert!(
                close(p, RED),
                "RustCast leaked into the locked recording at ({x},{y}): {p:?}"
            );
        }
        drop(pill);
    }

    #[test]
    #[ignore]
    fn full_screen_recording_paints_rustcast_out() {
        let (conn, screen) = x11rb::connect(None).expect("needs DISPLAY");
        let s = &conn.setup().roots[screen];
        let area = Rect {
            x: 0,
            y: 0,
            w: s.width_in_pixels as u32,
            h: s.height_in_pixels as u32,
        };
        let under = window(&conn, 80, 80, 500, 400, BLUE, "underapp");
        let fake = window(&conn, 200, 180, 240, 180, GREEN, "rustcast");
        std::thread::sleep(Duration::from_millis(600));
        raise(&conn, fake.id);
        std::thread::sleep(Duration::from_millis(300));

        // A second RustCast window that only appears *during* the recording
        // (like the REC pill or the launcher opened mid-recording).
        let late = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(900));
            let (conn, _) = x11rb::connect(None).unwrap();
            // Half over the blue window, half over bare desktop.
            let w = window(&conn, 10, 380, 300, 80, GREEN, "rustcast");
            raise(&conn, w.id);
            std::thread::sleep(Duration::from_millis(2500));
            drop(w);
        });

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("screen.mp4");
        record_monitor(area, &cfg(), spec(&out), stop_after(2600), || {}).unwrap();

        let (fx, fy) = origin(&conn, fake.id);
        let (ux, uy) = origin(&conn, under.id);
        // Middle of the RustCast window → the blue window it covers.
        let p = last_frame_pixel(&out, (fx + 120) as u32, (fy + 90) as u32);
        assert!(close(p, BLUE), "RustCast was not painted out: {p:?}");
        // The late RustCast window: over the blue window's lower part it
        // shows blue; over bare desktop it shows the desktop, never itself
        // (it must not have slipped into the clean background first).
        let l = last_frame_pixel(&out, (ux + 150) as u32, (uy + 340) as u32);
        assert!(close(l, BLUE), "late RustCast window leaked in: {l:?}");
        let bare = last_frame_pixel(&out, 30, (uy + 340) as u32);
        assert!(
            !close(bare, GREEN),
            "late RustCast window leaked in over the desktop: {bare:?}"
        );
        late.join().unwrap();
        // A part of the blue window RustCast doesn't cover is untouched.
        let q = last_frame_pixel(&out, (ux + 20) as u32, (uy + 300) as u32);
        assert!(close(q, BLUE), "{q:?}");
    }

    fn atom(conn: &RustConnection, name: &str) -> u32 {
        conn.intern_atom(false, name.as_bytes())
            .unwrap()
            .reply()
            .unwrap()
            .atom
    }

    fn cardinal(conn: &RustConnection, win: u32, prop: &str) -> Option<u32> {
        conn.get_property(false, win, atom(conn, prop), AtomEnum::ANY, 0, 1)
            .ok()?
            .reply()
            .ok()?
            .value32()?
            .next()
    }

    fn viewable(conn: &RustConnection, win: u32) -> bool {
        conn.get_window_attributes(win)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|a| a.map_state == x11rb::protocol::xproto::MapState::VIEWABLE)
    }

    /// Ask the window manager something about `win` (EWMH/ICCCM message).
    fn wm_message(conn: &RustConnection, win: u32, kind: &str, data: [u32; 5]) {
        let root = conn.setup().roots[0].root;
        let ev = x11rb::protocol::xproto::ClientMessageEvent::new(32, win, atom(conn, kind), data);
        conn.send_event(
            false,
            root,
            x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_REDIRECT
                | x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_NOTIFY,
            ev,
        )
        .unwrap();
        conn.flush().unwrap();
    }

    fn record_window_for(
        base: u32,
        ms: u64,
        layers: Vec<(u32, String)>,
        out: &std::path::Path,
    ) -> Result<(), String> {
        record_window_with(base, ms, layers, out, &cfg())
    }

    fn record_window_with(
        base: u32,
        ms: u64,
        layers: Vec<(u32, String)>,
        out: &std::path::Path,
        config: &RecorderConfig,
    ) -> Result<(), String> {
        let scene = Arc::new(Scene::default());
        *scene.layers.lock().unwrap() = layers;
        record_window(base, config, spec(out), stop_after(ms), scene, || {})
    }

    /// Whether any pixel in a 25×25 box around (x, y) of the last frame is
    /// clearly not `rgb` (i.e. something else — like a cursor — is there).
    fn anything_but(video: &std::path::Path, x: u32, y: u32, rgb: u32) -> bool {
        let img = video.with_extension("box.png");
        assert!(
            std::process::Command::new("ffmpeg")
                .args(["-loglevel", "error", "-sseof", "-0.3", "-i"])
                .arg(video)
                .args(["-frames:v", "1", "-y"])
                .arg(&img)
                .status()
                .unwrap()
                .success()
        );
        let frame = image::open(&img).unwrap().to_rgb8();
        (y.saturating_sub(12)..(y + 12).min(frame.height()))
            .flat_map(|py| {
                (x.saturating_sub(12)..(x + 12).min(frame.width())).map(move |px| (px, py))
            })
            .any(|(px, py)| !close(frame.get_pixel(px, py).0, rgb))
    }

    #[test]
    #[ignore]
    fn locked_recording_shows_only_the_window_not_foreign_pointer() {
        let (conn, _) = x11rb::connect(None).expect("needs DISPLAY");
        let root = conn.setup().roots[0].root;
        let base = window(&conn, 60, 60, 420, 320, RED, "lockedapp");
        let top = window(&conn, 260, 200, 220, 160, BLUE, "otherapp");
        std::thread::sleep(Duration::from_millis(600));
        raise(&conn, top.id);
        std::thread::sleep(Duration::from_millis(300));
        let (bx, by) = origin(&conn, base.id);
        let (tx, ty) = origin(&conn, top.id);
        let with_cursor = RecorderConfig {
            show_cursor: true,
            ..cfg()
        };
        let dir = tempfile::tempdir().unwrap();

        // Pointer over the window covering the locked one: the recording
        // must show only the locked window there — no pointer, no overlap.
        let (px, py) = (tx + 60, ty + 60);
        conn.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, px as i16, py as i16)
            .unwrap();
        conn.flush().unwrap();
        let covered = dir.path().join("covered.mp4");
        record_window_with(base.id, 1200, vec![], &covered, &with_cursor).unwrap();
        assert!(
            !anything_but(&covered, (px - bx) as u32, (py - by) as u32, RED),
            "something other than the locked window appeared where another window covers it"
        );

        // Pointer really over the locked window: it is part of the recording.
        let (qx, qy) = (bx + 60, by + 60);
        conn.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, qx as i16, qy as i16)
            .unwrap();
        conn.flush().unwrap();
        let visible = dir.path().join("visible.mp4");
        record_window_with(base.id, 1200, vec![], &visible, &with_cursor).unwrap();
        assert!(
            anything_but(&visible, (qx - bx) as u32, (qy - by) as u32, RED),
            "pointer over the locked window should be recorded"
        );
    }

    #[test]
    #[ignore]
    fn locked_window_ignores_overlap_and_layers_add_windows() {
        let (conn, _) = x11rb::connect(None).expect("needs DISPLAY");
        let base = window(&conn, 60, 60, 400, 300, RED, "lockedapp");
        let top = window(&conn, 200, 160, 240, 160, BLUE, "otherapp");
        std::thread::sleep(Duration::from_millis(600));
        raise(&conn, top.id);
        std::thread::sleep(Duration::from_millis(300));
        let (bx, by) = origin(&conn, base.id);
        let (tx, ty) = origin(&conn, top.id);
        let probe = ((tx - bx + 40) as u32, (ty - by + 40) as u32);

        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain.mp4");
        record_window_for(base.id, 1300, vec![], &plain).unwrap();
        let p = last_frame_pixel(&plain, probe.0, probe.1);
        assert!(close(p, RED), "an overlapping window leaked in: {p:?}");

        let layered = dir.path().join("layered.mp4");
        record_window_for(base.id, 1300, vec![(top.id, "top".into())], &layered).unwrap();
        let p = last_frame_pixel(&layered, probe.0, probe.1);
        assert!(close(p, BLUE), "the added window is missing: {p:?}");
    }

    #[test]
    #[ignore]
    fn minimized_locked_window_keeps_recording_and_is_restored() {
        let (conn, _) = x11rb::connect(None).expect("needs DISPLAY");
        let base = window(&conn, 60, 60, 400, 300, RED, "lockedapp");
        std::thread::sleep(Duration::from_millis(600));
        let id = base.id;
        let observed = std::thread::spawn(move || {
            let (conn, _) = x11rb::connect(None).unwrap();
            std::thread::sleep(Duration::from_millis(600));
            wm_message(&conn, id, "WM_CHANGE_STATE", [3, 0, 0, 0, 0]); // minimize
            std::thread::sleep(Duration::from_millis(1200));
            (
                viewable(&conn, id),
                cardinal(&conn, id, "_NET_WM_WINDOW_OPACITY"),
            )
        });
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("min.mp4");
        record_window_for(base.id, 2400, vec![], &out).unwrap();
        let (mapped_while_recording, opacity) = observed.join().unwrap();
        std::thread::sleep(Duration::from_millis(300));

        assert!(
            mapped_while_recording,
            "minimized window was not kept rendering"
        );
        assert_eq!(opacity, Some(0), "ghost should be invisible");
        assert_eq!(cardinal(&conn, id, "WM_STATE"), Some(3), "not re-minimized");
        assert_eq!(cardinal(&conn, id, "_NET_WM_WINDOW_OPACITY"), None);
        let p = last_frame_pixel(&out, 200, 150);
        assert!(close(p, RED), "recording went dark after minimize: {p:?}");
        // Real time: ~2.4 s recorded → ~2.4 s of video (not sped up).
        let d = duration(&out);
        assert!(
            (1.9..=3.0).contains(&d),
            "video lasts {d}s for a 2.4s recording"
        );
    }

    #[test]
    #[ignore]
    fn locked_window_keeps_recording_on_another_workspace() {
        let (conn, _) = x11rb::connect(None).expect("needs DISPLAY");
        let root = conn.setup().roots[0].root;
        if cardinal(&conn, root, "_NET_NUMBER_OF_DESKTOPS").unwrap_or(1) < 2 {
            eprintln!("window manager has a single workspace; skipping");
            return;
        }
        let base = window(&conn, 60, 60, 400, 300, RED, "lockedapp");
        std::thread::sleep(Duration::from_millis(600));
        let home = cardinal(&conn, base.id, "_NET_WM_DESKTOP").unwrap_or(0);
        let away = if home == 0 { 1 } else { 0 };
        let id = base.id;
        let observed = std::thread::spawn(move || {
            let (conn, _) = x11rb::connect(None).unwrap();
            let root = conn.setup().roots[0].root;
            std::thread::sleep(Duration::from_millis(600));
            wm_message(&conn, root, "_NET_CURRENT_DESKTOP", [away, 0, 0, 0, 0]);
            std::thread::sleep(Duration::from_millis(1200));
            let mapped = viewable(&conn, id);
            wm_message(&conn, root, "_NET_CURRENT_DESKTOP", [home, 0, 0, 0, 0]);
            mapped
        });
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("ws.mp4");
        record_window_for(base.id, 2600, vec![], &out).unwrap();
        let mapped_while_away = observed.join().unwrap();
        std::thread::sleep(Duration::from_millis(300));

        assert!(
            mapped_while_away,
            "window stopped rendering on another workspace"
        );
        assert_eq!(
            cardinal(&conn, id, "_NET_WM_DESKTOP"),
            Some(home),
            "window was not returned to its workspace"
        );
        assert_eq!(cardinal(&conn, id, "_NET_WM_WINDOW_OPACITY"), None);
        let p = last_frame_pixel(&out, 200, 150);
        assert!(close(p, RED), "{p:?}");
    }
}
