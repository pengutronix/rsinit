// SPDX-FileCopyrightText: 2024 The rsinit Authors
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
use crate::util::{read_file, wait_for_device, Result};

const DM_VERSION_MAJOR: u32 = 4;

const DM_MAX_TYPE_NAME: usize = 16;
const DM_NAME_LEN: usize = 128;
const DM_UUID_LEN: usize = 129;

struct VerityParams<'a> {
    data_blocks: &'a str,
    data_sectors: u64,
    data_block_size: &'a str,
    hash_block_size: &'a str,
    hash_algorithm: &'a str,
    salt: &'a str,
    root_hash: &'a str,
    verity_params: (usize, &'a str),
}

impl<'a> VerityParams<'a> {
    fn from_string(params: &'a str) -> Result<VerityParams<'a>> {
        let mut data_blocks = "";
        let mut data_sectors = 0;
        let mut data_block_size = "";
        let mut hash_block_size = "";
        let mut hash_algorithm = "";
        let mut salt = "";
        let mut root_hash = "";
        let mut verity_params = (1, "ignore_zero_blocks");

        for line in params.lines() {
            let (key, value) = match line.split_once('=') {
                Some((k, v)) => (k.trim(), v.trim()),
                None => continue,
            };

            match key {
                "VERITY_DATA_BLOCKS" => data_blocks = value,
                "VERITY_DATA_SECTORS" => {
                    data_sectors = value.parse::<u64>().map_err(|e| {
                        format!("Failed to parse 'VERITY_DATA_SECTORS={data_sectors}: {e}")
                    })?
                }
                "VERITY_DATA_BLOCK_SIZE" => data_block_size = value,
                "VERITY_HASH_BLOCK_SIZE" => hash_block_size = value,
                "VERITY_HASH_ALGORITHM" => hash_algorithm = value,
                "VERITY_SALT" => salt = value,
                "VERITY_ROOT_HASH" => root_hash = value,
                "VERITY_PARAMS" => verity_params = (value.split_ascii_whitespace().count(), value),
                _ => (),
            }
        }
        Ok(VerityParams {
            data_blocks,
            data_sectors,
            data_block_size,
            hash_block_size,
            hash_algorithm,
            salt,
            root_hash,
            verity_params,
        })
    }
}

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

impl DmIoctl {
    fn uuid(device: &str) -> Result<String> {
        let rand = {
            let mut rand = [0u8; 16];
            getrandom(&mut rand).map_err(|_| "Getrandom failed")?;
            rand
        };
        let mut uuid_str = String::from("rsinit-verity-root-");
        for x in rand {
            uuid_str.push_str(format!("{x:02x}").as_str());
        }
        uuid_str.push('-');
        uuid_str.push_str(device.rsplit_once('/').unwrap_or(("", device)).1);
        Ok(uuid_str)
    }

    fn init_header(&mut self, size: u32, flags: u32, uuid: &str) -> Result<()> {
        let len = usize::min(uuid.len(), DM_UUID_LEN - 1);
        let uuid = &uuid.as_bytes()[..len];
        self.version[0] = DM_VERSION_MAJOR;
        self.data_size = size;
        self.data_start = u32::try_from(size_of::<DmIoctl>())?;
        self.flags = flags;
        self.uuid[..uuid.len()].copy_from_slice(uuid);
        Ok(())
    }

