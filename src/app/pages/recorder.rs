//! The screen recorder page: what can be recorded, plus quick toggles.
//!
//! The page is built from ordinary result [`App`] rows (one list, one keyboard
//! order, typing filters it) and rendered by [`recorder_page`] as a layout made
//! for recording: a live banner while recording, screen and window cards, and
//! switches for the options.

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

// ----------------------------------------------------------------------------
// View: a purpose-built layout for the rows above. The row list stays the
// single source of truth (order = keyboard order, same wording); this only
// decides how each kind of row looks.
// ----------------------------------------------------------------------------

use std::time::Instant;

use iced::widget::text::Wrapping;
use iced::widget::{Scrollable, column, row, scrollable, text};
use iced::{Alignment, Element, Length};

use crate::app::pages::ui::{self, Tone};
use crate::app::tile::Motion;
use crate::config::Theme;

/// What a recorder row is, from the action it performs.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Stop,
    Info,
    Remove,
    Add,
    Screen,
    Window,
    Toggle(RecorderOption),
    Folder,
    Other,
}

fn kind_of(app: &App) -> Kind {
    match &app.open_command {
        AppCommand::Message(Message::RecorderStop) => Kind::Stop,
        AppCommand::Message(Message::RecorderRemoveWindow(_)) => Kind::Remove,
        AppCommand::Message(Message::RecorderAddWindow(..)) => Kind::Add,
        AppCommand::Message(Message::RecorderToggle(opt)) => Kind::Toggle(*opt),
        AppCommand::Message(Message::RecorderStart(t)) => match t {
            RecordTarget::Monitor { .. } | RecordTarget::Portal(PortalSource::Monitor) => {
                Kind::Screen
            }
            _ => Kind::Window,
        },
        AppCommand::Function(Function::CreatePath { .. }) => Kind::Folder,
        AppCommand::Display => Kind::Info,
        _ => Kind::Other,
    }
}

fn section_title(kind: Kind) -> Option<&'static str> {
    Some(match kind {
        Kind::Remove => "In this recording",
        Kind::Add => "Bring into the recording",
        Kind::Screen => "Screens",
        Kind::Window => "Windows",
        Kind::Toggle(_) => "Options",
        _ => return None,
    })
}

struct Ctx<'a> {
    theme: &'a Theme,
    motion: &'a Motion,
    focus: u32,
    now: Instant,
    cfg: &'a RecorderConfig,
}

impl Ctx<'_> {
    fn fade(&self, i: usize) -> f32 {
        ui::stagger(self.motion.page_since, self.now, i)
    }
    fn focus(&self, i: usize) -> f32 {
        self.motion.focus_amount(i as u32, self.focus, self.now)
    }
}

fn title_text(s: &str, size: f32, theme: &Theme, fade: f32) -> Element<'static, Message> {
    text(s.to_string())
        .size(size)
        .font(theme.font())
        .wrapping(Wrapping::None)
        .color(theme.text_color(fade))
        .into()
}

fn sub_text(s: &str, theme: &Theme, fade: f32) -> Element<'static, Message> {
    text(s.to_string())
        .size(12)
        .font(theme.font())
        .wrapping(Wrapping::None)
        .color(theme.text_color(0.5 * fade))
        .into()
}

fn press(app: &App, i: usize) -> Option<Message> {
    match app.open_command {
        AppCommand::Display => None,
        _ => Some(Message::OpenResult(i as u32)),
    }
}

/// A standard tile: glyph, title, subtitle, optional trailing element.
fn tile_card(
    c: &Ctx,
    i: usize,
    app: &App,
    symbol: &str,
    color: iced::Color,
    trailing: Option<Element<'static, Message>>,
    height: f32,
) -> Element<'static, Message> {
    let fade = c.fade(i + 1);
    let focus = c.focus(i);
    let mut content = row![
        ui::glyph(symbol, color, 38.0, fade),
        iced::widget::container(
            column![
                title_text(&app.display_name, 14.0, c.theme, fade),
                sub_text(&app.desc, c.theme, fade)
            ]
            .spacing(3),
        )
        .width(Length::Fill)
        .clip(true),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .padding([10, 12])
    .height(height);
    if let Some(t) = trailing {
        content = content.push(t);
    }
    ui::enter(
        ui::card_button(content, c.theme, focus, fade, press(app, i)).width(Length::Fill),
        fade,
    )
}

