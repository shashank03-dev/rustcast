//! OCR: read the text inside a screenshot — as prose, as code or as a table.
//!
//! Engine: the system `tesseract` binary, run as a short-lived child process.
//! Nothing is loaded into RustCast itself — the engine and its language model
//! only occupy memory for the fraction of a second the recognition takes and
//! are released as soon as it exits. That keeps the launcher's footprint at
//! zero for this feature, which is what low-end machines need.
//!
//! One run produces every word with its position (Tesseract's TSV output).
//! From that single result:
//! - [`Recognition::text`] rebuilds lines and paragraphs,
//! - [`Recognition::code`] rebuilds indentation and spacing from the word
//!   positions (Tesseract itself drops leading whitespace),
//! - [`Recognition::table`] finds columns from the empty vertical gutters and
//!   returns cells (copied as tab-separated values, which spreadsheets paste
//!   straight into cells, or saved as CSV).
//!
//! The crop is prepared so Tesseract does well on screen text (which is far
//! smaller and lower-DPI than the scans it is tuned for):
//! - converted to 8-bit grayscale (¼ of the RGBA size) and sent as PGM over a
//!   pipe — no PNG encoding, no temporary files;
//! - light-on-dark UI (dark themes, terminals) is inverted to dark-on-light;
//! - small crops are upscaled 2–3× so glyphs reach a size the model expects,
//!   with a cap so large crops never balloon in memory;
//! - a white margin is added (Tesseract misses text touching the border);
//! - `OMP_THREAD_LIMIT=1`: on small images threads only add memory and
//!   start-up cost.

use std::io::Write;
use std::process::{Command, Stdio};

use image::{GrayImage, RgbaImage};

/// Largest image (in pixels) handed to the engine after upscaling.
const MAX_PIXELS: u32 = 6_000_000;
const MARGIN: u32 = 12;

/// How the recognised text should be laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Text,
    Code,
    Table,
}

impl Layout {
    pub fn parse(s: &str) -> Option<Layout> {
        match s.to_ascii_lowercase().as_str() {
            "text" => Some(Layout::Text),
            "code" => Some(Layout::Code),
            "table" => Some(Layout::Table),
            _ => None,
        }
    }
}

pub fn tesseract_installed() -> bool {
    Command::new("tesseract")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Installed Tesseract language packs (e.g. `["eng", "hin"]`).
pub fn installed_languages() -> Vec<String> {
    Command::new("tesseract")
        .arg("--list-langs")
        .stderr(Stdio::null())
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .skip(1)
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty() && l != "osd")
                .collect()
        })
        .unwrap_or_default()
}

/// Keep only the requested languages that are installed; fall back to English
/// (or whatever is installed) so a typo in the config never breaks OCR.
fn pick_languages(requested: &str, installed: &[String]) -> String {
    let ok: Vec<&str> = requested
        .split('+')
        .map(str::trim)
        .filter(|l| installed.iter().any(|i| i == l))
        .collect();
    if !ok.is_empty() {
        return ok.join("+");
    }
    if installed.iter().any(|l| l == "eng") {
        return "eng".to_string();
    }
    installed
        .first()
        .cloned()
        .unwrap_or_else(|| "eng".to_string())
}

/// Grayscale, auto-invert, upscale and pad an image for recognition.
pub fn prepare(img: &RgbaImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut gray = GrayImage::new(w, h);
    let mut sum: u64 = 0;
    for (src, dst) in img.pixels().zip(gray.pixels_mut()) {
        let [r, g, b, _] = src.0;
        let l = ((u32::from(r) * 77 + u32::from(g) * 150 + u32::from(b) * 29) >> 8) as u8;
        sum += u64::from(l);
        dst.0[0] = l;
    }
    let mean = sum / u64::from((w * h).max(1));
    if mean < 110 {
        for p in gray.pixels_mut() {
            p.0[0] = 255 - p.0[0];
        }
    }

    // Screen text is ~10–16 px tall; Tesseract is happiest around 30+ px.
    let mut factor = if h <= 120 { 3 } else { 2 };
    while factor > 1 && (w * factor) * (h * factor) > MAX_PIXELS {
        factor -= 1;
    }
    let scaled = if factor > 1 {
        image::imageops::resize(
            &gray,
            w * factor,
            h * factor,
            image::imageops::FilterType::Triangle,
        )
    } else {
        gray
    };

    let (sw, sh) = scaled.dimensions();
    let mut padded = GrayImage::from_pixel(sw + 2 * MARGIN, sh + 2 * MARGIN, image::Luma([255]));
    image::imageops::replace(&mut padded, &scaled, i64::from(MARGIN), i64::from(MARGIN));
    padded
}

