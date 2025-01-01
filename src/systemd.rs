// SPDX-License-Identifier: GPL-2.0-only
use crate::cmdline::CmdlineOptions;
use crate::mount::do_mount;
use crate::{mkdir, Result};
use nix::mount::{umount, MsFlags};
use nix::sys::reboot::{reboot, RebootMode};
use std::collections::BinaryHeap;
use std::env;
use std::fs::read_to_string;
use std::path::Path;

pub fn mount_systemd(options: &mut CmdlineOptions) -> Result<()> {
    do_mount(
        Option::<&str>::None,
        "/root/run",
        Some("tmpfs"),
        MsFlags::MS_NODEV
            .union(MsFlags::MS_NOSUID)
            .union(MsFlags::MS_STRICTATIME),
        Some("mode=0755"),
    )?;

    if !Path::new("/shutdown").exists() {
        return Ok(());
    }

    options.cleanup = false;

    /* expected by systemd when going back to the initramfs during shutdown */
    mkdir("/run")?;
    mkdir("/oldroot")?;

    do_mount(
        Some("/"),
        "/root/run/initramfs",
        Option::<&str>::None,
        MsFlags::MS_BIND,
        Option::<&str>::None,
    )?;

    Ok(())
}

fn umount_root() -> Result<()> {
    if let Ok(data) = read_to_string("/proc/self/mountinfo") {
        let mut mounts = BinaryHeap::new();
        for line in data.lines() {
            if let Some(mountpoint) = line.split(' ').nth(4) {
                if mountpoint.starts_with("/oldroot") {
                    mounts.push(mountpoint);
                }
            }
        }
        for mountpoint in mounts {
            umount(mountpoint).map_err(|e| format!("Failed to unmount {mountpoint}: {e}"))?;
        }
    }
    Ok(())
}

pub fn shutdown() -> Result<()> {
    umount_root()?;
    let arg = match env::args().nth(1).as_deref() {
        Some("halt") => RebootMode::RB_HALT_SYSTEM,
        Some("kexec") => RebootMode::RB_KEXEC,
        Some("poweroff") => RebootMode::RB_POWER_OFF,
        _ => RebootMode::RB_AUTOBOOT,
    };
    reboot(arg).map_err(|e| format!("reboot failed: {e}"))?;
    Ok(())
}