fn toggle_card(c: &Ctx, i: usize, app: &App, opt: RecorderOption) -> Element<'static, Message> {
    let on = opt.get(c.cfg);
    let knob = match c.motion.toggled {
        Some((o, at)) if o == opt => {
            let p = ui::progress(at, c.now, 0, 380, iced::animation::Easing::EaseOutBack);
            if on { p } else { 1.0 - p }
        }
        _ => {
            if on {
                1.0
            } else {
                0.0
            }
        }
    };
    let fade = c.fade(i + 1);
    let focus = c.focus(i);
    // "Aspect Lock: On" → name + state, same words.
    let (name, state) = app
        .display_name
        .rsplit_once(": ")
        .unwrap_or((app.display_name.as_str(), ""));
    let content = row![
        iced::widget::container(
            column![
                title_text(name, 13.0, c.theme, fade),
                sub_text(&app.desc, c.theme, fade)
            ]
            .spacing(3),
        )
        .width(Length::Fill)
        .clip(true),
        text(state.to_string())
            .size(11)
            .font(c.theme.font())
            .color(if on {
                ui::accent(fade)
            } else {
                c.theme.text_color(0.4 * fade)
            }),
        ui::switch(knob, c.theme, fade),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .padding([8, 12])
    .height(58);
    ui::enter(
        ui::card_button(content, c.theme, focus, fade, press(app, i)).width(Length::Fill),
        fade,
    )
}

/// The live banner while recording: pulsing dot, timer, big Stop.
fn stop_hero(c: &Ctx, i: usize, app: &App) -> Element<'static, Message> {
    let fade = c.fade(0);
    let focus = c.focus(i);
    let phase = c.motion.page_since.elapsed().as_secs_f32();
    let pulse = 0.55 + 0.45 * (phase * std::f32::consts::TAU / 1.4).cos();
    let status = app.desc.trim_start_matches("● ").to_string();
    let theme = c.theme.clone();
    let banner = row![
        text("●").size(22).color(ui::red(pulse * fade)),
        iced::widget::container(
            text(status)
                .size(18)
                .font(c.theme.font())
                .wrapping(Wrapping::None)
                .color(c.theme.text_color(fade)),
        )
        .width(Length::Fill)
        .clip(true),
        ui::pill_button(
            row![
                text("■").size(12),
                text(app.display_name.clone()).size(13).font(c.theme.font())
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            Tone::Danger,
            c.theme,
            press(app, i),
        )
        .padding([10, 18]),
    ]
    .spacing(14)
    .align_y(Alignment::Center);
    let ring = focus;
    ui::enter(
        iced::widget::container(banner)
            .padding([16, 18])
            .width(Length::Fill)
            .style(move |_| {
                let mut s = ui::panel_style(&theme, Some(ui::red(1.0)), fade);
                s.border.width = 1.0 + ring;
                s
            }),
        fade,
    )
}

