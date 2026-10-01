.PHONY: help install restart reinstall
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

install:
	cargo install --path . --root "$(INSTALL_ROOT)"

restart:
	"$(ADJ)" server restart

# Sequenced by hand so that `make -j reinstall` does not restart before the install is done.
reinstall:
	"$(MAKE)" install
	"$(MAKE)" restart
