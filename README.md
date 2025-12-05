<!--
SPDX-FileCopyrightText: 2024 The rsinit Authors
SPDX-License-Identifier: GPL-2.0-only
-->

rsinit
======

[![REUSE status](https://api.reuse.software/badge/github.com/pengutronix/rsinit)](https://api.reuse.software/info/github.com/pengutronix/rsinit)

rsinit is a minimalistic single binary init for the initramfs for embedded
systems. It's main objective is to mount the root filesystem as fast as possible
and then hand over to the actual init program.

The main use-case is, to mount root filesystems that cannot be mounted by the
Linux kernel directly. But it can also mount just about anything that the kernel
can mount directly based on the command-line.

Currently supported root filesystems are:

 * 9pfs with USB gadget transport
 * filesystems on a dm-verity device

 * regular devices or anything that requires just specifying the device and
   mount options
 * nfsroot
 * 9pfs with virtio transport (for QEMU)

Design Choices
--------------

### Binary Size

One important design goal for rsinit is to be as fast as possible. For cases
where the kernel can mount the same root filesystem directly, booting with
rsinit should be just as fast as booting without it.

To achieve this, the initramfs must be as small as possible. Changes that
significantly increase the binary size of rsinit are not acceptable. Crates
that wrap external (C) libraries cannot be used.

In general, the binary size should be kept in mind when making any changes.

### Configuration and Features

rsinit itself will only cover the simple use-cases to mount the root
filesystem and related generic tasks with minimal runtime and build time
configuration.

For more complex use-cases, e.g. to mount additional (overlay) filesystems,
rsinit can be used as a crate in a custom rust application. The code is
structured in a way that makes it possible to reuse the existing code and add
new functionality as needed.

Cross compilation with cross.rs
-------------------------------

The project includes a Makefile for easy cross-compilation.

### Prerequisites

Before building, ensure you have the following tools installed:

- [`cross`](https://github.com/cross-rs/cross) - Cross-compilation tool for Rust
- [`rustup`](https://rustup.rs/) - Rust toolchain manager
- [`podman`](https://podman.io/) or [`docker`](https://www.docker.com/) - Container runtime for cross
- `cpio` - For creating cpio/initramfs archives
- `gzip` - For compressing cpio/initramfs archives
- A `nightly` Rust toolchain

You can verify all dependencies are installed by running:

```bash
make check-toolchain
```

### Usage

To see all available make targets and options:

```bash
make help
```

Common build commands:

```bash
# Build binaries and CPIO archives for all default targets
make all

# Build for a specific target only
make aarch64-unknown-linux-musl-build

# Create CPIO archive for a specific target
make aarch64-unknown-linux-musl-cpio

# Build with minimal profile (smaller binary size)
make MINIMAL_BUILD=1 all

# Use Docker instead of Podman
make CROSS_CONTAINER_ENGINE=docker all
```

Default target architectures:
- `aarch64-unknown-linux-musl`
- `arm-unknown-linux-musleabihf`
- `x86_64-unknown-linux-musl`

Build artifacts are placed in `target/<arch>/<profile>/` directories.
