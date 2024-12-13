// SPDX-License-Identifier: GPL-2.0-only
use cmdline::{parse_cmdline, CmdlineOptions};
use dmverity::prepare_dmverity;
use mount::{mount_move_special, mount_root, mount_special};
use nix::sys::termios::tcdrain;
use nix::unistd::{chdir, chroot, dup2, execv, unlink};
use std::env::current_exe;
use std::fs::{read_to_string, OpenOptions};
use std::io;
use std::os::fd::{AsFd, AsRawFd, RawFd};
use usbg_9pfs::prepare_9pfs_gadget;

mod cmdline;
mod dmverity;
mod mount;
mod usbg_9pfs;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn read_file(filename: &str) -> std::result::Result<String, String> {
    read_to_string(filename).map_err(|e| format!("Failed to read {filename}: {e}"))
}

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

fn start_root(options: &CmdlineOptions) -> Result<()> {
    match current_exe() {
        Err(e) => println!("current_exe failed: {e}"),
        Ok(exe) => unlink(exe.as_path())?,
    }

    mount_move_special()?;

    chdir("/root")?;
    chroot(".")?;
    chdir("/")?;

    println!(
        "Starting {}...",
        options.init.to_str().unwrap_or("<invalid utf-8>")
    );
    execv(options.init.as_ref(), &[options.init.as_ref()])?;

    Ok(())
}

fn run() -> Result<()> {
    mount_special(true)?;

    let cmdline = read_file("/proc/cmdline")?;
    let mut options = CmdlineOptions {
        ..Default::default()
    };
    parse_cmdline(cmdline, &mut options)?;

    prepare_9pfs_gadget(&options)?;
    prepare_dmverity(&mut options)?;

    mount_root(&options)?;
    start_root(&options)?;

    Ok(())
}

fn main() -> Result<()> {
    setup_console()?;

    println!("Running init...");

    if let Err(e) = run() {
        println!("{e}");
    }
    /* Make sure all output is written before exiting */
    tcdrain(io::stdout().as_fd())?;
    Ok(())
}
