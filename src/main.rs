// SPDX-License-Identifier: GPL-2.0-only
use nix::mount::{mount, MsFlags};
use nix::unistd::{chdir, chroot, dup2, execv, unlink};
use nix::sys::termios::tcdrain;
use std::env::current_exe;
use std::ffi::CString;
use std::fs::{create_dir, read_dir, read_to_string, remove_dir, write, OpenOptions};
use std::io;
use std::os::fd::{AsFd, AsRawFd, RawFd};
use std::os::unix::fs::symlink;
use std::path::Path;
use std::{thread, time};
use cmdline::{parse_cmdline, CmdlineOptions};

mod cmdline;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn mkdir(dir: &str) -> Result<()> {
    create_dir(dir).map_err(|e| format!("Failed to create {dir}: {e}"))?;
    Ok(())
}

fn write_file(path: &str, content: &str) -> Result<()> {
    write(path, content).map_err(|e| format!("Failed to write to {path}: {e}"))?;
    Ok(())
}

fn setup_9pfs_gadget(device: &String) -> Result<()> {
    println!("Initializing USB 9pfs gadget ...");

    let mut udc = String::new();
    for entry in read_dir("/sys/class/udcx")? {
        let os_name = entry?.file_name();
        udc = String::from(os_name.to_str().unwrap());
        break;
    }
    if udc.is_empty() {
        return Err("No UDC found to attach the 9pfs gadget".into());
    }

    mkdir("/sys/kernel/config/usb_gadget/9pfs")?;

    write_file("/sys/kernel/config/usb_gadget/9pfs/idVendor", "0x1d6b")?;
    write_file("/sys/kernel/config/usb_gadget/9pfs/idProduct", "0x0109")?;

    mkdir("/sys/kernel/config/usb_gadget/9pfs/strings/0x409")?;
    write_file(
        "/sys/kernel/config/usb_gadget/9pfs/strings/0x409/serialnumber",
        "01234567",
    )?;
    write_file(
        "/sys/kernel/config/usb_gadget/9pfs/strings/0x409/manufacturer",
        "Pengutronix e.K.",
    )?;
    write_file(
        "/sys/kernel/config/usb_gadget/9pfs/strings/0x409/product",
        "9PFS Gadget",
    )?;

    mkdir("/sys/kernel/config/usb_gadget/9pfs/configs/c.1")?;
    mkdir("/sys/kernel/config/usb_gadget/9pfs/configs/c.1/strings/0x409")?;

    let function = format!("/sys/kernel/config/usb_gadget/9pfs/functions/usb9pfs.{device}");
    let link = format!("/sys/kernel/config/usb_gadget/9pfs/configs/c.1/usb9pfs.{device}");
    mkdir(&function)?;
    symlink(&function, &link)?;

    println!("Attaching 9pfs gatget to UDC {udc}");
    write_file("/sys/kernel/config/usb_gadget/9pfs/UDC", &udc)?;

    let d = time::Duration::new(1, 0);
    thread::sleep(d);
    Ok(())
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

    if !options.rootfstype.is_none()
        && options.rootfstype.as_ref().unwrap() == "9p"
        && !options.rootflags.is_none()
        && options.rootflags.as_ref().unwrap().contains("trans=usbg")
    {
        if options.root.is_none() {
            return Err("Missing root= for 9p!".into());
        }
        setup_9pfs_gadget(&options.root.as_ref().unwrap())?;
    }

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
