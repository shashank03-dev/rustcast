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

# Regenerate assets/icons/ from the RustCast logo (docs/icon.png)..
icon:
	python3 scripts/gen_icon.py

run:
	cargo run

test:
	cargo test
