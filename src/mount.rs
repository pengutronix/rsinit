// SPDX-FileCopyrightText: 2024 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::fs::remove_dir;
use std::path::Path;

use log::debug;
use nix::mount::{mount, umount, MsFlags};

use crate::util::{mkdir, wait_for_device, Result};

pub fn do_mount(
    src: Option<&str>,
    dst: &str,
    fstype: Option<&str>,
    flags: MsFlags,
    data: Option<&str>,
) -> Result<()> {
    mkdir(dst)?;

    mount(src, dst, fstype, flags, data).map_err(|e| {
        format!(
            "Failed to mount {} -> {} as {} with flags = {:#x}, data = '{}'): {e}",
            src.unwrap_or_default(),
            dst,
            fstype.unwrap_or_default(),
            flags.bits(),
            data.unwrap_or_default(),
        )
    })?;

    Ok(())
}

pub fn mount_apivfs(dst: &str, fstype: &str) -> Result<()> {
    do_mount(
        Some(fstype),
        dst,
        Some(fstype),
        MsFlags::empty(),
        Option::<&str>::None,
    )?;

    Ok(())
}

pub fn mount_root(
    device: Option<&str>,
    fstype: Option<&str>,
    fsflags: MsFlags,
    flags: Option<&str>,
) -> Result<()> {
    let root = device.as_ref().ok_or("root= not found in /proc/cmdline")?;

    match fstype {
        Some("nfs") | Some("9p") => (),
        _ => wait_for_device(root)?,
    }
    mkdir("/root")?;

    debug!(
        "Mounting rootfs {} -> /root as {} with flags = {:#x}, data = '{}'",
        device.ok_or("No root device argument")?,
        fstype.unwrap_or_default(),
        fsflags.bits(),
        flags.unwrap_or_default()
    );
    do_mount(device, "/root", fstype, fsflags, flags)?;

    Ok(())
}

fn mount_move(src: &str, dst: &str, cleanup: bool) -> Result<()> {
    mount(
        Some(Path::new(src)),
        dst,
        Option::<&Path>::None,
        MsFlags::MS_MOVE,
        Option::<&Path>::None,
    )
    .map_err(|e| format!("Failed to move mount {src} -> {dst}: {e}"))?;

    if cleanup {
        remove_dir(src)?;
    }

    Ok(())
}

pub fn mount_special() -> Result<()> {
    mount_apivfs("/dev", "devtmpfs")?;
    mount_apivfs("/sys", "sysfs")?;
    mount_apivfs("/proc", "proc")?;
    Ok(())
}

pub fn mount_move_special(cleanup: bool) -> Result<()> {
    mount_move("/dev", "/root/dev", cleanup)?;
    mount_move("/sys", "/root/sys", cleanup)?;
    mount_move("/proc", "/root/proc", cleanup)?;
    Ok(())
}

pub fn mount_overlay(
    flags: MsFlags,
    data: Option<&str>,
    upper: &str,
    mountpoint: &str,
    name: Option<&str>,
) -> Result<()> {
    let upperdir = format!("{upper}/upperdir");
    let workdir = format!("{upper}/workdir");
    let options = data.unwrap_or("");

    if !mountpoint.starts_with("/") {
        return Err(format!("Mountpoint '{mountpoint}' for overlays must start with a '/'").into());
    }
    let mountdir = format!("/root{mountpoint}");

    mkdir(&upperdir)?;
    mkdir(&workdir)?;

    do_mount(
        name,
        &mountdir,
        Some("overlay"),
        flags,
        Some(
            format!("lowerdir={mountdir},upperdir={upperdir},workdir={workdir},{options}").as_str(),
        ),
    )?;
    Ok(())
}

pub fn mount_tmpfs_overlay(
    overlayflags: MsFlags,
    mountpoint: &str,
    name: Option<&str>,
) -> Result<()> {
    let dir = "/.overlay";

    mkdir(dir)?;
    do_mount(
        Option::<&str>::None,
        dir,
        Some("tmpfs"),
        MsFlags::empty(),
        Some("mode=0755"),
    )?;

    mount_overlay(
        overlayflags,
        Some("redirect_dir=on,index=on,metacopy=on,volatile"),
        dir,
        mountpoint,
        name,
    )?;
    umount(dir).map_err(|e| format!("Failed to unmount {dir}: {e}"))?;
    remove_dir(dir)?;

    Ok(())
}
