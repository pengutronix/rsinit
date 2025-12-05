// SPDX-FileCopyrightText: 2024 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::fs::{read_dir, write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::{thread, time};

use log::debug;

use crate::cmdline::CmdlineOptions;
use crate::mount::mount_apivfs;
use crate::util::{mkdir, Result};

fn write_file<C: AsRef<[u8]>>(path: &str, content: C) -> Result<()> {
    write(path, content).map_err(|e| format!("Failed to write to {path}: {e}").into())
}

fn setup_9pfs_gadget(device: &String) -> Result<()> {
    debug!("Initializing USB 9pfs gadget ...");

    let udc = read_dir("/sys/class/udc")
        .map_err(|e| format!("Failed to list /sys/class/udc: {e}"))?
        .next()
        .ok_or("No UDC found to attach the 9pfs gadget".to_string())?
        .map_err(|e| format!("Failed to inspect the first entry in /sys/class/udc: {e}"))?
        .file_name();

    mount_apivfs("/sys/kernel/config", "configfs")?;

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

    debug!(
        "Attaching 9pfs gatget to UDC {}",
        udc.as_bytes().escape_ascii()
    );
    write_file(
        "/sys/kernel/config/usb_gadget/9pfs/UDC",
        udc.as_encoded_bytes(),
    )?;

    let d = time::Duration::new(1, 0);
    thread::sleep(d);
    Ok(())
}

pub fn prepare_9pfs_gadget(options: &CmdlineOptions) -> Result<bool> {
    if options.rootfstype.as_deref() == Some("9p")
        && options
            .rootflags
            .as_deref()
            .is_some_and(|flags| flags.contains("trans=usbg"))
    {
        if let Some(root) = &options.root {
            setup_9pfs_gadget(root)?;
            Ok(true)
        } else {
            Err("Missing root= for 9p!".into())
        }
    } else {
        Ok(false)
    }
}
