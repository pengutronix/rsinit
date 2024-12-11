// SPDX-License-Identifier: GPL-2.0-only
use nix::mount::MsFlags;
use std::fs::read_to_string;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub struct CmdlineOptions {
    pub root: Option<String>,
    pub rootfstype: Option<String>,
    pub rootflags: Option<String>,
    pub rootfsflags: MsFlags,
    pub nfsroot: Option<String>,
}

impl Default for CmdlineOptions {
    fn default() -> CmdlineOptions {
        CmdlineOptions {
            root: None,
            rootfstype: None,
            rootflags: None,
            rootfsflags: MsFlags::MS_RDONLY,
            nfsroot: None,
        }
    }
}

fn parse_option(key: String, value: Option<String>, options: &mut CmdlineOptions) {
    match key.as_str() {
        "root" => options.root = value,
        "rootfstype" => options.rootfstype = value,
        "rootflags" => options.rootflags = value,
        "ro" => options.rootfsflags.insert(MsFlags::MS_RDONLY),
        "rw" => options.rootfsflags.remove(MsFlags::MS_RDONLY),
        "nfsroot" => options.nfsroot = value,
        _ => (),
    }
}

fn parse_nfsroot(options: &mut CmdlineOptions) -> Result<()> {
    if options.nfsroot.is_none() {
        return Err("Missing nfsroot command-line option!".into());
    }
    let nfsroot_option = options.nfsroot.as_ref().unwrap();
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
        let pnp = read_to_string("/proc/net/pnp")
            .map_err(|e| format!("Failed to read /proc/net/pnp: {e}"))?;
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
        let (bootserver, _) = nfsroot.split_once(':').unwrap();
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
    let mut key = String::new();
    let mut value = String::new();

    for c in cmdline.chars() {
        let mut skip = false;
        match c {
            '=' => {
                if !have_value {
                    skip = true;
                }
                have_value = true;
            }
            '"' => {
                quoted = !quoted;
                skip = true;
            }
            ' ' => {
                if !quoted {
                    if !key.is_empty() {
                        parse_option(key, if have_value { Some(value) } else { None }, options);
                    }
                    key = String::new();
                    value = String::new();
                    have_value = false;
                    skip = true;
                }
            }
            _ => {}
        }
        if !skip {
            if have_value {
                value.push(c);
            } else {
                key.push(c)
            }
        }
    }
    if !key.is_empty() {
        parse_option(key, if have_value { Some(value) } else { None }, options);
    }
    if !options.root.is_none() && options.root.as_ref().unwrap() == "/dev/nfs"
        || !options.rootfstype.is_none() && options.rootfstype.as_ref().unwrap() == "nfs"
    {
        parse_nfsroot(options)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regular() {
        let cmdline = String::from("root=/dev/mmcblk0p1 rw");
        let mut options = CmdlineOptions {
            ..Default::default()
        };

        parse_cmdline(cmdline, &mut options).expect("failed");
        assert!(options.root.is_some());
        assert_eq!(options.root.unwrap(), "/dev/mmcblk0p1");
        assert!(options.rootfstype.is_none());
        assert!(options.rootflags.is_none());
        assert_eq!(options.rootfsflags, MsFlags::empty());
        assert!(options.nfsroot.is_none());
    }

    #[test]
    fn test_nfs() {
        let cmdline = String::from("root=/dev/nfs nfsroot=192.168.42.23:/path/to/nfsroot,v3,tcp ip=dhcp console=ttymxc1,115200n8 rootwait ro");
        let mut options = CmdlineOptions {
            ..Default::default()
        };

        parse_cmdline(cmdline, &mut options).expect("failed");
        assert!(options.root.is_some());
        assert_eq!(options.root.unwrap(), "192.168.42.23:/path/to/nfsroot");
        assert!(options.rootfstype.is_some());
        assert_eq!(options.rootfstype.unwrap(), "nfs");
        assert!(options.rootflags.is_some());
        assert_eq!(
            options.rootflags.unwrap(),
            "nolock,v3,tcp,addr=192.168.42.23"
        );
        assert_eq!(options.rootfsflags, MsFlags::MS_RDONLY);
        assert!(options.nfsroot.is_some());
        assert_eq!(
            options.nfsroot.unwrap(),
            "192.168.42.23:/path/to/nfsroot,v3,tcp"
        );
    }

    #[test]
    fn test_9p_qemu() {
        let cmdline = String::from(
            "root=/dev/root rootfstype=9p rootflags=trans=virtio console=ttyAMA0,115200",
        );
        let mut options = CmdlineOptions {
            ..Default::default()
        };

        parse_cmdline(cmdline, &mut options).expect("failed");
        assert!(options.root.is_some());
        assert_eq!(options.root.unwrap(), "/dev/root");
        assert!(options.rootfstype.is_some());
        assert_eq!(options.rootfstype.unwrap(), "9p");
        assert!(options.rootflags.is_some());
        assert_eq!(options.rootflags.unwrap(), "trans=virtio");
        assert_eq!(options.rootfsflags, MsFlags::MS_RDONLY);
        assert!(options.nfsroot.is_none());
    }

    #[test]
    fn test_9p_usbg() {
        let cmdline = String::from("root=rootdev rootfstype=9p rootflags=trans=usbg,cache=loose,uname=root,dfltuid=0,dfltgid=0,aname=/path/to/9pfsroot rw");
        let mut options = CmdlineOptions {
            ..Default::default()
        };

        parse_cmdline(cmdline, &mut options).expect("failed");
        assert!(options.root.is_some());
        assert_eq!(options.root.unwrap(), "rootdev");
        assert!(options.rootfstype.is_some());
        assert_eq!(options.rootfstype.unwrap(), "9p");
        assert!(options.rootflags.is_some());
        assert_eq!(
            options.rootflags.unwrap(),
            "trans=usbg,cache=loose,uname=root,dfltuid=0,dfltgid=0,aname=/path/to/9pfsroot"
        );
        assert_eq!(options.rootfsflags, MsFlags::empty());
        assert!(options.nfsroot.is_none());
    }
}
