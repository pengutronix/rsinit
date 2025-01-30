// SPDX-License-Identifier: GPL-2.0-only

use nix::mount::MsFlags;

use crate::{read_file, Result};

pub struct CmdlineOptions {
    pub root: Option<String>,
    pub rootfstype: Option<String>,
    pub rootflags: Option<String>,
    pub rootfsflags: MsFlags,
    pub nfsroot: Option<String>,
    pub init: String,
    pub cleanup: bool,
}

const SBIN_INIT: &str = "/sbin/init";

impl Default for CmdlineOptions {
    fn default() -> CmdlineOptions {
        CmdlineOptions {
            root: None,
            rootfstype: None,
            rootflags: None,
            rootfsflags: MsFlags::MS_RDONLY,
            nfsroot: None,
            init: SBIN_INIT.into(),
            cleanup: true,
        }
    }
}

fn ensure_value<'a>(key: &str, value: Option<&'a str>) -> Result<&'a str> {
    match value {
        None => Err(format!("Cmdline option '{key}' must have an argument!").into()),
        Some(s) => Ok(s),
    }
}

fn parse_option(key: &str, value: Option<&str>, options: &mut CmdlineOptions) -> Result<()> {
    match key {
        "root" => options.root = Some(ensure_value(key, value)?.to_string()),
        "rootfstype" => options.rootfstype = Some(ensure_value(key, value)?.to_string()),
        "rootflags" => options.rootflags = value.map(str::to_string),
        "ro" => options.rootfsflags.insert(MsFlags::MS_RDONLY),
        "rw" => options.rootfsflags.remove(MsFlags::MS_RDONLY),
        "nfsroot" => options.nfsroot = Some(ensure_value(key, value)?.to_string()),
        "init" => options.init = ensure_value(key, value)?.into(),
        _ => (),
    }
    Ok(())
}

fn parse_nfsroot(options: &mut CmdlineOptions) -> Result<()> {
    let nfsroot_option = options
        .nfsroot
        .as_ref()
        .ok_or("Missing nfsroot command-line option!")?;
    let mut rootflags = String::from("nolock");
    let mut nfsroot = match nfsroot_option.split_once(',') {
        None => nfsroot_option.to_string(),
        Some((root, flags)) => {
            rootflags.push(',');
            rootflags.push_str(flags);
            root.to_string()
        }
    };
    rootflags.push_str(",addr=");
    if !nfsroot.contains(':') {
        let pnp = read_file("/proc/net/pnp")?;
        for line in pnp.lines() {
            match line.split_once(' ') {
                None => continue,
                Some((key, value)) => {
                    if key == "bootserver" {
                        nfsroot = value.to_owned() + ":" + &nfsroot;
                        rootflags.push_str(value);
                        break;
                    }
                }
            }
        }
    } else {
        let (bootserver, _) = nfsroot
            .split_once(':')
            .ok_or("Failed to split out path from nfsroot parameter")?;
        rootflags.push_str(bootserver);
    }
    options.root = Some(nfsroot.to_string());
    options.rootflags = Some(rootflags);
    options.rootfstype = Some("nfs".to_string());
    Ok(())
}

