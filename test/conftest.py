# SPDX-FileCopyrightText: 2026 The rsinit Authors
# SPDX-License-Identifier: GPL-2.0-only

import os
import shlex
import subprocess
from pathlib import Path

import pytest
from genimage import GenImage
from qemu import Qemu


class RsInit:
    def __init__(self, arch):
        self.arch = arch
        match arch:
            case "x86_64":
                target = "x86_64-unknown-linux-musl"
            case "aarch64":
                target = "aarch64-unknown-linux-musl"
            case _:
                raise NotImplementedError(f"support for architecture {arch} is missing")

        args = shlex.split(os.environ["CARGO"]) if "CARGO" in os.environ else ["cargo"]
        args += ["build", "--target", target, "--all-features", "--all-targets"]

        subprocess.check_call(args)
        self.build_path = Path(".") / "target" / target / "debug"
        self.rdinit_path = self.build_path / "init"
        self.init_path = self.build_path / "integration-test"


# TODO: aarch64 is implemented but there is no kernel image yet
@pytest.fixture(scope="session", params=["x86_64"])
def rsinit(request):
    return RsInit(request.param)


@pytest.fixture(scope="session")
def genimage(tmp_path_factory, rsinit):
    g = GenImage(tmp_path_factory.mktemp(f"genimage-{rsinit.arch}"))
    g.set_init(rsinit.init_path)
    g.set_rdinit(rsinit.rdinit_path)
    return g


@pytest.fixture
def qemu(tmp_path, rsinit):
    qemu = Qemu(tmp_path)
    qemu.set_arch(rsinit.arch)
    return qemu
