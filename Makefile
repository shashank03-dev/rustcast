.PHONY: build release install install-start uninstall icon run test

build:
	cargo build

release:
	cargo build --release

# Build + install to ~/.local, add launcher, enable autostart.
install:
	./scripts/install.sh

# Same, but also launch RustCast immediately.
install-start:
	./scripts/install.sh --start

uninstall:
	./scripts/uninstall.sh

# Regenerate the brand SVGs, docs/icon.png and assets/icons/ (needs Playwright).
icon:
	python3 scripts/brand/build_logo.py && node scripts/brand/render.mjs

run:
	cargo run

test:
	cargo test
