#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 The rsinit Authors
# SPDX-License-Identifier: GPL-2.0-only

import os
import shutil
import subprocess
import tempfile
from pathlib import Path


def prepare_root(root):
    root.mkdir(exist_ok=True)
    for dir in ["run", "dev", "sys", "proc", "sbin", "usr", "mnt"]:
        root.joinpath(dir).mkdir()


class GenImage:
    def __init__(self, path, force=False):
        self.__path = Path(path)
        self.__image_path = self.__path / "images"
        self.__force = force
        self.__rdinit = None
        self.__init = None

    def set_rdinit(self, rdinit):
        self.__rdinit = rdinit

    def set_init(self, init):
        self.__init = init

    def _sbin_env(self):
        env = os.environ.copy()
        if (":" + env["PATH"] + ":").find(":/usr/bin:"):
            env["PATH"] = "/usr/sbin:" + env["PATH"]
        if (":" + env["PATH"] + ":").find(":/bin:"):
            env["PATH"] = "/sbin:" + env["PATH"]
        return env

    def _run_genimage(self, config, root=None):
        self.__image_path.mkdir(exist_ok=True)
        tmp_path = tempfile.TemporaryDirectory(dir=self.__path)
        with tempfile.NamedTemporaryFile(mode="w+", dir=self.__path) as cfg:
            cfg.write(config)
            cfg.flush()
            args = [
                "genimage",
                "--loglevel",
                "1",
                "--outputpath",
                str(self.__image_path),
                "--inputpath",
                str(self.__image_path),
                "--tmppath",
                tmp_path.name,
                "--config",
                cfg.name,
            ]
            if root:
                args += ["--root", root.name]
            subprocess.run(args, env=self._sbin_env()).check_returncode()

    def __image_file(self, name):
        return self.__image_path / name

    def __done(self, name):
        if self.__force:
            return False
        return self.__image_file(name).exists()

    def _prepare_root(self, files, initramfs=False):
        root = tempfile.TemporaryDirectory(dir=self.__path)
        base = Path(root.name)
        if not initramfs:
            prepare_root(base)
        for dst, src in files.items():
            dst_path = base / dst.lstrip("/")
            shutil.copyfile(src, dst_path)
            if set(Path(dst).parts).intersection({"bin", "sbin"}) or dst == "/init":
                dst_path.chmod(0o755)
        return root

    def prepare_root(self, init):
        return self._prepare_root({"/sbin/init": init})

    def _create_root_ext4(self, name):
        config = f"""\
image {name} {{
    ext4 {{
        use-mke2fs = true
        extraargs = "-b 4096"
    }}
    size = 32M
}}"""
        self._run_genimage(config, self._prepare_root({"/sbin/init": self.__init}))

    def _create_disk(self, name, rootfs):
        config = f"""\
image {name} {{
    hdimage {{
        partition-table-type = "gpt"
    }}
    size = 64M
    partition root {{
        image = "{rootfs}"
        partition-type-uuid = "L"
    }}
}}"""
        self._run_genimage(config)

    def get_ext4_disk(self):
        name = "hd-ext4.img"
        if self.__done(name):
            return self.__image_file(name)
        self._create_root_ext4("root.ext4")
        self._create_disk(name, "root.ext4")
        return self.__image_file(name)

    def _create_initramfs_with_files(self, name, files):
        name = f"rsinit-{name}.cpio.zstd"
        config = f"""\
image {name} {{
    cpio {{
        format = newc
        compress = zstd
    }}
    size = 32M
}}"""
        self._run_genimage(config, self._prepare_root(files, True))
        return self.__image_file(name)

    def _create_initramfs(self, name, rdinit):
        return self._create_initramfs_with_files(name, {"/init": rdinit})

    def get_initramfs(self):
        name = "rsinit.cpio.zstd"
        if self.__done(name):
            return self.__image_file(name)
        return self._create_initramfs(name, self.__rdinit)

    def create_initramfs(self, rdinit):
        name = f"rsinit-{Path(rdinit).name}.cpio.zstd"
        return self._create_initramfs(name, rdinit)

    def create_initramfs_with_files(self, name, files):
        name = f"rsinit-{name}.cpio.zstd"
        return self._create_initramfs_with_files(name, files)

    def _create_root_ext4_verity(self, name):
        self._create_root_ext4(name)
        image = self.__image_file(name)
        size = os.path.getsize(image)
        args = [
            "veritysetup",
            "--no-superblock",
            f"--hash-offset={size}",
            "format",
            image,
            image,
        ]
        output = subprocess.run(
            args,
            check=True,
            stdout=subprocess.PIPE,
            encoding="utf-8",
            env=self._sbin_env(),
        ).stdout.splitlines()
        verity_params = self.__path / (name + "-verity.params")
        with open(verity_params, "w+") as verity_config:
            for line in output:
                try:
                    key, value = line.split(":", 1)
                except ValueError:
                    continue
                key = "VERITY_" + key.strip().upper().replace(" ", "_")
                value = value.strip()
                verity_config.write(f"{key}={value}\n")
            verity_config.write(f"VERITY_DATA_SECTORS={int(size / 512)}\n")
            return verity_params

    def get_ext4_verity_disk(self):
        name = "hd-ext4-verity.img"
        if self.__done(name):
            return self.__image_file(name)
        verity_params = self._create_root_ext4_verity("root.verity")
        self._create_disk(name, "root.verity")
        return self.__image_file(name), verity_params


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser()

    parser.add_argument(
        "-d",
        "--dir",
        type=str,
        help=("the base directory used for everything"),
    )

    parser.add_argument(
        "-i",
        "--init",
        type=str,
        help=("the program used as /sbin/init"),
    )

    parser.add_argument(
        "-r",
        "--rdinit",
        type=str,
        help=("the program used as /init in the initramfs"),
    )

    parser.add_argument(
        "--disk",
        action="store_true",
        help=("The initramfs to use"),
    )

    parser.add_argument(
        "--initramfs",
        action="store_true",
        help=("The initramfs to use"),
    )

    args = parser.parse_args()

    if not args.dir:
        raise SystemExit("'--dir' is not optional")
    if args.disk and not args.init:
        raise SystemExit("'--init' required with '--disk'")
    if args.initramfs and not args.rdinit:
        raise SystemExit("'--rdinit' required with '--initramfs'")

    genimage = GenImage(args.dir, force=True)

    if args.disk:
        genimage.set_init(args.init)
        genimage.get_ext4_disk()

    if args.initramfs:
        genimage.set_rdinit(args.rdinit)
        genimage.get_initramfs()
