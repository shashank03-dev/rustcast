//! Model fallback for Jev: when the rule-based parser in [`crate::jev`] can
//! only guess "open <the whole sentence>", ask TypeSafe AI's Jev model on
//! Vercel AI Gateway (`POST /v1/evaluate`) which built-in action was meant.
//!
//! Jev is a decision model: it picks one option from a fixed set and returns
//! probabilities, it does not generate text. So it only covers actions that
//! need no name (tiling, show desktop, screenshot, recording, clipboard,
//! settings). An "open something by name" option lets real open requests fall
//! through to the parser untouched.
//!
//! Disabled unless an AI Gateway key is found in `AI_GATEWAY_API_KEY` or in
//! `~/.config/rustcast/ai_gateway_key`.

use std::time::Duration;

use log::{info, warn};
use serde_json::{Map, Value, json};

use crate::jev::Intent;
use crate::platform::window::TilePosition;

const ENDPOINT: &str = "https://ai-gateway.vercel.sh/v1/evaluate";
const MODEL: &str = "typesafe-ai/jev";
/// Minimum probability for the chosen action before it is shown.
const MIN_PROBABILITY: f64 = 0.6;
const TIMEOUT_SECS: u64 = 5;

/// The AI Gateway key, from the environment or the key file.
pub fn api_key() -> Option<String> {
    if let Ok(key) = std::env::var("AI_GATEWAY_API_KEY")
        && !key.trim().is_empty()
    {
        return Some(key.trim().to_string());
    }
    let path = dirs::home_dir()?.join(".config/rustcast/ai_gateway_key");
    let key = std::fs::read_to_string(path).ok()?;
    let key = key.trim();
    (!key.is_empty()).then(|| key.to_string())
}

/// Choice key, description, and the intent it maps to (`None` = no action).
fn options() -> Vec<(&'static str, &'static str, Option<Intent>)> {
    use TilePosition::*;
    let tile = |key, desc, pos| (key, desc, Some(Intent::Tile(pos)));
    let capture = |mode: &str| Intent::Capture {
        mode: mode.to_string(),
        delay: 0,
    };
    vec![
        tile(
            "tile_left_half",
            "Move the current window to the left half of the screen",
            LeftHalf,
        ),
        tile(
            "tile_right_half",
            "Move the current window to the right half of the screen",
            RightHalf,
        ),
        tile(
            "tile_top_half",
            "Move the current window to the top half of the screen",
            TopHalf,
        ),
        tile(
            "tile_bottom_half",
            "Move the current window to the bottom half of the screen",
            BottomHalf,
        ),
        tile(
            "tile_top_left",
            "Move the current window to the top left quarter",
            TopLeft,
        ),
        tile(
            "tile_top_right",
            "Move the current window to the top right quarter",
            TopRight,
        ),
        tile(
            "tile_bottom_left",
            "Move the current window to the bottom left quarter",
            BottomLeft,
        ),
        tile(
            "tile_bottom_right",
            "Move the current window to the bottom right quarter",
            BottomRight,
        ),
        tile(
            "tile_left_third",
            "Move the current window to the left third",
            LeftThird,
        ),
        tile(
            "tile_center_third",
            "Move the current window to the center third",
            CenterThird,
        ),
        tile(
            "tile_right_third",
            "Move the current window to the right third",
            RightThird,
        ),
        tile(
            "maximize",
            "Maximize the current window to fill the screen",
            Maximize,
        ),
        (
            "show_desktop",
            "Hide or minimize all windows to show the desktop",
            Some(Intent::ShowDesktop),
        ),
        ("screenshot", "Take a screenshot", Some(Intent::Screenshot)),
        (
            "screenshot_full_screen",
            "Take a screenshot of the whole screen",
            Some(capture("fullscreen")),
        ),
        (
            "copy_text_from_screen",
            "Copy or read text that is visible on the screen (OCR)",
            Some(capture("ocr")),
        ),
        (
            "copy_code_from_screen",
            "Copy code that is visible on the screen, keeping indentation",
            Some(capture("ocr-code")),
        ),
        (
            "copy_table_from_screen",
            "Copy a table on the screen into a spreadsheet",
            Some(capture("ocr-table")),
        ),
        (
            "pick_colors",
            "Pick colours or get a colour palette from the screen",
            Some(capture("palette")),
        ),
        (
            "compare_screenshots",
            "Compare two screenshots, before and after, find differences",
            Some(capture("compare")),
        ),
        (
            "record_screen",
            "Start recording the whole screen as a video",
            Some(Intent::Record(None)),
        ),
        (
            "stop_recording",
            "Stop the screen recording that is running",
            Some(Intent::StopRecording),
        ),
        (
            "clipboard",
            "Show clipboard history, things copied earlier",
            Some(Intent::Clipboard),
        ),
        (
            "settings",
            "Open RustCast settings or preferences",
            Some(Intent::Settings),
        ),
        (
            "open_by_name",
            "Open, launch, find or create a specific named file, folder, app or website",
            None,
        ),
        ("none", "None of these", None),
    ]
}

