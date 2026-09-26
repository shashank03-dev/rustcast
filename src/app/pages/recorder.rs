//! The screen recorder page: what can be recorded, plus quick toggles.
//!
//! Rows are ordinary result [`App`]s, so the page looks and behaves exactly like
//! the rest of the launcher (arrow keys, Enter, typing to filter, theme).

use crate::app::apps::{App, AppCommand, ICNS_ICON, file_result_icon};
use crate::app::{Message, RecorderOption};
use crate::commands::Function;
use crate::config::RecorderConfig;
use crate::platform::linux::x11;
use crate::recorder::portal::{PortalSource, is_wayland_session};
use crate::recorder::{self, RecordTarget};
use crate::utils::icns_data_to_handle;

fn item(title: String, desc: String, cmd: AppCommand) -> App {
    App {
        ranking: 0,
        open_command: cmd,
        search_name: title.to_lowercase(),
        desc,
        icons: icns_data_to_handle(ICNS_ICON.to_vec()),
        display_name: title,
    }
}

fn clock(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
    } else {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    }
}

fn output_note(cfg: &RecorderConfig) -> String {
    if cfg.aspect_lock {
        let (w, h) = cfg.output_size();
        format!("video {w}×{h}")
    } else {
        "native size".to_string()
    }
}

fn toggle(opt: RecorderOption, cfg: &RecorderConfig, name: &str, on: &str, off: &str) -> App {
    let enabled = opt.get(cfg);
    item(
        format!("{name}: {}", if enabled { "On" } else { "Off" }),
        (if enabled { on } else { off }).to_string(),
        AppCommand::Message(Message::RecorderToggle(opt)),
    )
}

/// Rows shown while a recording runs: stop, the windows in the shot, and the
/// windows that can still be brought into it.
fn recording_rows(status: &recorder::Status, cfg: &RecorderConfig, rows: &mut Vec<App>) {
    let desc = match status.elapsed {
        Some(t) => format!("● REC {} — {}", clock(t.as_secs()), status.label),
        None => "Starting… pick what to share in the dialog".to_string(),
    };
    rows.push(item(
        "Stop Recording".to_string(),
        desc,
        AppCommand::Message(Message::RecorderStop),
    ));

    let Some(locked) = status.locked else {
        rows.push(item(
            "Everything on screen is being recorded".to_string(),
            "new windows appear in the video automatically".to_string(),
            AppCommand::Display,
        ));
        return;
    };

    for (xid, title) in &status.layers {
        rows.push(item(
            format!("Remove {title} from Recording"),
            "added window · take it out of the video".to_string(),
            AppCommand::Message(Message::RecorderRemoveWindow(*xid)),
        ));
    }
    let layout = if cfg.picture_in_picture {
        "as a corner tile"
    } else {
        "where you place it"
    };
    for w in recorder::recordable_windows() {
        if w.id == locked || status.layers.iter().any(|(id, _)| *id == w.id) {
            continue;
        }
        let title = recorder::window_label(&w);
        rows.push(item(
            format!("Add {title} to Recording"),
            format!("bring into the recording · {layout}"),
            AppCommand::Message(Message::RecorderAddWindow(w.id, title)),
        ));
    }
    rows.push(toggle(
        RecorderOption::PictureInPicture,
        cfg,
        "Picture-in-Picture",
        "added windows become corner tiles",
        "added windows show where they are",
    ));
}

