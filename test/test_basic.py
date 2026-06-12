# SPDX-FileCopyrightText: 2026 The rsinit Authors
# SPDX-License-Identifier: GPL-2.0-only


def assert_apivfs(result):
    sysfs = result.get_mount(mount_point="/sys")
    assert sysfs["filesystem-type"] == "sysfs"
    assert set(sysfs["mount-options"]) == {
        "rw",
        "nosuid",
        "nodev",
        "noexec",
        "relatime",
    }
    proc = result.get_mount(mount_point="/proc")
    assert proc["filesystem-type"] == "proc"
    assert set(proc["mount-options"]) == {
        "rw",
        "nosuid",
        "nodev",
        "noexec",
        "relatime",
    }
    dev = result.get_mount(mount_point="/dev")
    assert dev["filesystem-type"] == "devtmpfs"
    assert set(dev["mount-options"]) == {
        "rw",
        "nosuid",
    }
    assert {"size=4096k", "mode=755"} <= set(dev["super-options"])
    run = result.get_mount(mount_point="/run")
    assert run["filesystem-type"] == "tmpfs"
    assert set(run["mount-options"]) == {
        "rw",
        "nosuid",
        "nodev",
    }
    assert "mode=755" in set(dev["super-options"])


def test_basic_ext4(genimage, qemu):
    qemu.set_initramfs(genimage.get_initramfs())
    qemu.set_diskimage(genimage.get_ext4_disk())
    qemu.set_cmdline("rootwait root=/dev/vda1 rootfstype=ext4")
    result = qemu.run()
    result.assert_system_state()
    root_mount = result.get_mount(mount_point="/")
    assert root_mount["mount-source"] == "/dev/vda1"
    assert root_mount["st_dev"] in result.block_devices
    assert result.block_devices[root_mount["st_dev"]] == "vda1"
    assert root_mount["filesystem-type"] == "ext4"
    assert_apivfs(result)
    assert len(result.mountinfo) == 5


def test_basic_9pfs(rsinit, genimage, qemu):
    root = genimage.prepare_root(rsinit.init_path)
    qemu.set_initramfs(genimage.get_initramfs())
    qemu.set_9p_root(root.name)
    qemu.set_cmdline("root=/dev/root rootfstype=9p rootflags=trans=virtio")
    result = qemu.run()
    result.assert_system_state()
    root_mount = result.get_mount(mount_point="/")
    assert root_mount["mount-source"] == "/dev/root"
    assert root_mount["filesystem-type"] == "9p"
    assert_apivfs(result)
    assert len(result.mountinfo) == 5


def test_nfs_bind_mounts(rsinit, genimage, qemu):
    qemu.set_initramfs(
        genimage.create_initramfs(rsinit.build_path / "examples" / "nfs-bind-mounts")
    )
    qemu.set_diskimage(genimage.get_ext4_disk())
    qemu.set_cmdline(
        "rootwait root=/dev/vda1 rootfstype=ext4 rsinit.bind=/root/sys,/root/mnt"
    )
    result = qemu.run()
    result.assert_system_state()
    root_mount = result.get_mount(mount_point="/")
    assert root_mount["mount-source"] == "/dev/vda1"
    assert root_mount["filesystem-type"] == "ext4"
    assert_apivfs(result)
    bind_mount = result.get_mount(mount_point="/mnt")
    assert bind_mount["mount-source"] == "/dev/vda1"
    assert bind_mount["filesystem-type"] == "ext4"
    assert bind_mount["root"] == "/sys"
    assert len(result.mountinfo) == 6


def test_missing_root(genimage, qemu):
    qemu.set_initramfs(genimage.get_initramfs())
    qemu.set_cmdline("root=/dev/vda1 rootfstype=ext4")
    result = qemu.run()
    assert result.rsinit_messages == [
        {"message": "Timeout reached while waiting for the device"}
    ]
    assert not result.mountinfo


def test_verity(rsinit, genimage, qemu):
    disk, verity_params = genimage.get_ext4_verity_disk()
    qemu.set_diskimage(disk)
    files = {"/init": rsinit.rdinit_path, "/verity-params": verity_params}
    qemu.set_initramfs(genimage.create_initramfs_with_files("ext4-verity", files))
    qemu.set_cmdline("rootwait rsinit.verity_root=/dev/vda1 rootfstype=ext4")
    result = qemu.run()
    result.assert_system_state()
    root_mount = result.get_mount(mount_point="/")
    assert root_mount["mount-source"] == "/dev/dm-0"
    assert root_mount["st_dev"] in result.block_devices
    assert result.block_devices[root_mount["st_dev"]] == "dm-0"
    assert root_mount["filesystem-type"] == "ext4"
    assert_apivfs(result)
    assert len(result.mountinfo) == 5
