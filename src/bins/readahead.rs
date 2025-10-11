// SPDX-FileCopyrightText: 2024 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

extern crate rsinit;

use clap::Parser;
use log::{LevelFilter, Log, Metadata, Record};
use rsinit::readahead;

struct DebugLogger;

impl Log for DebugLogger {
    fn enabled(&self, _: &Metadata) -> bool {
        println!("hit");
        true
    }
    fn log(&self, record: &Record) {
        println!("{} - {}", record.level(), record.args());
    }
    fn flush(&self) {}
}

static LOGGER: DebugLogger = DebugLogger;

#[derive(Parser, Debug)]
#[command(about, long_about = None)]
struct Args {
    #[arg(short, long)]
    trace: bool,
    file: String,
}

fn run() -> Result<()> {
    log::set_logger(&LOGGER)?;
    log::set_max_level(LevelFilter::Trace);
    let args = Args::parse();
    if !args.trace {
        if let Some(input) = readahead::readahead_open(&args.file)? {
            readahead::readahead_load(&input);
        } else {
            println!("Trace file {} missing!", args.file);
        }
    } else {
        readahead::enable_readahead_tracing(true)?;
        let readahead = readahead::readahead_trace(true)?;
        readahead::readahead_write(&readahead, &args.file)?;
    }
    Ok(())
}

fn main() -> Result<()> {
    if let Err(e) = run() {
        println!("Failed: {e}");
    }
    Ok(())
}
