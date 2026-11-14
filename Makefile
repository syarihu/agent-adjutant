.PHONY: help install restart reinstall check app app-install
.DEFAULT_GOAL := help

# One root for both install and restart, so restart runs the binary install just wrote and not
# whichever `adj` comes first in PATH: the restarted server is the binary that runs the restart.
# Cargo's `install.root` config is not read here; pass `INSTALL_ROOT=...` instead, or
# `make restart ADJ=adj` for the one on PATH.
INSTALL_ROOT ?= $${CARGO_INSTALL_ROOT:-$${CARGO_HOME:-$$HOME/.cargo}}
ADJ ?= $(INSTALL_ROOT)/bin/adj

help:
	@echo "make install    build and install adjutant and adj with cargo"
	@echo "make restart    restart the resident server (INSTALL_ROOT=<dir> or ADJ=<path> picks the adj)"
	@echo "make reinstall  install, then restart the server on the new binary"
	@echo "make check      everything a PR has to pass"
	@echo "make app        build the macOS app (app/src-tauri; needs: cargo install tauri-cli --version '^2' --locked)"
	@echo "make app-install  copy the built app to /Applications"

install:
	cargo install --path . --root "$(INSTALL_ROOT)"

restart:
	"$(ADJ)" server restart

# Sequenced by hand so that `make -j reinstall` does not restart before the install is done.
reinstall:
	"$(MAKE)" install
	"$(MAKE)" restart

check:
	./scripts/check-layering.sh
	./scripts/test-check-move-only.sh
	for f in src/ui/*.js; do node --check "$$f" || exit 1; done
	node --test src/ui/tests/*.test.js
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	./scripts/check-test-leaks.sh cargo test

# The macOS app is its own crate, outside the root package, so none of the targets above build it.
APP_DIR := app/src-tauri
APP_BUNDLE := $(APP_DIR)/target/release/bundle/macos/Adjutant.app

app:
	cd "$(APP_DIR)" && cargo tauri build --bundles app

app-install: app
	rm -rf /Applications/Adjutant.app
	cp -R "$(APP_BUNDLE)" /Applications/Adjutant.app