pub fn parse_cmdline(cmdline: String, options: &mut CmdlineOptions) -> Result<()> {
    let mut have_value = false;
    let mut quoted = false;
    let mut key = &cmdline[0..0];
    let mut start = 0;

    for (i, c) in cmdline.chars().enumerate() {
        let mut skip = false;
        match c {
            '=' => {
                if !have_value {
                    skip = true;
                    key = &cmdline[start..i];
                    start = i;
                }
                have_value = true;
            }
            '"' => {
                quoted = !quoted;
                skip = true;
            }
            ' ' | '\n' => {
                if !quoted {
                    if !have_value {
                        key = &cmdline[start..i];
                    }
                    if !key.is_empty() {
                        parse_option(
                            key,
                            if have_value {
                                Some(&cmdline[start..i])
                            } else {
                                None
                            },
                            options,
                        )?;
                    }
                    key = &cmdline[0..0];
                    have_value = false;
                    skip = true;
                }
            }
            _ => {}
        }
        if skip {
            start = i + 1;
        }
    }
    if options.root.as_deref() == Some("/dev/nfs") || options.rootfstype.as_deref() == Some("nfs") {
        parse_nfsroot(options)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regular() {
        let cmdline = String::from("root=/dev/mmcblk0p1 rw\n");
        let mut options = CmdlineOptions {
            ..Default::default()
        };

        parse_cmdline(cmdline, &mut options).expect("failed");
        assert_eq!(options.root.as_deref(), Some("/dev/mmcblk0p1"));
        assert!(options.rootfstype.is_none());
        assert!(options.rootflags.is_none());
        assert_eq!(options.rootfsflags, MsFlags::empty());
        assert!(options.nfsroot.is_none());
        assert_eq!(options.init, SBIN_INIT);
    }

    #[test]
    fn test_nfs() {
        let cmdline = String::from("root=/dev/nfs nfsroot=192.168.42.23:/path/to/nfsroot,v3,tcp ip=dhcp console=ttymxc1,115200n8 rootwait ro\n");
        let mut options = CmdlineOptions {
            ..Default::default()
        };

        parse_cmdline(cmdline, &mut options).expect("failed");
        assert_eq!(
            options.root.as_deref(),
            Some("192.168.42.23:/path/to/nfsroot")
        );
        assert_eq!(options.rootfstype.as_deref(), Some("nfs"));
        assert_eq!(
            options.rootflags.as_deref(),
            Some("nolock,v3,tcp,addr=192.168.42.23")
        );
        assert_eq!(options.rootfsflags, MsFlags::MS_RDONLY);
        assert_eq!(
            options.nfsroot.as_deref(),
            Some("192.168.42.23:/path/to/nfsroot,v3,tcp")
        );
        assert_eq!(options.init, SBIN_INIT);
    }

    #[test]
    fn test_9p_qemu() {
        let cmdline = String::from(
            "root=/dev/root rootfstype=9p rootflags=trans=virtio console=ttyAMA0,115200\n",
        );
        let mut options = CmdlineOptions {
            ..Default::default()
        };

        parse_cmdline(cmdline, &mut options).expect("failed");
        assert_eq!(options.root.as_deref(), Some("/dev/root"));
        assert_eq!(options.rootfstype.as_deref(), Some("9p"));
        assert_eq!(options.rootflags.as_deref(), Some("trans=virtio"));
        assert_eq!(options.rootfsflags, MsFlags::MS_RDONLY);
        assert!(options.nfsroot.is_none());
        assert_eq!(options.init, SBIN_INIT);
    }

    #[test]
    fn test_9p_usbg() {
        let cmdline = String::from("root=rootdev rootfstype=9p rootflags=trans=usbg,cache=loose,uname=root,dfltuid=0,dfltgid=0,aname=/path/to/9pfsroot rw\n");
        let mut options = CmdlineOptions {
            ..Default::default()
        };

        parse_cmdline(cmdline, &mut options).expect("failed");
        assert_eq!(options.root.as_deref(), Some("rootdev"));
        assert_eq!(options.rootfstype.as_deref(), Some("9p"));
        assert_eq!(
            options.rootflags.as_deref(),
            Some("trans=usbg,cache=loose,uname=root,dfltuid=0,dfltgid=0,aname=/path/to/9pfsroot")
        );
        assert_eq!(options.rootfsflags, MsFlags::empty());
        assert!(options.nfsroot.is_none());
        assert_eq!(options.init, SBIN_INIT);
    }

    #[test]
    fn test_init() {
        let cmdline = String::from("root=/dev/mmcblk0p1 init=/bin/sh\n");
        let mut options = CmdlineOptions {
            ..Default::default()
        };

        parse_cmdline(cmdline, &mut options).expect("failed");
        assert_eq!(options.root.as_deref(), Some("/dev/mmcblk0p1"));
        assert!(options.rootfstype.is_none());
        assert!(options.rootflags.is_none());
        assert_eq!(options.rootfsflags, MsFlags::MS_RDONLY);
        assert!(options.nfsroot.is_none());
        assert_eq!(options.init, "/bin/sh");
    }
}