fn to_pgm(img: &GrayImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let mut out = format!("P5\n{w} {h}\n255\n").into_bytes();
    out.extend_from_slice(img.as_raw());
    out
}

/// One recognised word, in the coordinates of the prepared image.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub block: u32,
    pub par: u32,
    pub line: u32,
}

impl Word {
    fn right(&self) -> i32 {
        self.left + self.width
    }
    fn bottom(&self) -> i32 {
        self.top + self.height
    }
    fn center_y(&self) -> f64 {
        f64::from(self.top) + f64::from(self.height) / 2.0
    }
}

/// Everything one OCR run found.
#[derive(Debug, Clone, Default)]
pub struct Recognition {
    pub words: Vec<Word>,
}

/// Recognise the words in `img`. `languages` is a Tesseract language string
/// such as `eng` or `eng+hin`. `layout` is what the caller wants (if known):
/// code and tables are read as one uniform block, which keeps lone braces
/// and sparse cells that page segmentation tends to drop.
pub fn recognize(
    img: &RgbaImage,
    languages: &str,
    layout: Option<Layout>,
) -> Result<Recognition, String> {
    if !tesseract_installed() {
        return Err(missing_engine_message());
    }
    let langs = pick_languages(languages, &installed_languages());
    let pgm = to_pgm(&prepare(img));

    let run = |psm: &str| -> Result<Recognition, String> {
        let mut child = Command::new("tesseract")
            .args([
                "stdin", "stdout", "-l", &langs, "--psm", psm, "--dpi", "300", "tsv",
            ])
            .env("OMP_THREAD_LIMIT", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&pgm).map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(parse_tsv(&String::from_utf8_lossy(&out.stdout)))
    };

    match layout {
        Some(Layout::Code | Layout::Table) => run("6"),
        _ => {
            // Automatic page segmentation handles columns and mixed layouts.
            // A single block is better for tiny crops (nothing found) and for
            // code, which it reads with its punctuation-only lines.
            let r = run("3")?;
            if r.words.is_empty() || r.looks_like_code() {
                let block = run("6")?;
                if block.words.len() >= r.words.len() {
                    return Ok(block);
                }
            }
            Ok(r)
        }
    }
}

/// Parse Tesseract's TSV output (level 5 rows are words).
pub fn parse_tsv(tsv: &str) -> Recognition {
    let words = tsv
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.splitn(12, '\t').collect();
            if f.len() < 12 || f[0] != "5" {
                return None;
            }
            let text = f[11].trim();
            if text.is_empty() {
                return None;
            }
            let n = |i: usize| f[i].trim().parse::<i32>().ok();
            Some(Word {
                text: text.to_string(),
                block: n(2)? as u32,
                par: n(3)? as u32,
                line: n(4)? as u32,
                left: n(6)?,
                top: n(7)?,
                width: n(8)?,
                height: n(9)?,
            })
        })
        .collect();
    Recognition { words }
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

