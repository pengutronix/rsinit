// SPDX-License-Identifier: GPL-2.0-only

use std::fs::read_dir;
use std::os::unix::fs::symlink;
use std::thread;
use std::time::Duration;

use log::debug;

use crate::cmdline::CmdlineOptions;
use crate::mount::mount_apivfs;
use crate::util::{mkdir, write_file};
use crate::Result;

fn setup_9pfs_gadget(options: &mut CmdlineOptions) -> Result<()> {
    debug!("Initializing USB 9pfs gadget ...");

    let udc = if let Some(device) = &options.root {
        device.to_owned()
    } else {
        read_dir("/sys/class/udc")
            .map_err(|e| format!("Failed to list /sys/class/udc: {e}"))?
            .next()
            .ok_or("No UDC found to attach the 9pfs gadget".to_string())?
            .map_err(|e| format!("Failed to inspect the first entry in /sys/class/udc: {e}"))?
            .file_name()
            .into_string()
            .map_err(|e| format!("invalid utf-8 in file name: {e:?}"))?
    }
    .as_bytes()
    .escape_ascii()
    .to_string();

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

    let function = format!("/sys/kernel/config/usb_gadget/9pfs/functions/usb9pfs.{udc}");
    let link = format!("/sys/kernel/config/usb_gadget/9pfs/configs/c.1/usb9pfs.{udc}");
    mkdir(&function)?;
    symlink(&function, &link)?;

    debug!("Attaching 9pfs gatget to UDC {udc}",);
    write_file("/sys/kernel/config/usb_gadget/9pfs/UDC", &udc)?;

    thread::sleep(Duration::from_secs(1));

    options.root = Some(udc);

    Ok(())
}

pub fn prepare_9pfs_gadget(options: &mut CmdlineOptions) -> Result<bool> {
    if options.rootfstype.as_deref() == Some("9p")
        && options
            .rootflags
            .as_deref()
            .is_some_and(|flags| flags.contains("trans=usbg"))
    {
        setup_9pfs_gadget(options)?;
        Ok(true)
    } else {
        Ok(false)
    }
}
