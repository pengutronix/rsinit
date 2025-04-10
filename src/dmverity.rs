// SPDX-License-Identifier: GPL-2.0-only

use std::fs::OpenOptions;
use std::mem::size_of;
use std::os::fd::IntoRawFd;
use std::path::Path;

use getrandom::getrandom;
use log::debug;
use nix::ioctl_readwrite;
use nix::libc::dev_t;
use nix::sys::stat::minor;

use crate::cmdline::CmdlineOptions;
use crate::{read_file, wait_for_device, Result};

const DM_VERSION_MAJOR: u32 = 4;

const DM_MAX_TYPE_NAME: usize = 16;
const DM_NAME_LEN: usize = 128;
const DM_UUID_LEN: usize = 129;

#[repr(C)]
struct DmIoctl {
    version: [u32; 3],
    data_size: u32,
    data_start: u32,
    target_count: u32,
    open_count: u32,
    flags: u32,
    event_nr: u32,
    padding: u32,
    dev: dev_t,
    name: [u8; DM_NAME_LEN],
    uuid: [u8; DM_UUID_LEN],
    data: [u8; 7],
}

impl Default for DmIoctl {
    fn default() -> Self {
        DmIoctl {
            version: [0; 3],
            data_size: u32::default(),
            data_start: u32::default(),
            target_count: u32::default(),
            open_count: u32::default(),
            flags: u32::default(),
            event_nr: u32::default(),
            padding: u32::default(),
            dev: dev_t::default(),
            name: [0; DM_NAME_LEN],
            uuid: [0; DM_UUID_LEN],
            data: [0; 7],
        }
    }
}

#[repr(C)]
struct DmTargetSpec {
    sector_start: u64,
    length: u64,
    status: u32,
    next: u32,
    target_type: [u8; DM_MAX_TYPE_NAME],
}

impl Default for DmTargetSpec {
    fn default() -> Self {
        DmTargetSpec {
            sector_start: u64::default(),
            length: u64::default(),
            status: u32::default(),
            next: u32::default(),
            target_type: [0; DM_MAX_TYPE_NAME],
        }
    }
}

#[repr(C)]
struct DmTableLoad {
    header: DmIoctl,
    target_spec: DmTargetSpec,
    params: [u8; 1024],
}

impl Default for DmTableLoad {
    fn default() -> Self {
        DmTableLoad {
            header: DmIoctl::default(),
            target_spec: DmTargetSpec::default(),
            params: [0; 1024],
        }
    }
}

const DM_READONLY_FLAG: u32 = 1;

const DM_DEV_CREATE_CMD: u8 = 3;
const DM_DEV_SUSPEND_CMD: u8 = 6;
const DM_TABLE_LOAD_CMD: u8 = 9;

ioctl_readwrite!(dm_dev_create, 0xfd, DM_DEV_CREATE_CMD, DmIoctl);
ioctl_readwrite!(dm_table_load, 0xfd, DM_TABLE_LOAD_CMD, DmIoctl);
ioctl_readwrite!(dm_dev_suspend, 0xfd, DM_DEV_SUSPEND_CMD, DmIoctl);

fn init_header(header: &mut DmIoctl, size: u32, flags: u32, uuid: &[u8]) -> Result<()> {
    header.version[0] = DM_VERSION_MAJOR;
    header.data_size = size;
    header.data_start = u32::try_from(size_of::<DmIoctl>())?;
    header.flags = flags;
    header.uuid[..uuid.len()].copy_from_slice(uuid);
    Ok(())
}