fn info_banner(c: &Ctx, i: usize, app: &App) -> Element<'static, Message> {
    let fade = c.fade(i + 1);
    let theme = c.theme.clone();
    ui::enter(
        iced::widget::container(
            row![
                ui::glyph("◉", ui::green(1.0), 34.0, fade),
                column![
                    title_text(&app.display_name, 14.0, c.theme, fade),
                    sub_text(&app.desc, c.theme, fade)
                ]
                .spacing(3)
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .padding([12, 14])
        .width(Length::Fill)
        .style(move |_| ui::panel_style(&theme, Some(ui::green(1.0)), fade)),
        fade,
    )
}

fn folder_button(c: &Ctx, i: usize, app: &App) -> Element<'static, Message> {
    let fade = c.fade(i + 1);
    let focus = c.focus(i);
    let content = row![
        ui::glyph("▤", ui::accent(1.0), 30.0, fade),
        title_text(&app.display_name, 13.0, c.theme, fade),
        ui::spacer(),
        sub_text(&app.desc, c.theme, fade),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .padding([8, 12]);
    ui::enter(
        ui::card_button(content, c.theme, focus, fade, press(app, i)).width(Length::Fill),
        fade,
    )
}

fn render_item(c: &Ctx, i: usize, app: &App, kind: Kind) -> Element<'static, Message> {
    match kind {
        Kind::Stop => stop_hero(c, i, app),
        Kind::Info => info_banner(c, i, app),
        Kind::Folder => folder_button(c, i, app),
        Kind::Toggle(opt) => toggle_card(c, i, app, opt),
        Kind::Screen => tile_card(c, i, app, "▭", ui::accent(1.0), None, 68.0),
        Kind::Window => {
            let minimized = app.desc.contains("minimized");
            let badge =
                minimized.then(|| ui::badge("minimized", ui::green(1.0), c.theme, c.fade(i + 1)));
            tile_card(c, i, app, "◧", ui::accent(1.0), badge, 64.0)
        }
        Kind::Add => tile_card(c, i, app, "+", ui::green(1.0), None, 60.0),
        Kind::Remove => tile_card(c, i, app, "−", ui::red(1.0), None, 56.0),
        Kind::Other => tile_card(c, i, app, "•", ui::accent(1.0), None, 56.0),
    }
}

/// Whether a kind is laid out two-per-row.
fn in_grid(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Screen | Kind::Window | Kind::Add | Kind::Toggle(_)
    )
}

/// The recorder page body (between the search field and the footer).
pub fn recorder_page(
    rows: &[App],
    cfg: &RecorderConfig,
    focus: u32,
    theme: &Theme,
    motion: &Motion,
) -> Element<'static, Message> {
    let c = Ctx {
        theme,
        motion,
        focus,
        now: Instant::now(),
        cfg,
    };

    if rows.is_empty() {
        return iced::widget::container(sub_text("No results found", theme, 1.0))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
    }

    // Group consecutive rows by section, keeping their order (= keyboard order).
    let mut blocks: Vec<Element<'static, Message>> = Vec::new();
    let mut i = 0;
    let mut last_title: Option<&str> = None;
    while i < rows.len() {
        let kind = kind_of(&rows[i]);
        let section = |k: Kind| section_title(k).map(|_| std::mem::discriminant(&k));
        if let Some(title) = section_title(kind)
            && last_title != Some(title)
        {
            blocks.push(ui::enter(
                ui::section_label(title, theme, c.fade(i)),
                c.fade(i),
            ));
            last_title = Some(title);
        }
        if in_grid(kind) {
            // Collect the run of same-section items and lay them out 2-up.
            let start = i;
            while i < rows.len() && section(kind_of(&rows[i])) == section(kind) {
                i += 1;
            }
            let items: Vec<usize> = (start..i).collect();
            for pair in items.chunks(2) {
                let mut r = row![].spacing(10);
                for &j in pair {
                    r = r.push(
                        iced::widget::container(render_item(&c, j, &rows[j], kind_of(&rows[j])))
                            .width(Length::FillPortion(1)),
                    );
                }
                // A lone screen or add-card spans the row; options keep the grid.
                if pair.len() == 1 && matches!(kind, Kind::Toggle(_) | Kind::Window) {
                    r = r.push(iced::widget::Space::new().width(Length::FillPortion(1)));
                }
                blocks.push(r.into());
            }
        } else {
            blocks.push(render_item(&c, i, &rows[i], kind));
            i += 1;
        }
    }

    Scrollable::with_direction(
        column(blocks).spacing(10).padding([14, 16]),
        scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(3).scroller_width(3)),
    )
    .id("results")
    .height(Length::Fill)
    .into()
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
