// SPDX-FileCopyrightText: 2024 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::env;
use std::panic::set_hook;

extern crate rsinit;

use rsinit::init::{finalize, init, setup_console};
#[cfg(feature = "systemd")]
use rsinit::systemd::shutdown;
use rsinit::util::Result;

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