pub fn prepare_dmverity(options: &mut CmdlineOptions) -> Result<bool> {
    if !Path::new("/verity-params").exists() {
        return Ok(false);
    }
    if options.root.is_none() {
        return Ok(false);
    }
    let root_device = options.root.as_ref().ok_or("No root device")?;
    match options.rootfstype.as_deref() {
        Some("nfs") | Some("9p") => return Ok(false),
        _ => wait_for_device(root_device)?,
    }

    let mut data_blocks = "";
    let mut data_sectors = "";
    let mut data_block_size = "";
    let mut hash_block_size = "";
    let mut hash_algorithm = "";
    let mut salt = "";
    let mut root_hash = "";

    let params = read_file("/verity-params")?;
    for line in params.lines() {
        match line.split_once('=') {
            None => continue,
            Some((key, value)) => match key {
                "VERITY_DATA_BLOCKS" => data_blocks = value,
                "VERITY_DATA_SECTORS" => data_sectors = value,
                "VERITY_DATA_BLOCK_SIZE" => data_block_size = value,
                "VERITY_HASH_BLOCK_SIZE" => hash_block_size = value,
                "VERITY_HASH_ALGORITHM" => hash_algorithm = value,
                "VERITY_SALT" => salt = value,
                "VERITY_ROOT_HASH" => root_hash = value,
                _ => (),
            },
        }
    }

    debug!("Configuring dm-verity rootfs with root-hash = {root_hash}");

    let f = OpenOptions::new()
        .write(true)
        .open("/dev/mapper/control")
        .map_err(|e| format!("Failed to open /dev/mapper/control: {e}"))?;
    let dm_fd = f.into_raw_fd();

    let mut rand = [0u8; 16];
    if getrandom(&mut rand).is_err() {
        return Err("Getrandom failed".into());
    };
    let mut uuid_str = String::from("rsinit-verity-root-");
    for x in rand {
        uuid_str.push_str(format!("{:02x}", x).as_str());
    }
    uuid_str.push('-');
    uuid_str.push_str(root_device.rsplit_once('/').unwrap_or(("", root_device)).1);
    let len = usize::min(uuid_str.len(), DM_UUID_LEN - 1);
    let uuid = &uuid_str.as_bytes()[..len];

    let mut create_data = DmIoctl::default();
    init_header(
        &mut create_data,
        u32::try_from(size_of::<DmIoctl>())?,
        0,
        uuid,
    )?;

    let name = "verity-rootfs\0".as_bytes();
    create_data.name[..name.len()].copy_from_slice(name);

    unsafe { dm_dev_create(dm_fd, &mut create_data) }
        .map_err(|e| format!("Failed to create dm device: {e}"))?;

    let mut table_load_data = DmTableLoad::default();
    init_header(
        &mut table_load_data.header,
        u32::try_from(size_of::<DmTableLoad>())?,
        DM_READONLY_FLAG,
        uuid,
    )?;
    table_load_data.header.target_count = 1;
    table_load_data.target_spec.status = 0;
    table_load_data.target_spec.sector_start = 0;
    table_load_data.target_spec.length = data_sectors
        .parse::<u64>()
        .map_err(|e| format!("Failed to parse 'VERITY_DATA_SECTORS={data_sectors}: {e}"))?;
    let target_type = "verity\0".as_bytes();
    table_load_data.target_spec.target_type[..target_type.len()].copy_from_slice(target_type);

    let table_str = format!("1 {root_device} {root_device} {data_block_size} {hash_block_size} {data_blocks} {data_blocks} {hash_algorithm} {root_hash} {salt} 1 ignore_zero_blocks\0");
    let table = table_str.as_bytes();
    table_load_data.params[..table.len()].copy_from_slice(table);

    unsafe { dm_table_load(dm_fd, &mut table_load_data.header) }
        .map_err(|e| format!("Failed to load dm table: {e}"))?;

    let mut suspend_data = DmIoctl::default();
    init_header(
        &mut suspend_data,
        u32::try_from(size_of::<DmIoctl>())?,
        0,
        uuid,
    )?;

    unsafe { dm_dev_suspend(dm_fd, &mut suspend_data) }
        .map_err(|e| format!("Failed to suspend dm device: {e}"))?;

    options.root = Some(format!("/dev/dm-{}", minor(suspend_data.dev)));

    Ok(true)
}
