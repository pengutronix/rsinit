#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 The rsinit Authors
# SPDX-License-Identifier: GPL-2.0-only


import hashlib
import json
import os
import subprocess
import tempfile
import urllib.request
from pathlib import Path

# this kernel was created for rauc testing. It has the required features, so use it for now
KERNEL_X86_64_URL = (
    "https://github.com/jluebbe/linux/releases/download/rauc-test-20241015-1/bzImage"
)
KERNEL_X86_64_SHA256 = (
    "1d645700187640c58ea3821e55723cfe1627b68fcf0f38706d7bdd93c71fa056"
)

KERNEL_AARCH64_URL = None
KERNEL_AARCH64_SHA256 = None


class RuntimeStatus:
    def __init__(self, result):
        result = [json.loads(block) for block in result.split("\0") if block]
        if (
            len(result) > 0
            and isinstance(result[-1], dict)
            and "mountinfo" in result[-1]
        ):
            self._system_status = result.pop(-1)
            self.mountinfo = self._system_status["mountinfo"]
            self.block_devices = self._system_status.get("block-devices")
        else:
            self._system_status = None
            self.mountinfo = None
            self.block_devices = None
        self.rsinit_messages = result

    def assert_system_state(self):
        assert self.mountinfo, "Missing /proc/self/mountinfo data"
        assert self.block_devices, "Missing /sys/dev/block data"

    def get_mount(self, *, mount_point=None, filesystem_type=None):
        assert self.mountinfo, "missing mountinfo data"
        mounts = [
            mount
            for mount in self.mountinfo
            if (not mount_point or mount_point == mount["mount-point"])
            and (not filesystem_type or filesystem_type == mount["filesystem-type"])
        ]
        if len(mounts) > 1:
            raise AssertionError(
                f"multiple mounts found for mount_point={mount_point} and filesystem_type={filesystem_type}"
            )
        if len(mounts) == 0:
            raise AssertionError(
                f"no mount found for mount_point={mount_point} and filesystem_type={filesystem_type}"
            )
        return mounts[0]


class Qemu:
    def __init__(self, tmp_path=None):
        self.__arch = "x86_64"
        self.__cmdline = ""
        self.__initramfs = None
        self.__diskimage = None
        self.__9p_root = None
        self.__tmp_path = tmp_path

    def set_arch(self, arch):
        self.__arch = arch

    def set_cmdline(self, cmdline):
        self.__cmdline = cmdline

    def set_initramfs(self, initramfs):
        self.__initramfs = initramfs

    def set_diskimage(self, diskimage):
        self.__diskimage = diskimage

    def set_9p_root(self, root):
        self.__9p_root = root

    def __ensure_kernel(self, url, file, sha256):
        if file.exists():
            with open(file, "rb") as f:
                digest = hashlib.file_digest(f, "sha256")
            # allow local kernel images without URL and sha256 for testing
            assert (not sha256) == (not url)
            if not sha256 or digest.hexdigest() == sha256:
                return
        if not url or not sha256:
            raise NotImplementedError(f"URL / sha256 is missing for {self.__arch}")
        urllib.request.urlretrieve(url, file)
        with open(file, "rb") as f:
            digest = hashlib.file_digest(f, "sha256")
        if digest.hexdigest() != sha256:
            raise Exception(
                f"sha256 for {file} does not match: {digest.hexdigest()} != {sha256} (expected)"
            )

    def run(self):
        args = [f"qemu-system-{self.__arch}"]
        cmdline = "loglevel=7 panic=-1"
        match self.__arch:
            case "x86_64":
                args += ["-machine", "q35"]
                if os.access("/dev/kvm", os.W_OK):
                    args += ["-enable-kvm"]
                if Path("/usr/share/qemu/qboot.rom").exists():
                    args += ["-bios", "/usr/share/qemu/qboot.rom"]
                cmdline += " console=ttyS0,115200"
                virt_suffix = "pci"
                url = KERNEL_X86_64_URL
                kernel = Path("bzImage-x86_64")
                self.__ensure_kernel(url, kernel, KERNEL_X86_64_SHA256)
            case "aarch64":
                args += ["-machine", "virt", "-cpu", "cortex-a72"]
                virt_suffix = "device"
                url = KERNEL_AARCH64_URL
                kernel = Path("Image-aarch64")
                self.__ensure_kernel(url, kernel, KERNEL_AARCH64_SHA256)
            case _:
                raise NotImplementedError(
                    f"support for architecture {self.__arch} is missing"
                )
        if self.__cmdline:
            cmdline += " " + self.__cmdline

        args += ["-nographic"]
        args += ["-m", "512M", "-smp", "2"]
        args += ["-kernel", kernel]
        if self.__initramfs:
            args += ["-initrd", self.__initramfs]
        args += ["-no-reboot"]
        args += ["-append", cmdline]
        if self.__diskimage:
            args += [
                "-drive",
                f"if=none,format=raw,file={self.__diskimage},id=disk",
                "-device",
                f"virtio-blk-{virt_suffix},drive=disk",
            ]
        if self.__9p_root:
            args += [
                "-fsdev",
                f"local,id=rootfs,path={self.__9p_root},security_model=none",
                "-device",
                f"virtio-9p-{virt_suffix},fsdev=rootfs,mount_tag=/dev/root",
            ]
        with tempfile.NamedTemporaryFile(mode="r", dir=self.__tmp_path) as result:
            args += [
                "-device",
                f"virtio-serial-{virt_suffix}",
                "-chardev",
                f"file,id=rsinit,path={result.name}",
                "-device",
                "virtserialport,chardev=rsinit,name=rsinit.result.0",
            ]
            subprocess.run(args)
            return RuntimeStatus(result.read())


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser()

    parser.add_argument(
        "-c",
        "--cmdline",
        type=str,
        help=("The kernel command-line"),
    )

    parser.add_argument(
        "-i",
        "--initramfs",
        type=str,
        help=("The initramfs to use"),
    )

    parser.add_argument(
        "-d",
        "--diskimage",
        type=str,
        help=("The disk image to use"),
    )

    args = parser.parse_args()

    qemu = Qemu()
    if args.cmdline:
        qemu.set_cmdline(args.cmdline)
    if args.initramfs:
        qemu.set_initramfs(args.initramfs)
    if args.diskimage:
        qemu.set_diskimage(args.diskimage)

    print(qemu.run())