impl Recognition {
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Lines and paragraphs as Tesseract understood them.
    pub fn text(&self) -> String {
        let mut out = String::new();
        let mut prev: Option<&Word> = None;
        for w in &self.words {
            if let Some(p) = prev {
                if (p.block, p.par) != (w.block, w.par) {
                    out.push_str("\n\n");
                } else if p.line != w.line {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
            }
            out.push_str(&w.text);
            prev = Some(w);
        }
        out
    }

    /// Average character width (monospace advance for code).
    fn char_width(&self) -> f64 {
        let cw = median(
            self.words
                .iter()
                .filter(|w| w.text.chars().count() >= 2)
                .map(|w| f64::from(w.width) / w.text.chars().count() as f64)
                .collect(),
        );
        if cw > 0.0 {
            cw
        } else {
            median(
                self.words
                    .iter()
                    .map(|w| f64::from(w.height) * 0.55)
                    .collect(),
            )
            .max(1.0)
        }
    }

    fn line_height(&self) -> f64 {
        median(self.words.iter().map(|w| f64::from(w.height)).collect()).max(1.0)
    }

    /// Visual rows: words grouped by vertical position (independent of how
    /// Tesseract split blocks), each sorted left to right, top to bottom.
    fn rows(&self) -> Vec<Vec<&Word>> {
        let lh = self.line_height();
        let mut words: Vec<&Word> = self.words.iter().collect();
        words.sort_by(|a, b| a.center_y().total_cmp(&b.center_y()));
        let mut rows: Vec<(f64, Vec<&Word>)> = Vec::new();
        for w in words {
            match rows.last_mut() {
                Some((cy, row)) if (w.center_y() - *cy).abs() < lh * 0.5 => {
                    row.push(w);
                    *cy = row.iter().map(|w| w.center_y()).sum::<f64>() / row.len() as f64;
                }
                _ => rows.push((w.center_y(), vec![w])),
            }
        }
        rows.into_iter()
            .map(|(_, mut r)| {
                r.sort_by_key(|w| w.left);
                r
            })
            .collect()
    }

    /// Code: indentation and runs of spaces rebuilt from word positions,
    /// blank lines kept.
    pub fn code(&self) -> String {
        let cw = self.char_width();
        let lh = self.line_height();
        let rows = self.rows();
        let min_left = rows
            .iter()
            .filter_map(|r| r.first().map(|w| w.left))
            .min()
            .unwrap_or(0);
        let indents: Vec<usize> = rows
            .iter()
            .map(|r| (f64::from(r[0].left - min_left) / cw).round().max(0.0) as usize)
            .collect();
        // The indent unit (2, 4, …): the smallest indentation used.
        let unit = indents
            .iter()
            .copied()
            .filter(|i| *i >= 2)
            .min()
            .unwrap_or(0);
        let mut out = String::new();
        let mut prev_bottom: Option<i32> = None;
        for (ri, row) in rows.iter().enumerate() {
            let top = row.iter().map(|w| w.top).min().unwrap_or(0);
            if let Some(pb) = prev_bottom {
                let gap = f64::from(top - pb);
                let blanks = ((gap - lh * 0.6) / (lh * 1.4)).round().clamp(0.0, 2.0) as usize;
                for _ in 0..blanks {
                    out.push('\n');
                }
                out.push('\n');
            }
            out.push_str(&" ".repeat(snap_indent(indents[ri], unit)));
            for (i, w) in row.iter().enumerate() {
                if i > 0 {
                    // Ink boxes exclude side bearings, so a single space
                    // measures a bit more than one advance.
                    let gap = f64::from(w.left - row[i - 1].right()) / cw;
                    out.push_str(&" ".repeat((gap - 0.35).round().max(0.0) as usize));
                }
                out.push_str(&w.text);
            }
            prev_bottom = Some(row.iter().map(|w| w.bottom()).max().unwrap_or(top));
        }
        out
    }

    /// Table cells: rows by vertical position, columns from the gutters that
    /// no phrase crosses.
    pub fn table(&self) -> Vec<Vec<String>> {
        let cw = self.char_width();
        let rows = self.rows();
        // Words closer than ~2 characters belong to the same cell.
        let phrases: Vec<Vec<(i32, i32, String)>> = rows
            .iter()
            .map(|row| {
                let mut cells: Vec<(i32, i32, String)> = Vec::new();
                for w in row {
                    match cells.last_mut() {
                        Some((_, r, t)) if f64::from(w.left - *r) < cw * 1.8 => {
                            t.push(' ');
                            t.push_str(&w.text);
                            *r = w.right();
                        }
                        _ => cells.push((w.left, w.right(), w.text.clone())),
                    }
                }
                cells
            })
            .collect();

        // Column bands: union of overlapping phrase extents.
        let mut spans: Vec<(i32, i32)> =
            phrases.iter().flatten().map(|(l, r, _)| (*l, *r)).collect();
        spans.sort();
        let mut bands: Vec<(i32, i32)> = Vec::new();
        for (l, r) in spans {
            match bands.last_mut() {
                Some((_, br)) if l <= *br => *br = (*br).max(r),
                _ => bands.push((l, r)),
            }
        }

        phrases
            .into_iter()
            .map(|cells| {
                let mut out = vec![String::new(); bands.len()];
                for (l, r, t) in cells {
                    let mid = (l + r) / 2;
                    let col = bands
                        .iter()
                        .position(|(bl, br)| mid >= *bl && mid <= *br)
                        .unwrap_or(0);
                    if !out[col].is_empty() {
                        out[col].push(' ');
                    }
                    out[col].push_str(&t);
                }
                out
            })
            .collect()
    }

    /// Heuristic: indentation or code punctuation on many lines.
    pub fn looks_like_code(&self) -> bool {
        let code = self.code();
        let lines: Vec<&str> = code.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() < 2 {
            return false;
        }
        let codey = lines
            .iter()
            .filter(|l| {
                l.starts_with("  ")
                    || l.trim_end().ends_with(['{', '}', ';', ')', ','])
                    || [
                        "=>", "->", "::", "();", "==", "!=", "fn ", "def ", "let ", "const ", "$ ",
                    ]
                    .iter()
                    .any(|t| l.contains(t))
            })
            .count();
        codey * 10 >= lines.len() * 4
    }

    /// Heuristic: at least three rows with two or more aligned cells.
    pub fn looks_like_table(&self) -> bool {
        let t = self.table();
        let cols = t.first().map(Vec::len).unwrap_or(0);
        let multi = t
            .iter()
            .filter(|r| r.iter().filter(|c| !c.is_empty()).count() >= 2)
            .count();
        cols >= 2 && multi >= 3 && multi * 10 >= t.len() * 6
    }

    /// The best initial layout for this result.
    pub fn guess_layout(&self) -> Layout {
        if self.looks_like_code() {
            Layout::Code
        } else {
            Layout::Text
        }
    }

    pub fn render(&self, layout: Layout) -> String {
        match layout {
            Layout::Text => self.text(),
            Layout::Code => self.code(),
            Layout::Table => to_tsv(&self.table()),
        }
    }
}

/// Snap an indentation that is one column off a multiple of the indent unit
/// (glyph side-bearings differ between letters).
fn snap_indent(indent: usize, unit: usize) -> usize {
    if unit < 2 {
        return indent;
    }
    let nearest = ((indent as f64 / unit as f64).round() as usize) * unit;
    if nearest.abs_diff(indent) <= 1 {
        nearest
    } else {
        indent
    }
}

/// Tab-separated values — what spreadsheets expect on the clipboard.
pub fn to_tsv(rows: &[Vec<String>]) -> String {
    rows.iter()
        .map(|r| {
            r.iter()
                .map(|c| c.replace(['\t', '\n'], " "))
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// RFC 4180 CSV.
pub fn to_csv(rows: &[Vec<String>]) -> String {
    let field = |c: &String| {
        if c.contains([',', '"', '\n', '\r']) || c.starts_with(' ') || c.ends_with(' ') {
            format!("\"{}\"", c.replace('"', "\"\""))
        } else {
            c.clone()
        }
    };
    let mut out = rows
        .iter()
        .map(|r| r.iter().map(field).collect::<Vec<_>>().join(","))
        .collect::<Vec<_>>()
        .join("\r\n");
    out.push_str("\r\n");
    out
}

/// Something actionable found in recognised text.
#[derive(Debug, Clone, PartialEq)]
pub enum Smart {
    Link(String),
    Email(String),
    Phone(String),
    Color(String),
    /// An arithmetic expression and its value.
    Math(String, String),
}

/// Find links, e-mail addresses, phone numbers, hex colours and sums.
pub fn smart_actions(text: &str) -> Vec<Smart> {
    let mut out: Vec<Smart> = Vec::new();
    let mut push = |s: Smart| {
        if !out.contains(&s) && out.len() < 8 {
            out.push(s);
        }
    };
    let trim = |t: &str| {
        t.trim_matches(|c: char| {
            matches!(
                c,
                '(' | ')' | '[' | ']' | '<' | '>' | '"' | '\'' | ',' | ';' | '.' | ':' | '!' | '?'
            )
        })
        .to_string()
    };
    const TLDS: [&str; 16] = [
        "com", "org", "net", "io", "dev", "app", "ai", "co", "in", "uk", "de", "gov", "edu", "me",
        "rs", "info",
    ];
    for raw in text.split_whitespace() {
        let tok = trim(raw);
        if tok.len() < 4 {
            continue;
        }
        let lower = tok.to_ascii_lowercase();
        if let Some((user, domain)) = tok.split_once('@')
            && !user.is_empty()
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
            && !tok.contains('/')
        {
            push(Smart::Email(tok.clone()));
        } else if lower.starts_with("http://") || lower.starts_with("https://") {
            push(Smart::Link(tok.clone()));
        } else if lower.starts_with("www.") && tok.len() > 6 {
            push(Smart::Link(format!("https://{tok}")));
        } else if let Some(host) = lower.split('/').next()
            && host.contains('.')
            && !host.starts_with('.')
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            && host
                .rsplit('.')
                .next()
                .is_some_and(|tld| TLDS.contains(&tld))
        {
            push(Smart::Link(format!("https://{tok}")));
        } else if let Some(hex) = tok.strip_prefix('#')
            && (hex.len() == 6 || hex.len() == 3)
            && hex.chars().all(|c| c.is_ascii_hexdigit())
        {
            push(Smart::Color(format!("#{}", hex.to_ascii_uppercase())));
        }
    }

    // Phone numbers may contain spaces: scan runs of phone characters.
    let mut run = String::new();
    for c in text.chars().chain(std::iter::once('\n')) {
        if c.is_ascii_digit() || matches!(c, '+' | '-' | '(' | ')' | ' ') {
            run.push(c);
            continue;
        }
        let digits = run.chars().filter(char::is_ascii_digit).count();
        let candidate = run.trim().trim_matches(['-', ')']).trim().to_string();
        if (10..=15).contains(&digits)
            && (candidate.starts_with('+') || candidate.contains([' ', '-', '(']))
        {
            push(Smart::Phone(candidate));
        }
        run.clear();
    }

    // A single line that is a sum: "1,249.00 * 12".
    let t = text.trim();
    // Dates and ranges ("2024-01-05", "10-20") are not sums: a minus only
    // counts when it is spaced out.
    let has_operator = t
        .chars()
        .any(|c| matches!(c, '+' | '*' | '/' | '×' | '÷' | '^'))
        || t.contains(" - ");
    if !t.contains('\n') && has_operator && !t.contains("://") {
        let expr = t
            .replace(['×', 'x', 'X'], "*")
            .replace('÷', "/")
            .replace(',', "");
        if expr.chars().any(|c| c.is_ascii_digit())
            && let Ok(e) = crate::calculator::Expr::from_str(&expr)
            && let Some(v) = e.eval()
            && v.is_finite()
        {
            push(Smart::Math(
                t.to_string(),
                crate::unit_conversion::format_number(v),
            ));
        }
    }
    out
}

pub fn missing_engine_message() -> String {
    "Text recognition needs Tesseract, which is not installed.\n\n\
     Ubuntu / Debian:  sudo apt install tesseract-ocr\n\
     Fedora:           sudo dnf install tesseract\n\
     Arch:             sudo pacman -S tesseract tesseract-data-eng\n\n\
     Extra languages, e.g. Hindi:  sudo apt install tesseract-ocr-hin\n\
     then set OCR languages to eng+hin in RustCast Settings."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, left: i32, top: i32, block: u32, line: u32) -> Word {
        Word {
            text: text.to_string(),
            left,
            top,
            width: text.chars().count() as i32 * 10,
            height: 20,
            block,
            par: 1,
            line,
        }
    }

    #[test]
    fn language_selection_falls_back_gracefully() {
        let installed = vec!["eng".to_string(), "hin".to_string()];
        assert_eq!(pick_languages("eng+hin", &installed), "eng+hin");
        assert_eq!(pick_languages("deu+hin", &installed), "hin");
        assert_eq!(pick_languages("xyz", &installed), "eng");
        assert_eq!(pick_languages("xyz", &["fra".to_string()]), "fra");
    }

    #[test]
    fn tsv_is_parsed_into_words() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   1\t1\t0\t0\t0\t0\t0\t0\t100\t100\t-1\t\n\
                   5\t1\t1\t1\t1\t1\t10\t12\t50\t20\t96.1\tHello\n\
                   5\t1\t1\t1\t1\t2\t70\t12\t50\t20\t95.0\tworld\n\
                   5\t1\t1\t1\t1\t3\t130\t12\t5\t20\t10.0\t \n";
        let r = parse_tsv(tsv);
        assert_eq!(r.words.len(), 2);
        assert_eq!(r.text(), "Hello world");
    }

    #[test]
    fn text_keeps_lines_and_paragraphs() {
        let r = Recognition {
            words: vec![
                w("a", 0, 0, 1, 1),
                w("a2", 30, 0, 1, 1),
                w("b", 0, 40, 1, 2),
                w("c", 0, 100, 2, 1),
            ],
        };
        assert_eq!(r.text(), "a a2\nb\n\nc");
    }

    #[test]
    fn code_rebuilds_indentation_and_blank_lines() {
        // 10 px per character.
        let r = Recognition {
            words: vec![
                w("fn", 0, 0, 1, 1),
                w("main()", 30, 0, 1, 1),
                w("{", 100, 0, 1, 1),
                w("let", 40, 25, 2, 1),
                w("x", 80, 25, 2, 1),
                w("=", 100, 25, 2, 1),
                w("1;", 120, 25, 2, 1),
                w("}", 0, 75, 3, 1),
            ],
        };
        assert_eq!(r.code(), "fn main() {\n    let x = 1;\n\n}");
        assert!(r.looks_like_code());
    }

    #[test]
    fn code_indentation_snaps_to_its_unit() {
        assert_eq!(snap_indent(9, 4), 8);
        assert_eq!(snap_indent(3, 4), 4);
        assert_eq!(snap_indent(6, 4), 6);
        assert_eq!(snap_indent(5, 0), 5);
        // Words touching without a space stay together.
        let r = Recognition {
            words: vec![w("println!", 0, 0, 1, 1), w("(x);", 82, 0, 1, 1)],
        };
        assert_eq!(r.code(), "println!(x);");
    }

    #[test]
    fn table_columns_come_from_gutters() {
        let r = Recognition {
            words: vec![
                w("Name", 0, 0, 1, 1),
                w("Qty", 200, 0, 2, 1),
                w("Price", 300, 0, 3, 1),
                w("Blue", 0, 30, 1, 2),
                w("pen", 50, 30, 1, 2),
                w("2", 200, 30, 2, 2),
                w("1.50", 300, 30, 3, 2),
                w("Paper", 0, 60, 1, 3),
                w("10", 200, 60, 2, 3),
                w("4.00", 300, 60, 3, 3),
            ],
        };
        let t = r.table();
        assert_eq!(
            t,
            vec![
                vec!["Name", "Qty", "Price"],
                vec!["Blue pen", "2", "1.50"],
                vec!["Paper", "10", "4.00"],
            ]
        );
        assert!(r.looks_like_table());
        assert_eq!(to_tsv(&t).lines().nth(1), Some("Blue pen\t2\t1.50"));
    }

    #[test]
    fn csv_quotes_when_needed() {
        let rows = vec![vec![
            "a,b".to_string(),
            "say \"hi\"".to_string(),
            "x".to_string(),
        ]];
        assert_eq!(to_csv(&rows), "\"a,b\",\"say \"\"hi\"\"\",x\r\n");
    }

    #[test]
    fn empty_recognition_is_harmless() {
        let r = Recognition::default();
        assert_eq!(r.text(), "");
        assert_eq!(r.code(), "");
        assert!(r.table().is_empty());
        assert!(!r.looks_like_code() && !r.looks_like_table());
    }

    #[test]
    fn smart_actions_find_useful_things() {
        let s = smart_actions(
            "Mail hello@example.com or visit https://example.com, docs at example.org/x. \
             Call +91 98765 43210. Accent #0a84ff",
        );
        assert!(s.contains(&Smart::Email("hello@example.com".into())));
        assert!(s.contains(&Smart::Link("https://example.com".into())));
        assert!(s.contains(&Smart::Link("https://example.org/x".into())));
        assert!(s.contains(&Smart::Phone("+91 98765 43210".into())));
        assert!(s.contains(&Smart::Color("#0A84FF".into())));
        assert!(!s.iter().any(|x| matches!(x, Smart::Math(..))));

        assert_eq!(
            smart_actions("1,249.00 * 12"),
            vec![Smart::Math("1,249.00 * 12".into(), "14988".into())]
        );
        assert!(smart_actions("version 1.2.3 and file.txt").is_empty());
        assert!(smart_actions("2024-01-05").is_empty());
        assert!(smart_actions("12 - 5").contains(&Smart::Math("12 - 5".into(), "7".into())));
    }

    #[test]
    fn dark_text_backgrounds_are_inverted_and_small_crops_upscaled() {
        let img = RgbaImage::from_pixel(40, 20, image::Rgba([0, 0, 0, 255]));
        let g = prepare(&img);
        assert_eq!(g.dimensions(), (40 * 3 + 2 * MARGIN, 20 * 3 + 2 * MARGIN));
        assert!(g.pixels().all(|p| p.0[0] == 255));
    }

    #[test]
    fn huge_crops_are_not_upscaled() {
        let img = RgbaImage::from_pixel(3000, 2000, image::Rgba([255, 255, 255, 255]));
        let g = prepare(&img);
        assert_eq!(g.dimensions(), (3000 + 2 * MARGIN, 2000 + 2 * MARGIN));
    }
}