/// Build the recorder page rows, filtered by `query_lc`.
pub fn recorder_rows(cfg: &RecorderConfig, query_lc: &str) -> Vec<App> {
    let mut rows = Vec::new();
    let note = output_note(cfg);

    if let Some(status) = recorder::status() {
        recording_rows(&status, cfg, &mut rows);
        return filter(rows, query_lc);
    }

    let wayland = is_wayland_session();
    if wayland {
        rows.push(item(
            "Record Full Screen".to_string(),
            format!("choose a monitor · {note}"),
            AppCommand::Message(Message::RecorderStart(RecordTarget::Portal(
                PortalSource::Monitor,
            ))),
        ));
        rows.push(item(
            "Record Any Window…".to_string(),
            format!("choose any window · {note}"),
            AppCommand::Message(Message::RecorderStart(RecordTarget::Portal(
                PortalSource::Window,
            ))),
        ));
    } else {
        let monitors = x11::monitors();
        let many = monitors.len() > 1;
        for m in monitors {
            let title = if !many {
                "Record Full Screen".to_string()
            } else if m.primary {
                format!("Record Full Screen ({})", m.name)
            } else {
                format!("Record Monitor {}", m.name)
            };
            rows.push(item(
                title,
                format!("{}×{} · {note}", m.rect.w, m.rect.h),
                AppCommand::Message(Message::RecorderStart(RecordTarget::Monitor {
                    name: m.name.clone(),
                    rect: m.rect,
                })),
            ));
        }
    }

    for w in recorder::recordable_windows() {
        let title = recorder::window_label(&w);
        let class = w.class.split_whitespace().last().unwrap_or("").to_string();
        let state = if w.minimized { " · minimized" } else { "" };
        rows.push(item(
            format!("Lock onto {title}"),
            format!("{class}{state} · overlaps never show"),
            AppCommand::Message(Message::RecorderStart(RecordTarget::Window {
                xid: w.id,
                title,
            })),
        ));
    }

    let (ow, oh) = cfg.output_size();
    rows.push(toggle(
        RecorderOption::AspectLock,
        cfg,
        "Aspect Lock",
        &format!("fixed {ow}×{oh} video, resizing is fine"),
        "video keeps the window's starting size",
    ));
    rows.push(toggle(
        RecorderOption::KeepWhenMinimized,
        cfg,
        "Keep Recording When Minimized",
        "minimized windows stay in the video",
        "minimizing freezes the last frame",
    ));
    rows.push(toggle(
        RecorderOption::ShowCursor,
        cfg,
        "Show Cursor",
        "pointer drawn into the video",
        "pointer hidden",
    ));
    rows.push(toggle(
        RecorderOption::RecordAudio,
        cfg,
        "Record Audio",
        "default audio input",
        "silent video",
    ));
    rows.push(toggle(
        RecorderOption::ShowIndicator,
        cfg,
        "Recording Indicator",
        "floating ● REC pill with Stop",
        "hidden · press the hotkey to stop",
    ));

    let dir = cfg.output_dir();
    let mut open_dir = item(
        "Open Recordings Folder".to_string(),
        cfg.output_dir.clone(),
        AppCommand::Function(Function::CreatePath {
            path: dir.to_string_lossy().to_string(),
            folder: true,
        }),
    );
    open_dir.icons = file_result_icon(true);
    rows.push(toggle(
        RecorderOption::PictureInPicture,
        cfg,
        "Picture-in-Picture",
        "added windows become corner tiles",
        "added windows show where they are",
    ));
    rows.push(open_dir);
    filter(rows, query_lc)
}

fn filter(rows: Vec<App>, query_lc: &str) -> Vec<App> {
    if query_lc.is_empty() {
        return rows;
    }
    rows.into_iter()
        .filter(|r| {
            r.display_name.to_lowercase().contains(query_lc)
                || r.desc.to_lowercase().contains(query_lc)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_include_toggles_and_filter_by_query() {
        let cfg = RecorderConfig::default();
        let all = recorder_rows(&cfg, "");
        assert!(all.iter().any(|r| r.display_name == "Aspect Lock: On"));
        assert!(
            all.iter()
                .any(|r| r.display_name == "Open Recordings Folder")
        );

        let filtered = recorder_rows(&cfg, "cursor");
        assert!(!filtered.is_empty());
        assert!(filtered.iter().all(|r| {
            r.display_name.to_lowercase().contains("cursor")
                || r.desc.to_lowercase().contains("cursor")
        }));
    }

    #[test]
    fn clock_formats() {
        assert_eq!(clock(61), "01:01");
        assert_eq!(clock(3661), "1:01:01");
    }
}
