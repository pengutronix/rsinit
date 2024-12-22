// SPDX-License-Identifier: GPL-2.0-only
use cmdline::{parse_cmdline, CmdlineOptions};
#[cfg(feature = "dmverity")]
use dmverity::prepare_dmverity;
use mount::{mount_move_special, mount_root, mount_special};
#[cfg(feature = "reboot-on-failure")]
use nix::sys::reboot::{reboot, RebootMode};
use nix::sys::termios::tcdrain;
use nix::unistd::{chdir, chroot, dup2, execv, unlink};
use std::env;
use std::env::current_exe;
use std::ffi::CString;
use std::fs::{create_dir, read_to_string, OpenOptions};
use std::io;
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::panic::set_hook;
#[cfg(feature = "systemd")]
use systemd::{mount_systemd, shutdown};
#[cfg(feature = "usb9pfs")]
use usbg_9pfs::prepare_9pfs_gadget;

mod cmdline;
#[cfg(feature = "dmverity")]
mod dmverity;
mod mount;
#[cfg(feature = "systemd")]
mod systemd;
#[cfg(feature = "usb9pfs")]
mod usbg_9pfs;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn mkdir(dir: &str) -> Result<()> {
    if let Err(e) = create_dir(dir) {
        if e.kind() != io::ErrorKind::AlreadyExists {
            return Err(format!("Failed to create {dir}: {e}",).into());
        }
    }
    Ok(())
}

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

fn start_root(options: &mut CmdlineOptions) -> Result<()> {
    #[cfg(feature = "systemd")]
    mount_systemd(options)?;

    if options.cleanup {
        match current_exe() {
            Err(e) => println!("current_exe failed: {e}"),
            Ok(exe) => unlink(exe.as_path())?,
        }
    }

    mount_move_special(options)?;

    chdir("/root")?;
    chroot(".")?;
    chdir("/")?;

    let mut args = Vec::new();
    args.push(options.init.clone());

    for arg in env::args_os().skip(1) {
        let carg = CString::new(arg.as_bytes())?;
        args.push(carg);
    }
    print!("Starting ");
    for arg in &args {
        print!("{} ", arg.to_str().unwrap_or("<invalid utf-8>"));
    }
    println!("...");

    execv(options.init.as_ref(), &args)?;

    Ok(())
}

#[cfg(any(feature = "dmverity", feature = "usb9pfs"))]
fn prepare_aux(options: &mut CmdlineOptions) -> Result<()> {
    #[cfg(feature = "dmverity")]
    if prepare_dmverity(options)? {
        return Ok(());
    }
    #[cfg(feature = "usb9pfs")]
    if prepare_9pfs_gadget(options)? {
        return Ok(());
    }
    Ok(())
}

fn init() -> Result<()> {
    mount_special()?;

    let cmdline = read_file("/proc/cmdline")?;
    let mut options = CmdlineOptions {
        ..Default::default()
    };
    parse_cmdline(cmdline, &mut options)?;

    #[cfg(any(feature = "dmverity", feature = "usb9pfs"))]
    prepare_aux(&mut options)?;

    mount_root(&options)?;
    start_root(&mut options)?;

    Ok(())
}

fn finalize() {
    /* Make sure all output is written before exiting */
    let _ = tcdrain(io::stdout().as_fd());
    #[cfg(feature = "reboot-on-failure")]
    let _ = reboot(RebootMode::RB_AUTOBOOT);
}

fn main() -> Result<()> {
    setup_console()?;

    set_hook(Box::new(|panic_info| {
        println!("panic occurred: {panic_info}");
        finalize();
    }));

    let cmd = env::args().next().unwrap();
    println!("Running {}...", cmd);

    if let Err(e) = match cmd.as_str() {
        #[cfg(feature = "systemd")]
        "/shutdown" => shutdown(),
        _ => init(),
    } {
        println!("{e}");
    }
    finalize();
    Ok(())
}
