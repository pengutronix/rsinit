// SPDX-FileCopyrightText: 2024 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

extern crate rsinit;

use rsinit::init::InitContext;
use rsinit::util::Result;

fn main() -> Result<()> {
    InitContext::new()?.run_from_env()
}
