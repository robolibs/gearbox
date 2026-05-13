SHELL := /bin/bash

PROJECT_NAME := $(shell sed -n '/^[[:space:]]*[^#\[[:space:]]/p' PROJECT | head -1 | tr -d '[:space:]')
PROJECT_VERSION := $(shell sed -n '/^[[:space:]]*[^#\[[:space:]]/p' PROJECT | sed -n '2p' | tr -d '[:space:]')
ifeq ($(PROJECT_NAME),)
    $(error Error: PROJECT file not found or invalid)
endif

TOP_DIR := $(CURDIR)
CARGO := cargo
# DISPLAY pins which X server receives the window (matches the Nvidia GL
# display when running inside WSL / multi-X setups). Override if you need
# `:0` or similar: `make run DISPLAY=:0`.
DISPLAY ?= :1
# Wrapper that forwards GPU/display access. `nixVulkan` = Bevy/wgpu path.
# Override with `make run RUN_WITH=nixGL` or `RUN_WITH=` for native.
RUN_WITH ?= nixVulkan

$(info ------------------------------------------)
$(info Project: $(PROJECT_NAME) v$(PROJECT_VERSION))
$(info ------------------------------------------)

.PHONY: build b compile c run r test t check fmt bench clean help h headless

build:
	@$(CARGO) build --bin gearbox

b: build

compile:
	@$(CARGO) clean
	@$(MAKE) build

c: compile

run:
	@if pkg-config --exists wayland-client 2>/dev/null; then \
		DISPLAY=$(DISPLAY) $(RUN_WITH) $(CARGO) run --bin gearbox; \
	else \
		nix develop --impure -c $(MAKE) run DISPLAY=$(DISPLAY) RUN_WITH="$(RUN_WITH)" CARGO="$(CARGO)"; \
	fi

r: run

headless:
	@$(CARGO) build -p gearbox-core -p gearbox-physics

test:
	@$(CARGO) test -p gearbox-physics --test headless

t: test

check:
	@$(CARGO) check --bin gearbox

fmt:
	@$(CARGO) fmt --all

bench:
	@$(CARGO) bench

clean:
	@$(CARGO) clean

help:
	@echo
	@echo "Usage: make [target]"
	@echo
	@echo "Available targets:"
	@echo "  build        Build the gearbox editor binary"
	@echo "  compile      Clean and rebuild"
	@echo "  run          Run the editor: DISPLAY=$(DISPLAY) $(RUN_WITH) cargo run --bin gearbox"
	@echo "  headless     Build the headless sim crates (gearbox-core + gearbox-physics)"
	@echo "  test         Run the headless smoke test"
	@echo "  check        Run cargo check on the binary"
	@echo "  fmt          Format the workspace"
	@echo "  bench        Run benchmarks"
	@echo "  clean        Remove Cargo build artifacts"
	@echo
	@echo "Examples:"
	@echo "  make run"
	@echo "  make run DISPLAY=:0           # target a different X server"
	@echo "  make run RUN_WITH=nixGL       # OpenGL wrapper instead of Vulkan"
	@echo "  make run RUN_WITH=            # no wrapper (native run)"
	@echo

h: help
