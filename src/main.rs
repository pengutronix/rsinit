// SPDX-FileCopyrightText: 2024 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::env;

extern crate rsinit;

use rsinit::init::InitContext;
use rsinit::util::Result;

fn main() -> Result<()> {
    let mut init = InitContext::new()?;

    let cmd = env::args().next().ok_or("No cmd to run as found")?;
    init.run(&cmd);

    Ok(())
}
