// SPDX-License-Identifier: GPL-2.0-only
use cmdline::{parse_cmdline, CmdlineOptions};
use nix::mount::{mount, MsFlags};
use nix::sys::termios::tcdrain;
use nix::unistd::{chdir, chroot, dup2, execv, unlink};
use std::env::current_exe;
use std::ffi::CString;
use std::fs::{create_dir, read_to_string, remove_dir, OpenOptions};
use std::io;
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::path::Path;
use usbg_9pfs::prepare_9pfs_gadget;

mod cmdline;
mod usbg_9pfs;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/*
 * Setup stdout/stderr. The kernel will create /dev/console in the
 * initramfs, so we can use that.
 * Remove the device node since it is no longer needed and devtmpfs will be
 * mounted over it anyways.
 */
fn setup_console() -> Result<()> {
    let f = OpenOptions::new().write(true).open("/dev/console")?;
    let raw_fd: RawFd = f.as_raw_fd();

    dup2(raw_fd, io::stdout().as_raw_fd())?;
    dup2(raw_fd, io::stderr().as_raw_fd())?;

    let _ = unlink("/dev/console");

    Ok(())
}

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

fn mount_root(
    src: Option<String>,
    fstype: Option<String>,
    flags: MsFlags,
    args: Option<String>,
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
            src.unwrap(),
            fstype.unwrap_or_default(),
            args.unwrap_or_default()
        )
    })?;

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

fn mount_special() -> Result<()> {
    mount_apivfs("/dev", "devtmpfs")?;
    mount_apivfs("/sys", "sysfs")?;
    if Path::new("/sys/kernel/config").is_dir() {
        mount_apivfs("/sys/kernel/config", "configfs")?;
    }
    mount_apivfs("/proc", "proc")?;

    let cmdline = read_to_string("/proc/cmdline")
        .map_err(|e| format!("Failed to read /proc/cmdline: {e}"))?;
    let mut options = CmdlineOptions {
        ..Default::default()
    };
    parse_cmdline(cmdline, &mut options)?;

    prepare_9pfs_gadget(&options)?;

    if options.root.is_none() {
        return Err("root= not found in /proc/cmdline".into());
    }
    mount_root(
        options.root,
        options.rootfstype,
        options.rootfsflags,
        options.rootflags,
    )?;

    match current_exe() {
        Err(e) => println!("current_exe failed: {e}"),
        Ok(exe) => unlink(exe.as_path())?,
    }

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

    chdir("/root")?;
    chroot(".")?;
    chdir("/")?;

    println!("Starting /sbin/init...");
    execv(
        CString::new("/sbin/init").unwrap().as_ref(),
        &[CString::new("/sbin/init").unwrap().as_ref()],
    )?;

    Ok(())
}

fn main() -> Result<()> {
    setup_console()?;

    println!("Running init...");

    if let Err(e) = mount_special() {
        println!("{e}");
    }
    /* Make sure all output is written before exiting */
    tcdrain(io::stdout().as_fd())?;
    Ok(())
}
