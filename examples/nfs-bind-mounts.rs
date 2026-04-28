// SPDX-FileCopyrightText: 2026 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::net::IpAddr;
use std::{cell::RefCell, env};

extern crate rsinit;

use log::{error, info};
use nix::mount::MsFlags;
use rsinit::mount::do_mount;
#[cfg(feature = "systemd")]
use rsinit::systemd::shutdown;
use rsinit::util::Result;
use rsinit::{cmdline::ensure_value, init::InitContext};

fn run(ctx: &mut InitContext, mount_args: &RefCell<MountArgs>) -> Result<()> {
    let cmd = env::args().next().ok_or("No cmd to run as found")?;
    println!("Running {cmd}...");

    match cmd.as_str() {
        #[cfg(feature = "systemd")]
        "/shutdown" => shutdown(),
        _ => {
            ctx.setup()?;

            #[cfg(any(feature = "dmverity", feature = "usb9pfs"))]
            ctx.prepare_aux()?;

            ctx.mount_root()?;

            mount_args.borrow_mut().do_mounts()?;

            ctx.finish()?;
            Ok(())
        }
    }
}

fn main() -> Result<()> {
    // This object needs to be alive as long as the InitContext is alive! The RefCell allows us to
    // handout multiple mutable references in the callbacks.
    let mount_args = RefCell::new(MountArgs::default());

    let mut ctx = InitContext::new()?;
    ctx.add_cmdline_parser_callback(Box::new(|key, value| {
        mount_args.borrow_mut().parse_cmdline(key, value)
    }));

    if let Err(e) = run(&mut ctx, &mount_args) {
        println!("{e}");
    }

    drop(ctx);

    Ok(())
}

#[derive(Debug, PartialEq)]

struct MountOption {
    source: String,
    destination: String,
    options: String,
}

#[derive(Debug, Default)]
struct MountArgs {
    bind: Vec<MountOption>,
    nfs: Vec<MountOption>,
}

impl MountArgs {
    fn parse_cmdline(&mut self, key: &str, value: Option<&str>) -> Result<()> {
        match key {
            "rsinit.bind" => {
                let val = ensure_value(key, value)?;

                let (src, dst) = val.split_once(',').ok_or(format!(
                    "Bind mount option must be in the format '<source>,<destination>', got: {val}"
                ))?;

                self.bind.push(MountOption {
                    source: src.to_string(),
                    destination: dst.to_string(),
                    options: String::new(),
                });
            }
            "rsinit.nfs" => {
                let val = ensure_value(key, value)?;

                let (src, dst) = val.split_once(',').ok_or(format!(
                    "NFS mount option must be in the format '<host>:<source>,<destination>', got: {val}"
                ))?;

                let (host, _) = src
                    .split_once(':')
                    .ok_or("NFS source must be in the format '<host>:<path>'")?;

                host.parse::<IpAddr>().map_err(|_| {
                    "NFS host must be a valid IP address as DNS lookup is not supported (yet)"
                })?;

                self.nfs.push(MountOption {
                    source: src.to_string(),
                    destination: dst.to_string(),
                    options: format!("addr={host},vers=3,proto=tcp,nolock"),
                });
            }
            _ => {}
        }
        Ok(())
    }

    fn do_mounts(&self) -> Result<()> {
        for MountOption {
            source,
            destination,
            options,
        } in &self.nfs
        {
            info!("NFS mounting {source} to {destination} with options {options}");

            let ret = do_mount(
                Some(source),
                destination,
                Some("nfs"),
                MsFlags::empty(),
                Some(options),
            );

            if ret.is_err() {
                error!("NFS mounting {source} to {destination} failed!");
                error!("In case of ENETUNREACH or ENETDOWN ensure that an IP address is assigned to the network interface.");
                error!("Via DHCP this can be done by adding 'ip=:::::<interface>:dhcp' e.g. 'ip=:::::eth0:dhcp' to the kernel command-line.");
                error!("In case of EHOSTUNREACH check dhcp configuration and that your firewall allows nfs, rpcbind and mountd.");
                error!("Good luck next time!");
            };
            ret?
        }

        for MountOption {
            source,
            destination,
            options: _,
        } in &self.bind
        {
            info!("Bind mounting {source} to {destination}");

            do_mount(Some(source), destination, None, MsFlags::MS_BIND, None)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_bind_args() {
        let mut args = MountArgs::default();

        args.parse_cmdline("rsinit.bind", Some("/lib/modules,/root/lib/modules"))
            .unwrap();

        assert_eq!(
            args.bind,
            &[MountOption {
                source: "/lib/modules".to_string(),
                destination: "/root/lib/modules".to_string(),
                options: String::new(),
            }]
        );
    }

    #[test]
    fn test_nfs_args() {
        let mut args = MountArgs::default();

        args.parse_cmdline(
            "rsinit.nfs",
            Some("192.168.0.1:/full/path/to/lib/modules,/root/lib/modules"),
        )
        .unwrap();

        assert_eq!(
            args.nfs[0],
            MountOption {
                source: "192.168.0.1:/full/path/to/lib/modules".to_string(),
                destination: "/root/lib/modules".to_string(),
                options: "addr=192.168.0.1,vers=3,proto=tcp,nolock".to_string(),
            }
        );
    }
}
