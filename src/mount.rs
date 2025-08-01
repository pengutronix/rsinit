// SPDX-License-Identifier: GPL-2.0-only

use std::fs::remove_dir;
use std::path::Path;

use log::debug;
use nix::mount::{mount, MsFlags};

use crate::cmdline::CmdlineOptions;
use crate::{mkdir, wait_for_device, Result};

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

pub fn mount_root(options: &CmdlineOptions) -> Result<()> {
    let root = options
        .root
        .as_ref()
        .ok_or("root= not found in /proc/cmdline")?;

    match options.rootfstype.as_deref() {
        Some("nfs") | Some("9p") => (),
        _ => wait_for_device(root)?,
    }
    mkdir("/root")?;

    debug!(
        "Mounting rootfs {} -> /root as {} with flags = {:#x}, data = '{}'",
        options.root.as_deref().ok_or("No root device argument")?,
        options.rootfstype.as_deref().unwrap_or_default(),
        options.rootfsflags.bits(),
        options.rootflags.as_deref().unwrap_or_default()
    );
    do_mount(
        options.root.as_deref(),
        "/root",
        options.rootfstype.as_deref(),
        options.rootfsflags,
        options.rootflags.as_deref(),
    )?;

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

pub fn mount_move_special(options: &CmdlineOptions) -> Result<()> {
    mount_move("/dev", "/root/dev", options.cleanup)?;
    mount_move("/sys", "/root/sys", options.cleanup)?;
    mount_move("/proc", "/root/proc", options.cleanup)?;
    Ok(())
}
