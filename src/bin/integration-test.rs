// SPDX-FileCopyrightText: 2026 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::fs::read_dir;
use std::io;
use std::process;
use std::process::exit;

use nix::errno::Errno;
use nix::libc::{EXIT_FAILURE, EXIT_SUCCESS};
use nix::sched::unshare;
use nix::sched::CloneFlags;
use nix::sys::reboot::{reboot, RebootMode};
use nix::sys::wait::waitpid;
use nix::sys::wait::WaitStatus;
use nix::unistd::fork;
use nix::unistd::ForkResult;
use rsinit::integration::find_vport;
use rsinit::util::{read_file, Result};

fn parse_mountinfo() -> Result<json::JsonValue> {
    let data = read_file("/proc/self/mountinfo")?;
    let mut mountinfo = json::JsonValue::new_array();
    for line in data.lines() {
        let mut mount = json::JsonValue::new_object();
        let mut fields = line.split(" ");
        mount["mount-id"] = fields.next().into();
        mount["parent-id"] = fields.next().into();
        mount["st_dev"] = fields.next().into();
        mount["root"] = fields.next().into();
        mount["mount-point"] = fields.next().into();
        mount["mount-options"] = fields
            .next()
            .unwrap_or("")
            .split(",")
            .collect::<Vec<_>>()
            .into();
        let mut optional_fields = json::JsonValue::new_array();
        loop {
            let field = fields.next();
            if field == Some("-") {
                break;
            }
            optional_fields.push(field)?;
        }
        mount["optional-fields"] = optional_fields;
        mount["filesystem-type"] = fields.next().into();
        mount["mount-source"] = fields.next().into();
        mount["super-options"] = fields
            .next()
            .unwrap_or("")
            .split(",")
            .collect::<Vec<_>>()
            .into();
        mountinfo.push(mount)?;
    }
    Ok(mountinfo)
}

fn collect_block_devices() -> Result<json::JsonValue> {
    let mut devices = json::JsonValue::new_object();
    let entries = read_dir("/sys/dev/block")?;
    for entry in entries.flatten() {
        let target = entry.path().read_link()?;
        devices[entry.file_name().to_string_lossy().to_string()] = target
            .file_name()
            .map_or(String::new(), |v| v.to_string_lossy().to_string())
            .into();
    }
    Ok(devices)
}

fn collect_root_mount() -> Result<json::JsonValue> {
    match unsafe { fork() } {
        Ok(ForkResult::Child) => match unshare(CloneFlags::CLONE_NEWUSER) {
            Ok(_) => exit(EXIT_SUCCESS),
            Err(Errno::EINVAL) => {
                println!("Kernel does not support user namespaces, ignoring.");
                exit(EXIT_SUCCESS);
            }
            _ => exit(EXIT_FAILURE),
        },
        Ok(ForkResult::Parent { child, .. }) => match waitpid(child, None) {
            Ok(WaitStatus::Exited(_, status)) => {
                let user_ns = status == EXIT_SUCCESS;
                return Ok(json::JsonValue::Boolean(user_ns));
            }
            // Everything but a clean exit is a failure
            _ => return Err("waitpid failed!".into()),
        },
        Err(err) => {
            return Err(format!("Forking of user namespace canary process failed: {err}").into())
        }
    }
}

fn main() -> Result<()> {
    println!("Collecting system state...");

    let mut data = json::JsonValue::new_object();
    data["mountinfo"] = parse_mountinfo()?;
    data["block-devices"] = collect_block_devices()?;
    data["root-mount"] = collect_root_mount()?;

    let pid = process::id();
    if pid == 1 {
        let mut file = find_vport()?;
        data.write(&mut file)?;
    } else {
        data.write(&mut io::stdout())?;
    }

    if pid == 1 {
        println!("Power off...");
        reboot(RebootMode::RB_POWER_OFF).map_err(|e| format!("reboot failed: {e}"))?;
    }
    Ok(())
}
