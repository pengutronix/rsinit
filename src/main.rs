// SPDX-FileCopyrightText: 2024 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::path::Path;
use std::{cell::RefCell, env};

extern crate rsinit;

use log::{error, info};
use nix::mount::MsFlags;
use rsinit::mount::do_mount;
#[cfg(feature = "systemd")]
use rsinit::systemd::shutdown;
use rsinit::util::Result;
use rsinit::{
    cmdline::ensure_value,
    init::{InitContext, InitContextCallbacks},
};

#[derive(Default)]
struct CustomArgs {
    bind_mount: Vec<String>,
}

fn main() -> Result<()> {
    // This object needs to be alive as long as the InitContext is alive! The RefCell allows us to
    // handout multiple mutable references in the callbacks.
    let bind_mount_args = RefCell::new(CustomArgs::default());

    // First option - pass callback object
    let callbacks = InitContextCallbacks {
        cmdline_cb: vec![],
        post_setup_cb: vec![],
        post_root_mount_cb: vec![Box::new(|_| {
            info!("I'm lucky, I was the first callback to be added!");
            info!(
                "Let me show you the bind mounts collected so far: {:?}",
                bind_mount_args.borrow().bind_mount
            );
            Ok(())
        })],
    };

    let mut init = InitContext::new(Some(callbacks))?;

    // Second option - set callbacks via setter methods
    init.add_cmdline_cb(Box::new(|key, value| {
        if key == "rsinit.bind" {
            bind_mount_args
                .borrow_mut()
                .bind_mount
                .push(ensure_value(key, value)?.to_string())
        }
        Ok(())
    }));

    init.add_post_setup_cb(Box::new(|options| {
        info!(
            "Beep, beep! Post setup callback running... I'll just print the options: {options:#?}"
        );
        Ok(())
    }));

    init.add_post_root_mount_cb(Box::new(|_| {
        for src in &bind_mount_args.borrow().bind_mount {
            if !Path::new(src).exists() {
                error!("Can't bind mount {} as it doesn't exist", src);

                continue;
            }

            let dst = format!("/root{src}");

            info!("Bind mounting {src} to {dst}");

            do_mount(Some(src), &dst, None, MsFlags::MS_BIND, None)?;
        }
        Ok(())
    }));

    setup_custom_stuff(&mut init);

    let cmd = env::args().next().ok_or("No cmd to run as found")?;
    println!("Running {cmd}...");

    if let Err(e) = match cmd.as_str() {
        #[cfg(feature = "systemd")]
        "/shutdown" => shutdown(),
        _ => init.run(),
    } {
        println!("{e}");
    }

    Ok(())
}

fn setup_custom_stuff(init: &mut InitContext) {
    init.add_cmdline_cb(Box::new(|key, value| {
        if key == "custom.option" {
            let val = ensure_value(key, value)?;
            info!("Custom option provided: {val}");
        }
        Ok(())
    }));
}
