// SPDX-FileCopyrightText: 2024 The rsinit Authors
// SPDX-License-Identifier: GPL-2.0-only

use std::fmt::Debug;

use nix::mount::MsFlags;

use crate::util::{read_file, Result};

pub fn ensure_value<'a>(key: &str, value: Option<&'a str>) -> Result<&'a str> {
    value.ok_or(format!("Cmdline option '{key}' must have an argument!").into())
}

#[derive(Debug, PartialEq)]
pub struct CmdlineOptions {
    pub root: Option<String>,
    pub rootfstype: Option<String>,
    pub rootflags: Option<String>,
    pub rootfsflags: MsFlags,
    pub verity_root: Option<String>,
    pub nfsroot: Option<String>,
    pub init: String,
    pub cleanup: bool,
    /// Attempt to bind-mount `/lib/modules` from the initrd at `/root/lib/modules`.
    ///
    /// Enabled by the `rsinit.bind_modules` cmdline flag.
    pub bind_modules: bool,
}

impl Default for CmdlineOptions {
    fn default() -> CmdlineOptions {
        CmdlineOptions {
            root: None,
            rootfstype: None,
            rootflags: None,
            rootfsflags: MsFlags::MS_RDONLY,
            verity_root: None,
            nfsroot: None,
            init: "/sbin/init".into(),
            cleanup: true,
            bind_modules: false,
        }
    }
}

impl CmdlineOptions {
    fn parse_option<'a>(
        &mut self,
        key: &str,
        value: Option<&str>,
        callbacks: &mut [Box<CmdlineCallback<'a>>],
    ) -> Result<()> {
        match key {
            "root" => self.root = Some(ensure_value(key, value)?.to_string()),
            "rootfstype" => self.rootfstype = Some(ensure_value(key, value)?.to_string()),
            "rootflags" => self.rootflags = value.map(str::to_string),
            "ro" => self.rootfsflags.insert(MsFlags::MS_RDONLY),
            "rw" => self.rootfsflags.remove(MsFlags::MS_RDONLY),
            "rsinit.verity_root" => self.verity_root = Some(ensure_value(key, value)?.to_string()),
            "nfsroot" => self.nfsroot = Some(ensure_value(key, value)?.to_string()),
            "init" => self.init = ensure_value(key, value)?.into(),
            "rsinit.bind_modules" => self.bind_modules = true,
            _ => {
                for cb in callbacks {
                    cb(key, value)?
                }
            }
        }
        Ok(())
    }

    fn parse_nfsroot(&mut self) -> Result<()> {
        if self.root.as_deref() != Some("/dev/nfs") && self.rootfstype.as_deref() != Some("nfs") {
            return Ok(());
        }

        let nfsroot_option = self
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
        self.root = Some(nfsroot.to_string());
        self.rootflags = Some(rootflags);
        self.rootfstype = Some("nfs".to_string());
        Ok(())
    }
}

pub type CmdlineCallback<'a> = dyn FnMut(&str, Option<&str>) -> Result<()> + 'a;

#[derive(Default)]
pub struct CmdlineOptionsParser<'a> {
    callbacks: Vec<Box<CmdlineCallback<'a>>>,
}

impl<'a> CmdlineOptionsParser<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_callback(&mut self, cb: Box<CmdlineCallback<'a>>) {
        self.callbacks.push(cb);
    }

    pub fn parse_file(&mut self, path: &str) -> Result<CmdlineOptions> {
        let cmdline = read_file(path)?;
        self.parse_string(&cmdline)
    }

    pub fn parse_string(&mut self, cmdline: &str) -> Result<CmdlineOptions> {
        let mut options = CmdlineOptions::default();
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
                            options.parse_option(
                                key,
                                if have_value {
                                    Some(&cmdline[start..i])
                                } else {
                                    None
                                },
                                &mut self.callbacks,
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

        options.parse_nfsroot()?;

        Ok(options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regular() {
        let cmdline = "root=/dev/mmcblk0p1 rw\n";

        let expected = CmdlineOptions {
            root: Some("/dev/mmcblk0p1".into()),
            rootfsflags: MsFlags::empty(),
            ..Default::default()
        };

        let options = CmdlineOptionsParser::new()
            .parse_string(cmdline)
            .expect("failed");

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

        let options = CmdlineOptionsParser::new()
            .parse_string(cmdline)
            .expect("failed");

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

        let options = CmdlineOptionsParser::new()
            .parse_string(cmdline)
            .expect("failed");

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

        let options = CmdlineOptionsParser::new()
            .parse_string(cmdline)
            .expect("failed");

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

        let options = CmdlineOptionsParser::new()
            .parse_string(cmdline)
            .expect("failed");

        assert_eq!(options, expected);
    }

    #[test]
    fn test_custom_option() {
        let cmdline = "root=/dev/mmcblk0p1 rsinit.custom=xyz\n";
        let custom_option = std::cell::RefCell::new(String::new());

        let cb = Box::new(|key: &str, value: Option<&str>| {
            if key == "rsinit.custom" {
                *custom_option.borrow_mut() = ensure_value(key, value)?.to_owned();
            }
            Ok(())
        });

        let mut parser = CmdlineOptionsParser::new();
        parser.add_callback(cb);

        let _ = parser.parse_string(cmdline).expect("failed");

        assert_eq!(&*custom_option.borrow(), "xyz");
    }

    #[test]
    fn test_verity() {
        let cmdline = "rsinit.verity_root=/dev/mmcblk0p1 rootfstype=ext4\n";

        let expected = CmdlineOptions {
            verity_root: Some("/dev/mmcblk0p1".into()),
            rootfstype: Some("ext4".into()),
            ..Default::default()
        };

        let options = CmdlineOptionsParser::new()
            .parse_string(cmdline)
            .expect("failed");

        assert_eq!(options, expected);
    }

    #[test]
    fn test_rsinit_bind() {
        let cmdline = "root=/dev/root rsinit.bind_modules\n";

        let expected = CmdlineOptions {
            root: Some("/dev/root".into()),
            bind_modules: true,
            ..Default::default()
        };

        let options = CmdlineOptionsParser::new()
            .parse_string(cmdline)
            .expect("failed");

        assert_eq!(options, expected);
    }
}
