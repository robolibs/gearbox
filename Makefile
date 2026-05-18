SHELL := /bin/bash

PROJECT_NAME := $(shell sed -n '/^[[:space:]]*[^#\[[:space:]]/p' PROJECT | head -1 | tr -d '[:space:]')
PROJECT_VERSION := $(shell sed -n '/^[[:space:]]*[^#\[[:space:]]/p' PROJECT | sed -n '2p' | tr -d '[:space:]')
ifeq ($(PROJECT_NAME),)
    $(error Error: PROJECT file not found or invalid)
endif

TOP_DIR := $(CURDIR)
CARGO := cargo
BACKEND ?= x11
DISPLAY ?= :1
RUN_WITH ?= nixVulkan

$(info ------------------------------------------)
$(info Project: $(PROJECT_NAME) v$(PROJECT_VERSION))
$(info Display: $(BACKEND) backend)
$(info ------------------------------------------)

.PHONY: build b compile c run r headless test t check fmt bench clean help h

build:
	@$(CARGO) build --bin gearbox

b: build

compile:
	@$(CARGO) clean
	@$(MAKE) build

c: compile

run:
	@$(RUN_WITH) $(CARGO) run --bin gearbox

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
	@echo "  run          Run the editor ($(BACKEND) backend, $(RUN_WITH) wrapper)"
	@echo "  headless     Build the headless sim crates (gearbox-core + gearbox-physics)"
	@echo "  test         Run the headless smoke test"
	@echo "  check        Run cargo check on the binary"
	@echo "  fmt          Format the workspace"
	@echo "  bench        Run benchmarks"
	@echo "  clean        Remove Cargo build artifacts"
	@echo
	@echo "Examples:"
	@echo "  make run"
	@echo "  make run BACKEND=x11          # force X11 / XWayland (.envrc auto-detects)"
	@echo "  make run BACKEND=wayland      # force native Wayland"
	@echo "  make run DISPLAY=:0           # target a different X server (BACKEND=x11)"
	@echo "  make run RUN_WITH=nixGL       # OpenGL wrapper instead of Vulkan"
	@echo "  make run RUN_WITH=            # no wrapper (native run)"
	@echo

h: help
