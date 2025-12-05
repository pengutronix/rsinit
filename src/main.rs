// SPDX-FileCopyrightText: 2024 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::env;

extern crate rsinit;

use rsinit::init::InitContext;
#[cfg(feature = "systemd")]
use rsinit::systemd::shutdown;
use rsinit::util::Result;

fn main() -> Result<()> {
    let mut init = InitContext::new()?;

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
