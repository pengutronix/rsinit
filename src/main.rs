// SPDX-License-Identifier: GPL-2.0-only

use std::borrow::Borrow;
use std::env;
use std::env::current_exe;
use std::ffi::CString;
use std::fmt::Write as _;
use std::fs::{create_dir, read_to_string, File, OpenOptions};
use std::io;
use std::io::Write as _;
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::panic::set_hook;
use std::path::Path;
use std::thread;
use std::time;

use cmdline::{parse_cmdline, CmdlineOptions};
#[cfg(feature = "dmverity")]
use dmverity::prepare_dmverity;
use log::{debug, Level, LevelFilter, Metadata, Record};
use mount::{mount_move_special, mount_root, mount_special};
#[cfg(feature = "reboot-on-failure")]
use nix::sys::reboot::{reboot, RebootMode};
use nix::sys::termios::tcdrain;
use nix::unistd::{chdir, chroot, dup2, execv, unlink};
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
    if !Path::new(dir).exists() {
        if let Err(e) = create_dir(dir) {
            return Err(format!("Failed to create {dir}: {e}",).into());
        }
    }
    Ok(())
}

fn read_file(filename: &str) -> std::result::Result<String, String> {
    read_to_string(filename).map_err(|e| format!("Failed to read {filename}: {e}"))
}

fn wait_for_device(root_device: &str) -> Result<()> {
    let duration = time::Duration::from_millis(5);
    let path = Path::new(&root_device);

    for _ in 0..1000 {
        if path.exists() {
            return Ok(());
        }

        thread::sleep(duration);
    }

    Err("timout reached while waiting for the device".into())
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
        let exe = current_exe().map_err(|e| format!("current_exe failed: {e}"))?;
        unlink(exe.as_path())?;
    }

    mount_move_special(options)?;

    chdir("/root")?;
    chroot(".")?;
    chdir("/")?;

    let mut args = Vec::new();
    args.push(CString::new(options.init.as_str())?);

    for arg in env::args_os().skip(1) {
        let carg = CString::new(arg.as_bytes())?;
        args.push(carg);
    }
    let mut buf = "Starting ".to_string();
    for arg in &args {
        write!(buf, "{} ", arg.to_bytes().escape_ascii())?;
    }
    writeln!(buf, "...")?;
    debug!("{}", &buf);

    execv(&args[0], &args)?;

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

struct KmsgLogger {
    kmsg: File,
}

impl log::Log for KmsgLogger {
    fn enabled(&self, _: &Metadata) -> bool {
        true
    }
    fn log(&self, record: &Record) {
        let level = match record.level() {
            Level::Error => 3,
            Level::Warn => 4,
            /* 5 == notice has no equivalent */
            Level::Info => 6,
            Level::Debug | Level::Trace => 7,
        } | (1 << 3);
        /* Format first to ensure that the whole message is written with
         * one write() system-call */
        let msg = format!("<{level}> rsinit: {}", record.args());
        let _ = self.kmsg.borrow().write_all(msg.as_bytes());
    }
    fn flush(&self) {}
}

fn setup_log() -> Result<()> {
    let logger = KmsgLogger {
        kmsg: OpenOptions::new().write(true).open("/dev/kmsg")?,
    };
    log::set_boxed_logger(Box::new(logger)).map(|()| log::set_max_level(LevelFilter::Trace))?;
    Ok(())
}

fn init() -> Result<()> {
    mount_special()?;

    setup_log()?;

    let cmdline = read_file("/proc/cmdline")?;

    let mut options = parse_cmdline(&cmdline)?;

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

    let cmd = env::args().next().ok_or("No cmd to run as found")?;
    println!("Running {cmd}...");

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
