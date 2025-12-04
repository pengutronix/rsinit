// SPDX-FileCopyrightText: 2024 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::fmt::Debug;

use nix::mount::MsFlags;

use crate::{
    init::CmdlineCallback,
    util::{read_file, Result},
};

#[derive(Debug, PartialEq)]
pub struct CmdlineOptions<'a> {
    pub root: Option<String>,
    pub rootfstype: Option<String>,
    pub rootflags: Option<String>,
    pub rootfsflags: MsFlags,
    pub nfsroot: Option<String>,
    pub init: String,
    pub cleanup: bool,
    callbacks: CmdlineOptionsCallbacks<'a>,
}

#[derive(Default)]
struct CmdlineOptionsCallbacks<'a>(Vec<Box<CmdlineCallback<'a>>>);

impl PartialEq for CmdlineOptionsCallbacks<'_> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Debug for CmdlineOptionsCallbacks<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CmdlineOptionsCallbacks")
            .field("callbacks_count", &self.0.len())
            .finish()
    }
}

const SBIN_INIT: &str = "/sbin/init";

impl<'a> Default for CmdlineOptions<'a> {
    fn default() -> CmdlineOptions<'a> {
        CmdlineOptions {
            root: None,
            rootfstype: None,
            rootflags: None,
            rootfsflags: MsFlags::MS_RDONLY,
            nfsroot: None,
            init: SBIN_INIT.into(),
            cleanup: true,
            callbacks: CmdlineOptionsCallbacks::default(),
        }
    }
}

pub fn ensure_value<'a>(key: &str, value: Option<&'a str>) -> Result<&'a str> {
    value.ok_or(format!("Cmdline option '{key}' must have an argument!").into())
}

fn parse_option<'a>(
    key: &str,
    value: Option<&str>,
    options: &mut CmdlineOptions,
    callbacks: &mut [Box<CmdlineCallback<'a>>],
) -> Result<()> {
    match key {
        "root" => options.root = Some(ensure_value(key, value)?.to_string()),
        "rootfstype" => options.rootfstype = Some(ensure_value(key, value)?.to_string()),
        "rootflags" => options.rootflags = value.map(str::to_string),
        "ro" => options.rootfsflags.insert(MsFlags::MS_RDONLY),
        "rw" => options.rootfsflags.remove(MsFlags::MS_RDONLY),
        "nfsroot" => options.nfsroot = Some(ensure_value(key, value)?.to_string()),
        "init" => options.init = ensure_value(key, value)?.into(),
        _ => {
            for cb in callbacks {
                cb(key, value)?
            }
        }
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

impl<'a> CmdlineOptions<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_with_callbacks(callbacks: Vec<Box<CmdlineCallback<'a>>>) -> Self {
        Self {
            callbacks: CmdlineOptionsCallbacks(callbacks),
            ..Default::default()
        }
    }

    pub fn from_file(&mut self, path: &str) -> Result<Self> {
        let cmdline = read_file(path)?;
        self.from_string(&cmdline)
    }

    pub fn from_string(&mut self, cmdline: &str) -> Result<Self> {
        let mut options = Self::default();
        let mut have_value = false;
        let mut quoted = false;
        let mut key = &cmdline[0..0];
        let mut start = 0;

        for (i, c) in cmdline.char_indices() {
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
                                &mut options,
                                &mut self.callbacks.0,
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

        if options.root.as_deref() == Some("/dev/nfs")
            || options.rootfstype.as_deref() == Some("nfs")
        {
            parse_nfsroot(&mut options)?;
        }

        Ok(options)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[test]
    fn test_regular() {
        let cmdline = "root=/dev/mmcblk0p1 rw\n";

        let expected = CmdlineOptions {
            root: Some("/dev/mmcblk0p1".into()),
            rootfsflags: MsFlags::empty(),
            ..Default::default()
        };

        let options = CmdlineOptions::new().from_string(cmdline).expect("failed");

        assert_eq!(options, expected);
    }

    #[test]
    fn test_nfs() {
        let cmdline = "root=/dev/nfs nfsroot=192.168.42.23:/path/to/nfsroot,v3,tcp ip=dhcp console=ttymxc1,115200n8 rootwait ro\n";

        let expected = CmdlineOptions {
            root: Some("192.168.42.23:/path/to/nfsroot".into()),
            rootflags: Some("nolock,v3,tcp,addr=192.168.42.23".into()),
            rootfsflags: MsFlags::MS_RDONLY,
            nfsroot: Some("192.168.42.23:/path/to/nfsroot,v3,tcp".into()),
            rootfstype: Some("nfs".into()),
            ..Default::default()
        };

        let options = CmdlineOptions::new().from_string(cmdline).expect("failed");

        assert_eq!(options, expected);
    }

    #[test]
    fn test_9p_qemu() {
        let cmdline =
            "root=/dev/root rootfstype=9p rootflags=trans=virtio console=ttyAMA0,115200\n";

        let expected = CmdlineOptions {
            root: Some("/dev/root".into()),
            rootfstype: Some("9p".into()),
            rootflags: Some("trans=virtio".into()),
            ..Default::default()
        };

        let options = CmdlineOptions::new().from_string(cmdline).expect("failed");

        assert_eq!(options, expected);
    }

    #[test]
    fn test_9p_usbg() {
        let cmdline = "root=rootdev rootfstype=9p rootflags=trans=usbg,cache=loose,uname=root,dfltuid=0,dfltgid=0,aname=/path/to/9pfsroot rw\n";

        let expected = CmdlineOptions {
            root: Some("rootdev".into()),
            rootfstype: Some("9p".into()),
            rootflags: Some(
                "trans=usbg,cache=loose,uname=root,dfltuid=0,dfltgid=0,aname=/path/to/9pfsroot"
                    .into(),
            ),
            rootfsflags: MsFlags::empty(),
            ..Default::default()
        };

        let options = CmdlineOptions::new().from_string(cmdline).expect("failed");

        assert_eq!(options, expected);
    }

    #[test]
    fn test_init() {
        let cmdline = "root=/dev/mmcblk0p1 init=/bin/sh\n";

        let expected = CmdlineOptions {
            root: Some("/dev/mmcblk0p1".into()),
            init: "/bin/sh".into(),
            ..Default::default()
        };

        let options = CmdlineOptions::new().from_string(cmdline).expect("failed");

        assert_eq!(options, expected);
    }

    #[test]
    fn test_callbacks() {
        let cmdline = "root=/dev/mmcblk0p1 rsinit.custom=xyz\n";
        let custom_value = RefCell::new(String::new());

        let expected = CmdlineOptions {
            root: Some("/dev/mmcblk0p1".into()),
            ..Default::default()
        };

        let cb = Box::new(|key: &str, value: Option<&str>| {
            match key {
                "rsinit.custom" => {
                    *custom_value.borrow_mut() = ensure_value(key, value)?.to_owned();
                }
                _ => {}
            }
            Result::Ok(())
        });

        let options = CmdlineOptions::new_with_callbacks(vec![cb])
            .from_string(cmdline)
            .expect("failed");

        assert_eq!(options, expected);
        assert_eq!(&*custom_value.borrow(), "xyz");
    }
}
