// SPDX-FileCopyrightText: 2025 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::borrow::Borrow;
use std::env;
use std::env::current_exe;
use std::ffi::CString;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io;
use std::io::Write as _;
use std::os::fd::AsFd;
use std::os::unix::ffi::OsStrExt;

use log::{debug, Level, LevelFilter, Metadata, Record};
#[cfg(feature = "reboot-on-failure")]
use nix::sys::reboot::{reboot, RebootMode};
use nix::sys::termios::tcdrain;
use nix::unistd::{chdir, chroot, dup2_stderr, dup2_stdout, execv, unlink};

use crate::cmdline::{parse_cmdline, CmdlineOptions};
#[cfg(feature = "dmverity")]
use crate::dmverity::prepare_dmverity;
use crate::mount::{mount_move_special, mount_root, mount_special};
#[cfg(feature = "systemd")]
use crate::systemd::mount_systemd;
#[cfg(feature = "usb9pfs")]
use crate::usbg_9pfs::prepare_9pfs_gadget;
use crate::util::{read_file, Result};

/*
 * Setup stdout/stderr. The kernel will create /dev/console in the
 * initramfs, so we can use that.
 * Remove the device node since it is no longer needed and devtmpfs will be
 * mounted over it anyways.
 */
pub fn setup_console() -> Result<()> {
    let f = OpenOptions::new().write(true).open("/dev/console")?;
    let fd = f.as_fd();

    dup2_stdout(fd)?;
    dup2_stderr(fd)?;

    let _ = unlink("/dev/console");

    Ok(())
}

pub fn switch_root(options: &mut CmdlineOptions) -> Result<()> {
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
    Ok(())
}

pub fn start_init(options: &CmdlineOptions) -> Result<()> {
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
pub fn prepare_aux(options: &mut CmdlineOptions) -> Result<()> {
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

pub fn setup_log() -> Result<()> {
    let logger = KmsgLogger {
        kmsg: OpenOptions::new().write(true).open("/dev/kmsg")?,
    };
    log::set_boxed_logger(Box::new(logger)).map(|()| log::set_max_level(LevelFilter::Trace))?;
    Ok(())
}

pub fn init() -> Result<()> {
    mount_special()?;

    setup_log()?;

    let cmdline = read_file("/proc/cmdline")?;

    let mut options = parse_cmdline(&cmdline)?;

    #[cfg(any(feature = "dmverity", feature = "usb9pfs"))]
    prepare_aux(&mut options)?;

    mount_root(&options)?;
    switch_root(&mut options)?;

    start_init(&options)?;

    Ok(())
}

pub fn finalize() {
    /* Make sure all output is written before exiting */
    let _ = tcdrain(io::stdout().as_fd());
    #[cfg(feature = "reboot-on-failure")]
    let _ = reboot(RebootMode::RB_AUTOBOOT);
}
