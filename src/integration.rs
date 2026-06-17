// SPDX-FileCopyrightText: 2025 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::borrow::Borrow;
use std::fs::read_dir;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::Path;

use log::{LevelFilter, Metadata, Record};

use crate::kmsg::KmsgLogger;
use crate::util::Result;

pub fn find_vport() -> Result<File> {
    let entries = read_dir("/sys/class/virtio-ports")
        .map_err(|e| format!("Failed to access /sys/class/virtio-ports: {e}"))?;
    let dev = Path::new("/dev");
    for entry in entries.flatten() {
        let file_name = entry.file_name().into_string().unwrap();
        let device = dev.join(file_name);
        if device.exists() {
            let vport = OpenOptions::new().write(true).open(device)?;
            return Ok(vport);
        }
    }
    Err("No vport device found".into())
}

pub struct IntegrationLogger {
    next: KmsgLogger,
    vport: File,
}

impl IntegrationLogger {
    pub fn new() -> Result<IntegrationLogger> {
        let vport = find_vport()?;
        let kmsg = KmsgLogger::new()?;
        Ok(IntegrationLogger { next: kmsg, vport })
    }
    pub fn enable() -> Result<()> {
        let logger = IntegrationLogger::new()?;
        log::set_boxed_logger(Box::new(logger)).map(|()| log::set_max_level(LevelFilter::Trace))?;
        Ok(())
    }
}

impl log::Log for IntegrationLogger {
    fn enabled(&self, _: &Metadata) -> bool {
        true
    }
    fn log(&self, record: &Record) {
        self.next.log(record);
        let mut msg = json::JsonValue::new_object();
        msg["message"] = record.args().to_string().into();
        let _ = self
            .vport
            .borrow()
            .write_all(format!("{}\0", msg.dump()).as_bytes());
        let _ = self.vport.borrow().flush();
    }
    fn flush(&self) {}
}
