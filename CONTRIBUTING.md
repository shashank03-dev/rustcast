# Contributing to RustCast

Thanks for helping make RustCast better! Bug reports, ideas, docs fixes and code are
all welcome.

## Ways to help

- **Report a bug** with the [bug form](https://github.com/shashank03-dev/rustcast/issues/new?template=bug_report.yml).
  Your distro, desktop and session type (X11 or Wayland) matter a lot on Linux.
- **Suggest a feature** with the [feature form](https://github.com/shashank03-dev/rustcast/issues/new?template=feature_request.yml).
- **Pick up an issue** labelled `good first issue` or `help wanted`. Leave a comment so
  nobody else starts on the same thing.
- **Test on your desktop.** KDE, Xfce, Cinnamon and other distros need more testing, and
  a short report is genuinely useful.

## Getting set up

```sh
git clone https://github.com/shashank03-dev/rustcast
cd rustcast
sudo apt install libgtk-3-dev libxcb1-dev libxtst-dev tesseract-ocr ffmpeg   # see README for other distros
cargo run
```

Logs go to `/tmp/rustcast.log` in release builds and to the terminal in debug builds.

## Before you open a pull request

1. `cargo fmt --all`
2. `cargo clippy --all-targets` with no new warnings
3. `cargo test`
4. Update the README or `docs/` if you changed behaviour, hotkeys or settings.
5. For anything visible, add a screenshot or short recording to the PR.

Keep pull requests focused: one fix or feature per PR is much easier to review.

## Code guidelines

- Code must compile, be formatted with `cargo fmt`, and do what it says.
- No code that harms someone's device, data or privacy. Anything that goes online must
  be opt-in and documented in the README's Privacy section.
- Prefer small, readable functions and comments that explain *why*.
- Heavy work (OCR, image editing, encoding) belongs in the short-lived helper processes
  (`--snap`, `--rec-indicator`, `--overlay`), so the launcher stays light.
- Using AI tools is fine, but you are responsible for every line: read it, test it, and
  write the PR description yourself.

## Project layout

```
src/
├── app/               launcher window, pages (clipboard, recorder, settings, emoji), tray menu
│   ├── tile/          the launcher's state and update loop
│   └── pages/         full-window pages
├── snap/              screenshot overlay, editor, OCR, palette, compare, pin
├── recorder/          X11 + portal capture, encoder, ● REC pill
├── platform/linux/    desktop entries, hotkeys, window management, overlays
├── jev.rs             the Jev command parser
├── calculator.rs      maths
├── unit_conversion.rs unit conversion
├── clipboard.rs       clipboard history storage
├── config.rs          config file schema and defaults
└── main.rs            entry point and helper-process dispatch
docs/                  example and default config files
assets/                icons and brand files
scripts/               install, uninstall, release packaging, logo generation
launch-video/          scripts and Remotion project for the launch video
```

## Releases

Releases are automatic. Bump `version` in `Cargo.toml`, update `CHANGELOG.md` and
`.github/release-notes.md`, and merge to `main`. The Release workflow builds the
binary, tags `v<version>` and publishes the download.

## Code of conduct

Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
