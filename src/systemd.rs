// SPDX-FileCopyrightText: 2024 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::collections::BinaryHeap;
use std::env;
use std::fs::read_to_string;
use std::path::Path;

use log::info;
use nix::mount::{umount, MsFlags};
use nix::sys::reboot::{reboot, RebootMode};

use crate::cmdline::CmdlineOptions;
use crate::mount::do_mount;
use crate::util::{mkdir, Result};

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

#[allow(unreachable_code)]
pub fn shutdown() -> Result<()> {
    umount_root()?;
    let arg = match env::args().nth(1).as_deref() {
        Some("halt") => RebootMode::RB_HALT_SYSTEM,
        Some("kexec") => RebootMode::RB_KEXEC,
        Some("poweroff") => RebootMode::RB_POWER_OFF,
        _ => RebootMode::RB_AUTOBOOT,
    };
    reboot(arg).map_err(|e| format!("reboot failed: {e}"))?;

    Ok(()) // Actually unreachable
}

/// Merge the configuration extensions named on the command-line from
/// `/usr/lib/confexts/` of the root filesystem into `/root/etc` as a read-only
/// overlay. Later names take precedence over earlier ones.
pub fn mount_confexts(names: &[String]) -> Result<()> {
    if names.is_empty() {
        return Ok(());
    }

    let mut lowerdir: Vec<String> = names
        .iter()
        .rev()
        .map(|name| format!("/root/usr/lib/confexts/{name}/etc"))
        .collect();
    lowerdir.push("/root/etc".to_string());

    info!(
        "Merging configuration extension(s) {} into /etc",
        names.join(", ")
    );

    do_mount(
        Some("confext"),
        "/root/etc",
        Some("overlay"),
        MsFlags::MS_RDONLY | MsFlags::MS_NODEV | MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC,
        Some(&format!("lowerdir={}", lowerdir.join(":"))),
    )
}
