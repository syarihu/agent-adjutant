.PHONY: help install restart reinstall
.DEFAULT_GOAL := help

# The binary `cargo install` just wrote, not whichever `adj` comes first in PATH: the restarted
# server is the binary that runs the restart. `make restart ADJ=adj` uses the one on PATH.
ADJ ?= $${CARGO_INSTALL_ROOT:-$${CARGO_HOME:-$$HOME/.cargo}}/bin/adj

help:
	@echo "make install    build and install adjutant and adj with cargo"
	@echo "make restart    restart the resident server (ADJ=... overrides the adj used)"
	@echo "make reinstall  install, then restart the server on the new binary"

install:
	cargo install --path .

restart:
	$(ADJ) server restart

# Sequenced by hand so that `make -j reinstall` does not restart before the install is done.
reinstall:
	"$(MAKE)" install
	"$(MAKE)" restart
