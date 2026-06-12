// SPDX-FileCopyrightText: 2025 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::env;
use std::env::current_exe;
use std::ffi::CString;
use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io;
use std::mem::take;
use std::os::fd::AsFd;
use std::os::unix::ffi::OsStrExt;
use std::panic::set_hook;

use log::{error, info, LevelFilter};
#[cfg(feature = "reboot-on-failure")]
use nix::sys::reboot::{reboot, RebootMode};
use nix::sys::termios::tcdrain;
use nix::unistd::{chdir, chroot, dup2_stderr, dup2_stdout, execv, unlink};

use crate::cmdline::{CmdlineOptions, CmdlineOptionsParser};
#[cfg(feature = "dmverity")]
use crate::dmverity::prepare_dmverity;
#[cfg(feature = "integration-test")]
use crate::integration::IntegrationLogger as Logger;
#[cfg(not(feature = "integration-test"))]
use crate::kmsg::KmsgLogger as Logger;
use crate::mount::{
    mount_bind_kernel_modules, mount_move_special, mount_overlay, mount_root, mount_special,
    mount_tmpfs_overlay,
};
#[cfg(feature = "systemd")]
use crate::systemd::{mount_systemd, shutdown};
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

pub fn setup_log() -> Result<()> {
    let logger = Logger::new()?;
    log::set_boxed_logger(Box::new(logger)).map(|()| log::set_max_level(LevelFilter::Trace))?;
    Ok(())
}

fn finalize() {
    /* Make sure all output is written before exiting */
    let _ = tcdrain(io::stdout().as_fd());
    #[cfg(feature = "reboot-on-failure")]
    let _ = reboot(RebootMode::RB_AUTOBOOT);
}

/// The lifecycle phases where callbacks can be registered.
///
/// # Example
///
/// ```no_run
/// use rsinit::init::{CallBack, InitContext};
///
/// let mut ctx = InitContext::new()?;
/// ctx.add_callback(CallBack::PostSetup, |ctx| {
///     println!("Post setup phase");
///     Ok(())
/// });
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CallBack {
    /// Executed after the initial setup (mounting special filesystems, setting up logging, and parsing cmdline).
    PostSetup,
    /// Executed after the root filesystem has been mounted, before switching root.
    PostRootMount,
    /// Executed after switching the root filesystem, before starting the next init process.
    PostSwitchRoot,
}

pub trait InitCallback {
    fn call(&mut self, ctx: &mut InitContext) -> Result<()>;
}

impl<F> InitCallback for F
where
    F: FnMut(&mut InitContext) -> Result<()>,
{
    fn call(&mut self, ctx: &mut InitContext) -> Result<()> {
        self(ctx)
    }
}

pub struct InitContext<'a> {
    pub options: CmdlineOptions,
    parser: CmdlineOptionsParser<'a>,
    callbacks: Vec<(CallBack, Box<dyn InitCallback + 'a>)>,
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
            callbacks: Vec::default(),
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
    /// let custom_option = RefCell::new(String::new());
    /// let mut ctx = InitContext::new()?;
    ///
    /// ctx.add_cmdline_parser_callback(|key, val| {
    ///     if key == "my.option" {
    ///         *custom_option.borrow_mut() = ensure_value(key, val)?.to_owned();
    ///     }
    ///     Ok(())
    /// });
    ///
    /// // When `setup()` parses /proc/cmdline, the callback will be invoked
    /// ctx.setup()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn add_cmdline_parser_callback<F>(&mut self, cb: F)
    where
        F: FnMut(&str, Option<&str>) -> Result<()> + 'a,
    {
        self.parser.add_callback(Box::new(cb));
    }

    /// Register a callback to be executed during a specific lifecycle phase.
    ///
    /// Callbacks are executed in the order they were registered for a given [`CallBack`] phase.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rsinit::init::{CallBack, InitContext};
    ///
    /// let mut ctx = InitContext::new()?;
    /// ctx.add_callback(CallBack::PostRootMount, |ctx| {
    ///     println!("The root filesystem has been mounted!");
    ///     Ok(())
    /// });
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn add_callback<F>(&mut self, kind: CallBack, cb: F)
    where
        F: FnMut(&mut InitContext) -> Result<()> + 'a,
    {
        self.callbacks.push((kind, Box::new(cb)));
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
        if prepare_9pfs_gadget(&mut self.options)? {
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
        mount_tmpfs_overlay(self.options.rootfsflags, "/", self.options.root.as_deref())
    }

    pub fn mount_root_overlay(
        self: &InitContext<'a>,
        data: Option<&str>,
        upper: &str,
    ) -> Result<()> {
        mount_overlay(
            self.options.rootfsflags,
            data,
            upper,
            "/",
            self.options.root.as_deref(),
        )
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
        info!("{}", &buf);

        execv(&args[0], &args)?;

        Ok(())
    }

    pub fn finish(self: &mut InitContext<'a>) -> Result<()> {
        self.switch_root()?;
        self.run_callbacks(CallBack::PostSwitchRoot)?;
        self.start_init()?;

        Ok(())
    }

    /// Run rsinit using the first argument from the commandline. If run under
    /// systemd the argument is `shutdown`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use rsinit::init::InitContext;
    ///
    /// let mut ctx = InitContext::new()?;
    /// ctx.run_from_env()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn run_from_env(&mut self) -> Result<()> {
        let cmd = env::args().next().ok_or("No cmd to run was found")?;
        self.run(&cmd);
        Ok(())
    }

    pub fn run(self: &mut InitContext<'a>, cmd: &str) {
        // log isn't setup at this point
        println!("Running {cmd}...");
        let result = match cmd {
            #[cfg(feature = "systemd")]
            "/shutdown" => shutdown(),
            _ => self.run_impl(),
        };

        if let Err(e) = result {
            error!("{e}");
        }
    }

    fn run_callbacks(self: &mut InitContext<'a>, target_kind: CallBack) -> Result<()> {
        let mut cbs = take(&mut self.callbacks);

        for (kind, ref mut cb) in cbs.iter_mut() {
            if *kind == target_kind {
                cb.call(self)?;
            }
        }

        self.callbacks = cbs;

        Ok(())
    }

    fn run_impl(self: &mut InitContext<'a>) -> Result<()> {
        self.setup()?;

        self.run_callbacks(CallBack::PostSetup)?;

        #[cfg(any(feature = "dmverity", feature = "usb9pfs"))]
        self.prepare_aux()?;

        self.mount_root()?;

        self.run_callbacks(CallBack::PostRootMount)?;

        if self.options.bind_modules {
            mount_bind_kernel_modules()?;
        }

        self.finish()
    }
}

impl Drop for InitContext<'_> {
    fn drop(&mut self) {
        finalize();
    }
}
