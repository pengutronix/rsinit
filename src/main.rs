// SPDX-FileCopyrightText: 2024 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::env;

extern crate rsinit;

use rsinit::init::{finalize, init, setup_early};
#[cfg(feature = "systemd")]
use rsinit::systemd::shutdown;
use rsinit::util::Result;

fn main() -> Result<()> {
    setup_early()?;

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
