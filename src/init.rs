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
use std::panic::set_hook;

use log::{debug, Level, LevelFilter, Metadata, Record};
#[cfg(feature = "reboot-on-failure")]
use nix::sys::reboot::{reboot, RebootMode};
use nix::sys::termios::tcdrain;
use nix::unistd::{chdir, chroot, dup2_stderr, dup2_stdout, execv, unlink};

use crate::cmdline::{CmdlineCallback, CmdlineOptions, CmdlineOptionsParser};
#[cfg(feature = "dmverity")]
use crate::dmverity::prepare_dmverity;
use crate::mount::{
    mount_move_special, mount_overlay, mount_root, mount_special, mount_tmpfs_overlay,
};
#[cfg(feature = "systemd")]
use crate::systemd::mount_systemd;
#[cfg(feature = "usb9pfs")]
use crate::usbg_9pfs::prepare_9pfs_gadget;
use crate::util::Result;

/*
 * Setup stdout/stderr. The kernel will create /dev/console in the
 * initramfs, so we can use that.
 * Remove the device node since it is no longer needed and devtmpfs will be
 * mounted over it anyways.
 */
fn setup_console() -> Result<()> {
    let f = OpenOptions::new().write(true).open("/dev/console")?;
    let fd = f.as_fd();

    dup2_stdout(fd)?;
    dup2_stderr(fd)?;

    let _ = unlink("/dev/console");

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

fn finalize() {
    /* Make sure all output is written before exiting */
    let _ = tcdrain(io::stdout().as_fd());
    #[cfg(feature = "reboot-on-failure")]
    let _ = reboot(RebootMode::RB_AUTOBOOT);
}

pub struct InitContext<'a> {
    pub options: CmdlineOptions,
    parser: CmdlineOptionsParser<'a>,
}

impl<'a> InitContext<'a> {
    pub fn new() -> Result<Self> {
        setup_console()?;

        set_hook(Box::new(|panic_info| {
            println!("panic occurred: {panic_info}");
            finalize();
        }));

        Ok(Self {
            options: CmdlineOptions::default(),
            parser: CmdlineOptionsParser::new(),
        })
    }

    /// Register a command line parser callback for every option the built-in parser does not
    /// handle itself. Use [`crate::cmdline::ensure_value`] when your option requires an argument.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use std::cell::RefCell;
    /// use rsinit::cmdline::ensure_value;
    /// use rsinit::init::InitContext;
    ///
    /// let mut custom_option = RefCell::new(String::new());
    /// let mut ctx = InitContext::new()?;
    ///
    /// ctx.add_cmdline_parser_callback(Box::new(|key, val| {
    ///     if key == "my.option" {
    ///         *custom_option.borrow_mut() = ensure_value(key, val)?.to_owned();
    ///     }
    ///     Ok(())
    /// }));
    ///
    /// // When `setup()` parses /proc/cmdline, the callback will be invoked
    /// ctx.setup()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn add_cmdline_parser_callback(&mut self, cb: Box<CmdlineCallback<'a>>) {
        self.parser.add_callback(cb);
    }

    pub fn setup(&mut self) -> Result<()> {
        mount_special()?;

        setup_log()?;

        self.options = self.parser.parse_file("/proc/cmdline")?;

        Ok(())
    }

    #[cfg(any(feature = "dmverity", feature = "usb9pfs"))]
    pub fn prepare_aux(self: &mut InitContext<'a>) -> Result<()> {
        #[cfg(feature = "dmverity")]
        if prepare_dmverity(&mut self.options)? {
            return Ok(());
        }
        #[cfg(feature = "usb9pfs")]
        if prepare_9pfs_gadget(&self.options)? {
            return Ok(());
        }
        Ok(())
    }

    pub fn switch_root(self: &mut InitContext<'a>) -> Result<()> {
        #[cfg(feature = "systemd")]
        mount_systemd(&mut self.options)?;

        if self.options.cleanup {
            let exe = current_exe().map_err(|e| format!("current_exe failed: {e}"))?;
            unlink(exe.as_path())?;
        }

        mount_move_special(self.options.cleanup)?;

        chdir("/root")?;
        chroot(".")?;
        chdir("/")?;
        Ok(())
    }

    pub fn mount_root(self: &InitContext<'a>) -> Result<()> {
        mount_root(
            self.options.root.as_deref(),
            self.options.rootfstype.as_deref(),
            self.options.rootfsflags,
            self.options.rootflags.as_deref(),
        )?;
        Ok(())
    }

    pub fn mount_tmpfs_root_overlay(self: &InitContext<'a>) -> Result<()> {
        mount_tmpfs_overlay(self.options.rootfsflags, "/")
    }

    pub fn mount_root_overlay(
        self: &InitContext<'a>,
        data: Option<&str>,
        upper: &str,
    ) -> Result<()> {
        mount_overlay(self.options.rootfsflags, data, upper, "/")
    }

    pub fn start_init(self: &InitContext<'a>) -> Result<()> {
        let mut args = Vec::new();
        args.push(CString::new(self.options.init.as_str())?);

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

    pub fn finish(self: &mut InitContext<'a>) -> Result<()> {
        self.switch_root()?;
        self.start_init()?;

        Ok(())
    }

    pub fn run(self: &mut InitContext<'a>) -> Result<()> {
        self.setup()?;

        #[cfg(any(feature = "dmverity", feature = "usb9pfs"))]
        self.prepare_aux()?;

        self.mount_root()?;

        self.finish()?;

        Ok(())
    }
}

impl Drop for InitContext<'_> {
    fn drop(&mut self) {
        finalize();
    }
}
