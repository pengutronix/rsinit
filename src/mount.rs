// SPDX-License-Identifier: GPL-2.0-only
use crate::cmdline::CmdlineOptions;
use crate::Result;
use nix::mount::{mount, MsFlags};
use std::fs::{create_dir, remove_dir};
use std::io;
use std::path::Path;

fn mount_apivfs(dst: &str, fstype: &str) -> Result<()> {
    if let Err(e) = create_dir(dst) {
        if e.kind() != io::ErrorKind::AlreadyExists {
            return Err(format!("Failed to create {dst}: {e}").into());
        }
    }

    mount(
        Some(Path::new(fstype)),
        Path::new(dst),
        Some(Path::new(fstype)),
        MsFlags::empty(),
        Option::<&Path>::None,
    )
    .map_err(|e| format!("Failed to mount {fstype} -> {dst}: {e}"))?;

    Ok(())
}

fn mount_rootfs(
    src: &Option<String>,
    fstype: &Option<String>,
    flags: MsFlags,
    args: &Option<String>,
) -> Result<()> {
    if let Err(e) = create_dir("/root") {
        if e.kind() != io::ErrorKind::AlreadyExists {
            return Err(format!("Failed to create /root: {e}").into());
        }
    }

    println!(
        "Mounting rootfs {} -> /root ({}, '{}')",
        src.as_deref().unwrap(),
        fstype.as_deref().unwrap_or_default(),
        args.as_deref().unwrap_or_default()
    );
    mount(
        src.as_deref(),
        Path::new("/root"),
        fstype.as_deref(),
        flags,
        args.as_deref(),
    )
    .map_err(|e| {
        format!(
            "Failed to mount {} -> /root ({}, '{}'): {e}",
            src.clone().unwrap_or_default(),
            fstype.clone().unwrap_or_default(),
            args.clone().unwrap_or_default()
        )
    })?;

    Ok(())
}

pub fn mount_root(options: &CmdlineOptions) -> Result<()> {
    if options.root.is_none() {
        return Err("root= not found in /proc/cmdline".into());
    }
    mount_rootfs(
        &options.root,
        &options.rootfstype,
        options.rootfsflags,
        &options.rootflags,
    )?;
    Ok(())
}

fn mount_move(src: &str, dst: &str) -> Result<()> {
    mount(
        Some(Path::new(src)),
        dst,
        Option::<&Path>::None,
        MsFlags::MS_MOVE,
        Option::<&Path>::None,
    )
    .map_err(|e| format!("Failed to move mount {src} -> {dst}: {e}"))?;

    remove_dir(src)?;

    Ok(())
}

pub fn mount_special(mount_config: bool) -> Result<()> {
    mount_apivfs("/dev", "devtmpfs")?;
    mount_apivfs("/sys", "sysfs")?;
    if mount_config && Path::new("/sys/kernel/config").is_dir() {
        mount_apivfs("/sys/kernel/config", "configfs")?;
    }
    mount_apivfs("/proc", "proc")?;
    Ok(())
}

pub fn mount_move_special() -> Result<()> {
    mount_move("/dev", "/root/dev")?;
    mount_move("/sys", "/root/sys")?;
    mount_move("/proc", "/root/proc")?;

    mount(
        Some(Path::new("/")),
        "/root/mnt",
        Option::<&Path>::None,
        MsFlags::MS_BIND,
        Option::<&Path>::None,
    )
    .map_err(|e| format!("Failed to bind mount / -> /root/mnt: {e}"))?;

    Ok(())
}