    fn new(uuid: &str) -> Result<DmIoctl> {
        let mut create_data = DmIoctl::default();
        create_data.init_header(u32::try_from(size_of::<DmIoctl>())?, 0, uuid)?;
        Ok(create_data)
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

impl DmTableLoad {
    fn new(params: &VerityParams, root_device: &str, uuid: &str) -> Result<DmTableLoad> {
        let mut table_load_data = DmTableLoad::default();
        table_load_data.header.init_header(
            u32::try_from(size_of::<DmTableLoad>())?,
            DM_READONLY_FLAG,
            uuid,
        )?;
        table_load_data.header.target_count = 1;
        table_load_data.target_spec.status = 0;
        table_load_data.target_spec.sector_start = 0;
        table_load_data.target_spec.length = params.data_sectors;

        let target_type = "verity\0".as_bytes();
        table_load_data.target_spec.target_type[..target_type.len()].copy_from_slice(target_type);

        let table_str = format!(
            "1 {} {} {} {} {} {} {} {} {} {} {}\0",
            root_device,
            root_device,
            params.data_block_size,
            params.hash_block_size,
            params.data_blocks,
            params.data_blocks,
            params.hash_algorithm,
            params.root_hash,
            params.salt,
            params.verity_params.0,
            params.verity_params.1
        );
        let table = table_str.as_bytes();
        table_load_data.params[..table.len()].copy_from_slice(table);
        debug!("Configuring dm-verity with table = '{table_str}'");
        Ok(table_load_data)
    }
}

const DM_READONLY_FLAG: u32 = 1;

const DM_DEV_CREATE_CMD: u8 = 3;
const DM_DEV_SUSPEND_CMD: u8 = 6;
const DM_TABLE_LOAD_CMD: u8 = 9;

ioctl_readwrite!(dm_dev_create, 0xfd, DM_DEV_CREATE_CMD, DmIoctl);
ioctl_readwrite!(dm_table_load, 0xfd, DM_TABLE_LOAD_CMD, DmIoctl);
ioctl_readwrite!(dm_dev_suspend, 0xfd, DM_DEV_SUSPEND_CMD, DmIoctl);

pub fn prepare_dmverity(options: &mut CmdlineOptions) -> Result<bool> {
    if !Path::new("/verity-params").exists() {
        return Ok(false);
    }
    match options.rootfstype.as_deref() {
        Some("nfs") | Some("9p") => return Ok(false),
        _ => (),
    }
    let root_device = options
        .verity_root
        .as_ref()
        .ok_or("No verity root device")?;
    wait_for_device(root_device)?;

    let param_data = read_file("/verity-params")?;
    let params = VerityParams::from_string(&param_data)?;

    debug!(
        "Configuring dm-verity rootfs with root-hash = {}",
        params.root_hash
    );

    let f = OpenOptions::new()
        .write(true)
        .open("/dev/mapper/control")
        .map_err(|e| format!("Failed to open /dev/mapper/control: {e}"))?;
    let dm_fd = f.into_raw_fd();

    let uuid = DmIoctl::uuid(root_device)?;
    let mut create_data = DmIoctl::new(&uuid)?;
    let name = "verity-rootfs\0".as_bytes();
    create_data.name[..name.len()].copy_from_slice(name);

    unsafe { dm_dev_create(dm_fd, &mut create_data) }
        .map_err(|e| format!("Failed to create dm device: {e}"))?;

    let mut table_load_data = DmTableLoad::new(&params, root_device, &uuid)?;

    unsafe { dm_table_load(dm_fd, &mut table_load_data.header) }
        .map_err(|e| format!("Failed to load dm table: {e}"))?;

    let mut suspend_data = DmIoctl::new(&uuid)?;

    unsafe { dm_dev_suspend(dm_fd, &mut suspend_data) }
        .map_err(|e| format!("Failed to suspend dm device: {e}"))?;

    options.root = Some(format!("/dev/dm-{}", minor(suspend_data.dev)));

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic() {
        let param_data = "
VERITY_DATA_BLOCKS=26624
VERITY_DATA_BLOCK_SIZE=4096
VERITY_HASH_BLOCK_SIZE=4096
VERITY_HASH_ALGORITHM=sha256
VERITY_SALT=a224908192cf3202b8c3eda4a5f5c320a82f2f750681e1cb30bac367b08f3973
VERITY_ROOT_HASH=c63dc40d73bdbb4093e3c54592182a6b74ea9e611145ba498033b696c6e072df
VERITY_DATA_SECTORS=212992";

        let root_device = "/dev/mmcblk3p2";
        let uuid = "rsinit-verity-root-test-uuid".to_string();
        let create_data = DmIoctl::new(&uuid).unwrap();

        let expected_uuid = *b"rsinit-verity-root-test-uuid\0";
        assert_eq!(create_data.uuid[..expected_uuid.len()], expected_uuid);

        let params = VerityParams::from_string(param_data).expect("parsing params failed");
        let table_load_data = DmTableLoad::new(&params, root_device, &uuid).unwrap();
        let expected_table = *b"1 /dev/mmcblk3p2 /dev/mmcblk3p2 4096 4096 26624 26624 sha256 c63dc40d73bdbb4093e3c54592182a6b74ea9e611145ba498033b696c6e072df a224908192cf3202b8c3eda4a5f5c320a82f2f750681e1cb30bac367b08f3973 1 ignore_zero_blocks\0";
        assert_eq!(
            table_load_data.params[..expected_table.len()],
            expected_table
        );
        assert_eq!(table_load_data.target_spec.length, 212992);
    }

    #[test]
    fn test_params() {
        let param_data = "
            VERITY_DATA_BLOCKS = 26624
            VERITY_DATA_BLOCK_SIZE = 4096
            VERITY_HASH_BLOCK_SIZE = 4096
            VERITY_HASH_ALGORITHM = sha256
            VERITY_SALT = a224908192cf3202b8c3eda4a5f5c320a82f2f750681e1cb30bac367b08f3973
            VERITY_ROOT_HASH = c63dc40d73bdbb4093e3c54592182a6b74ea9e611145ba498033b696c6e072df
            VERITY_DATA_SECTORS = 212992
            VERITY_PARAMS = ignore_zero_blocks  panic_on_corruption ";

        let root_device = "/dev/mmcblk3p2";
        let uuid = "rsinit-verity-root-test-uuid".to_string();

        let params = VerityParams::from_string(param_data).expect("parsing params failed");
        let table_load_data = DmTableLoad::new(&params, root_device, &uuid).unwrap();
        let expected_table = *b"1 /dev/mmcblk3p2 /dev/mmcblk3p2 4096 4096 26624 26624 sha256 c63dc40d73bdbb4093e3c54592182a6b74ea9e611145ba498033b696c6e072df a224908192cf3202b8c3eda4a5f5c320a82f2f750681e1cb30bac367b08f3973 2 ignore_zero_blocks  panic_on_corruption\0";
        assert_eq!(
            table_load_data.params[..expected_table.len()],
            expected_table
        );
    }
}