pub fn build_request(text: &str) -> Value {
    let mut criteria = Map::new();
    for (key, desc, _) in options() {
        criteria.insert(key.to_string(), Value::String(desc.to_string()));
    }
    json!({
        "model": MODEL,
        "state": text,
        "questions": {
            "action": {
                "type": "choice",
                "instructions": "Which desktop launcher action is the user asking for?",
                "criteria": criteria,
            }
        }
    })
}

/// The intent Jev chose, if it picked a real action with enough confidence.
pub fn parse_response(body: &Value) -> Option<(Intent, f64)> {
    let answer = body.get("answers")?.get("action")?;
    let choice = answer.get("choice")?.as_str()?;
    let probability = answer.get("probabilities")?.get(choice)?.as_f64()?;
    if probability < MIN_PROBABILITY {
        return None;
    }
    let (_, _, intent) = options().into_iter().find(|(key, _, _)| *key == choice)?;
    Some((intent?, probability))
}

/// Ask Jev which action `text` means. Blocking; call off the UI thread.
pub fn classify(text: &str, key: &str) -> Option<Intent> {
    let resp = minreq::post(ENDPOINT)
        .with_header("Authorization", format!("Bearer {key}"))
        .with_header("Content-Type", "application/json")
        .with_body(build_request(text).to_string())
        .with_timeout(TIMEOUT_SECS)
        .send();
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            warn!("Jev model request failed: {e}");
            return None;
        }
    };
    let body: Value = serde_json::from_str(resp.as_str().ok()?).ok()?;
    if !(200..300).contains(&resp.status_code) {
        warn!("Jev model returned {}: {body}", resp.status_code);
        return None;
    }
    let (intent, probability) = parse_response(&body)?;
    info!("Jev model chose {intent:?} ({probability:.2})");
    Some(intent)
}

/// Async wrapper that runs [`classify`] on the blocking pool.
pub async fn classify_async(text: String, key: String) -> Option<Intent> {
    tokio::time::timeout(
        Duration::from_secs(TIMEOUT_SECS + 1),
        tokio::task::spawn_blocking(move || classify(&text, &key)),
    )
    .await
    .ok()
    .and_then(|r| r.ok())
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(choice: &str, p: f64) -> Value {
        json!({ "answers": { "action": {
            "type": "choice",
            "choice": choice,
            "probabilities": { choice: p }
        }}})
    }

    #[test]
    fn request_lists_every_option() {
        let req = build_request("snap this left");
        assert_eq!(req["model"], MODEL);
        assert_eq!(req["state"], "snap this left");
        let criteria = req["questions"]["action"]["criteria"].as_object().unwrap();
        assert_eq!(criteria.len(), options().len());
        assert!(criteria.contains_key("tile_left_half"));
        assert!(criteria.contains_key("open_by_name"));
    }

    #[test]
    fn confident_action_maps_to_intent() {
        let (intent, p) = parse_response(&answer("tile_left_half", 0.97)).unwrap();
        assert_eq!(intent, Intent::Tile(TilePosition::LeftHalf));
        assert!((p - 0.97).abs() < 1e-9);
        assert_eq!(
            parse_response(&answer("record_screen", 0.8)).unwrap().0,
            Intent::Record(None)
        );
    }

    #[test]
    fn open_none_unknown_or_unsure_give_nothing() {
        assert!(parse_response(&answer("open_by_name", 0.99)).is_none());
        assert!(parse_response(&answer("none", 0.99)).is_none());
        assert!(parse_response(&answer("not_an_option", 0.99)).is_none());
        assert!(parse_response(&answer("maximize", 0.4)).is_none());
        assert!(parse_response(&json!({ "error": { "message": "nope" } })).is_none());
    }

    /// Hits the real gateway. Run with `cargo test -- --ignored jev_model`.
    #[test]
    #[ignore]
    fn live_gateway_classifies_phrasings() {
        let key = api_key().expect("no AI Gateway key configured");
        assert_eq!(
            classify("snap this window to the left side", &key),
            Some(Intent::Tile(TilePosition::LeftHalf))
        );
        assert_eq!(
            classify("hide everything so i can see my wallpaper", &key),
            Some(Intent::ShowDesktop)
        );
        assert_eq!(classify("open report.pdf in downloads", &key), None);
    }
}
