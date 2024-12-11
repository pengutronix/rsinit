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
