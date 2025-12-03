# SPDX-FileCopyrightText: 2025 The rsinit Authors
# SPDX-License-Identifier: GPL-2.0-only

CARGO_RUNNER			?= cross
CROSS_CONTAINER_ENGINE		 = podman
MINIMAL_BUILD			?= 0

ifneq ($(MINIMAL_BUILD),0)
	TARGET_PROFILE	:= minimal
	RUSTFLAGS		:= -Zfmt-debug=none -Zlocation-detail=none
	CARGO_FLAGS		:= -Z build-std=std,panic_abort -Z build-std-features=panic_immediate_abort
else
	TARGET_PROFILE	?= release
	RUSTFLAGS		?=
	CARGO_FLAGS		?=
endif

default-target-list	:= aarch64-unknown-linux-musl arm-unknown-linux-musleabihf x86_64-unknown-linux-musl

all: check-toolchain build cpio
build: $(addsuffix -build,$(default-target-list))
cpio: $(addsuffix -cpio,$(default-target-list))

help:
	@echo "rsinit cross compilation Makefile"
	@echo ""
	@echo "Available targets:"
	@echo "  help              - Show this help message"
	@echo "  check-toolchain   - Verify all required CLI tools are installed"
	@echo "  all               - Build and create CPIO archives for all default targets"
	@echo "  build             - Build binaries for all default targets"
	@echo "  cpio              - Create CPIO archives for all default targets"
	@echo "  <target>-build    - Build for a specific target (e.g., aarch64-unknown-linux-musl-build)"
	@echo "  <target>-cpio     - Create CPIO archive for a specific target"
	@echo ""
	@echo "Default targets: $(default-target-list)"
	@echo ""
	@echo "Configuration variables:"
	@echo "  CARGO_RUNNER           - Cargo wrapper for cross-compilation (default: $(CARGO_RUNNER))"
	@echo "  CROSS_CONTAINER_ENGINE - Container engine for cross (default: $(CROSS_CONTAINER_ENGINE))"
	@echo "  MINIMAL_BUILD          - Enable minimal build profile (default: $(MINIMAL_BUILD))"
	@echo "  TARGET_PROFILE         - Build profile to use (current: $(TARGET_PROFILE))"
	@echo ""
	@echo "Examples:"
	@echo "  make check-toolchain                    - Verify dependencies"
	@echo "  make all                                - Build everything"
	@echo "  make aarch64-unknown-linux-musl-build   - Build for aarch64 only"
	@echo "  make MINIMAL_BUILD=1 all                - Build with minimal profile"
	@echo "  make CROSS_CONTAINER_ENGINE=docker all  - Use Docker instead of Podman"

check-toolchain:
	@echo "Checking for required CLI tools..."
	@command -v $(CARGO_RUNNER) >/dev/null 2>&1 || { echo "Error: $(CARGO_RUNNER) is not installed. Please install it to continue."; exit 1; }
	@command -v rustup >/dev/null 2>&1 || { echo "Error: rustup is not installed. Please install it from https://rustup.rs/"; exit 1; }
	@command -v find >/dev/null 2>&1 || { echo "Error: find is not installed."; exit 1; }
	@command -v cpio >/dev/null 2>&1 || { echo "Error: cpio is not installed. Please install it using your package manager."; exit 1; }
	@command -v gzip >/dev/null 2>&1 || { echo "Error: gzip is not installed."; exit 1; }
	@command -v $(CROSS_CONTAINER_ENGINE) >/dev/null 2>&1 || { echo "Error: $(CROSS_CONTAINER_ENGINE) is not installed. Please install podman or set CROSS_CONTAINER_ENGINE to docker."; exit 1; }
	@rustup toolchain list | grep -q nightly || { echo "Error: nightly toolchain is not installed. Run: rustup toolchain install nightly"; exit 1; }
	@echo "All required tools are available!"

%-build: check-toolchain
	CROSS_CONTAINER_ENGINE=$(CROSS_CONTAINER_ENGINE) RUSTFLAGS="$(RUSTFLAGS)" $(CARGO_RUNNER) +nightly build --target $* $(if $(TARGET_PROFILE),--profile $(TARGET_PROFILE)) $(CARGO_FLAGS)

%-cpio: T=target/$*/$(if $(TARGET_PROFILE),$(TARGET_PROFILE),debug)
%-cpio: %-build
	cd $T && find init | cpio --create --format=newc > init-$*.cpio
	cd $T && gzip --keep --best --force init-$*.cpio

clean:
	cargo clean

.PHONY: all build clean cpio check-toolchain help
