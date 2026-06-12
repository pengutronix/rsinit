// SPDX-FileCopyrightText: 2025 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::borrow::Borrow;
use std::fs::{File, OpenOptions};
use std::io::Write as _;

use log::{Level, LevelFilter, Metadata, Record};

use crate::util::Result;

pub struct KmsgLogger {
    kmsg: File,
}

impl log::Log for KmsgLogger {
    fn enabled(&self, _: &Metadata) -> bool {
        true
    }
    fn log(&self, record: &Record) {
        let level = match record.level() {
            Level::Error => 3,
            Level::Warn => 4,
            /* 5 == notice has no equivalent */
            Level::Info => 6,
            Level::Debug | Level::Trace => 7,
        } | (1 << 3);
        /* Format first to ensure that the whole message is written with
         * one write() system-call */
        let msg = format!("<{level}>rsinit: {}", record.args());
        let _ = self.kmsg.borrow().write_all(msg.as_bytes());
    }
    fn flush(&self) {}
}

impl KmsgLogger {
    pub fn new() -> Result<KmsgLogger> {
        let kmsg = OpenOptions::new().write(true).open("/dev/kmsg")?;
        Ok(KmsgLogger { kmsg })
    }
    pub fn enable() -> Result<()> {
        let logger = KmsgLogger::new()?;
        log::set_boxed_logger(Box::new(logger)).map(|()| log::set_max_level(LevelFilter::Trace))?;
        Ok(())
    }
}
