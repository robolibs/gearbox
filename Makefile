SHELL := /bin/bash

PROJECT_NAME := $(shell grep '^name =' bin/gearbox/Cargo.toml | head -n 1 | cut -d '"' -f 2)
PROJECT_VERSION := $(shell grep '^version =' Cargo.toml | head -n 1 | cut -d '"' -f 2)
ifeq ($(PROJECT_NAME),)
    $(error Error: package name not found in bin/gearbox/Cargo.toml)
endif

TOP_DIR := $(CURDIR)
CARGO := cargo
NIX := nix develop --impure -c
CARGO_ENV := $(NIX) $(CARGO)
TARGET ?=
TARGET_ARG := $(if $(TARGET),--target $(TARGET),)
LOCKED ?=
RUN_WITH ?= nixVulkan
RUN_ARGS ?=
# Bevy/wgpu can panic on NVIDIA + Wayland when acquiring the swapchain
# texture (`SurfaceError::Timeout`). X11/Xwayland is the stable default here;
# override with `BACKEND=wayland` if you explicitly want native Wayland.
BACKEND ?= x11

HAS_REL := $(shell command -v git-rel 2>/dev/null)

$(info ------------------------------------------)
$(info Project: $(PROJECT_NAME) v$(PROJECT_VERSION))
$(info Display: $(BACKEND) backend)
$(info ------------------------------------------)

.PHONY: build b build-release-bin build-bins compile c run r sim cli test t check fmt bench clean bind bind-c bind-py help h

build:
	@$(CARGO_ENV) build --lib

b: build

build-release-bin:
	@$(CARGO_ENV) build --release $(LOCKED) -p gearbox-sim --bin gearbox-sim -p gearbox-cli --bin gearbox $(TARGET_ARG)

build-bins:
	@$(CARGO_ENV) build -p gearbox-sim --bin gearbox-sim -p gearbox-cli --bin gearbox

compile:
	@$(CARGO) clean
	@$(MAKE) build

c: compile

run: build-bins
	@WINIT_UNIX_BACKEND=$(BACKEND) $(NIX) $(RUN_WITH) target/debug/gearbox run --sim target/debug/gearbox-sim $(RUN_ARGS)

r: run

sim: build-bins
	@WINIT_UNIX_BACKEND=$(BACKEND) $(NIX) $(RUN_WITH) target/debug/gearbox-sim $(RUN_ARGS)

cli:
	@$(CARGO_ENV) run -q -p gearbox-cli --bin gearbox -- $(RUN_ARGS)

test:
	@$(CARGO_ENV) test --all-targets

t: test

check:
	@$(CARGO_ENV) check --all-targets

fmt:
	@$(CARGO_ENV) fmt --all

clean:
	@$(CARGO) clean

bind: bind-c bind-py

bind-c:
	@$(CARGO_ENV) build --lib
	@cbindgen --config cbindgen.toml --crate $(PROJECT_NAME) \
		--output include/$(PROJECT_NAME).h

bind-py:
	@maturin build --features python

docs:
	@command -v mdbook >/dev/null 2>&1 || { echo "mdbook is not installed. Please install it first."; exit 1; }
	@mdbook build $(TOP_DIR)/book --dest-dir $(TOP_DIR)/docs
	@git add --all && git commit -m "docs: building website/mdbook"

release:
	@if [ -z "$(HAS_REL)" ]; then \
		echo "git-rel is not installed. Please install it first."; \
		exit 1; \
	fi
	@if [ -z "$(TYPE)" ]; then \
		echo "Release type not specified. Use 'make release TYPE=[patch|minor|major|m.m.p]'"; \
		exit 1; \
	fi
	@git rel $(TYPE)

help:
	@echo
	@echo "Usage: make [target]"
	@echo
	@echo "Available targets:"
	@echo "  build        Build the library"
	@echo "  build-bins   Build the gearbox CLI and gearbox-sim (debug)"
	@echo "  build-release-bin"
	@echo "               Build the release gearbox and gearbox-sim binaries"
	@echo "  compile      Clean and rebuild"
	@echo "  run          Launch the simulator through the CLI ($(BACKEND) backend, $(RUN_WITH) wrapper)"
	@echo "  sim          Launch gearbox-sim directly"
	@echo "  cli          Run the gearbox CLI: make cli RUN_ARGS='instance list'"
	@echo "  test         Run all tests"
	@echo "  bind         Generate both C and Python bindings"
	@echo "  check        Run cargo check on all targets"
	@echo "  fmt          Format the workspace"
	@echo "  clean        Remove Cargo build artifacts"
	@echo "  docs         Build the documentation"
	@echo "  release      Release a new version"
	@echo
	@echo "Examples:"
	@echo "  make run"
	@echo "  make run"
	@echo "  make run RUN_ARGS='bin/gearbox/assets/oxbo.usd'"
	@echo "  make run BACKEND=x11"
	@echo "  make run RUN_WITH=nixGL"
	@echo "  make run RUN_WITH="
	@echo

h: help
