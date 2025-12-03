// SPDX-FileCopyrightText: 2025 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::fs::{create_dir, read_to_string};
use std::path::Path;
use std::thread;
use std::time;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn mkdir(dir: &str) -> Result<()> {
    if !Path::new(dir).exists() {
        if let Err(e) = create_dir(dir) {
            return Err(format!("Failed to create {dir}: {e}",).into());
        }
    }
    Ok(())
}

pub fn read_file(filename: &str) -> std::result::Result<String, String> {
    read_to_string(filename).map_err(|e| format!("Failed to read {filename}: {e}"))
}

pub fn wait_for_device(root_device: &str) -> Result<()> {
    let duration = time::Duration::from_millis(5);
    let path = Path::new(&root_device);

    for _ in 0..1000 {
        if path.exists() {
            return Ok(());
        }

        thread::sleep(duration);
    }

    Err("Timeout reached while waiting for the device".into())
}
