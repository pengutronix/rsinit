// SPDX-License-Identifier: GPL-2.0-only
use crate::cmdline::CmdlineOptions;
use crate::Result;
use std::fs::{create_dir, read_dir, write};
use std::os::unix::fs::symlink;
use std::{thread, time};

fn mkdir(dir: &str) -> Result<()> {
    create_dir(dir).map_err(|e| format!("Failed to create {dir}: {e}"))?;
    Ok(())
}

fn write_file<C: AsRef<[u8]>>(path: &str, content: C) -> Result<()> {
    write(path, content).map_err(|e| format!("Failed to write to {path}: {e}"))?;
    Ok(())
}

fn setup_9pfs_gadget(device: &String) -> Result<()> {
    println!("Initializing USB 9pfs gadget ...");

    let mut udc = String::new();
    for entry in
        read_dir("/sys/class/udc").map_err(|e| format!("Failed to list /sys/class/udc: {e}"))?
    {
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

pub fn prepare_9pfs_gadget(options: &CmdlineOptions) -> Result<()> {
    if options.rootfstype.as_deref() == Some("9p")
        && options.rootflags.as_deref() == Some("trans=usbg")
    {
        if let Some(root) = &options.root {
            setup_9pfs_gadget(root)
        } else {
            Err("Missing root= for 9p!".into())
        }
    } else {
        Ok(())
    }
}
