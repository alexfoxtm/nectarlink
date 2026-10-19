// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS connection doctor integrations.
//!
//! Only what can actually be read is reported: network adapters come from
//! `ifconfig`. The application firewall's per-app state and network trust
//! profiles aren't read yet, so those checks return errors and the shared
//! doctor leaves them out rather than showing a made-up pass.

use std::{net::Ipv4Addr, path::Path, process::Command};

use crate::i18n::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Firewall {
    pub on: bool,
    pub allowed: bool,
    pub blocked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub name: String,
    pub trusted: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Adapters {
    pub local_address: bool,
    pub vpn: bool,
}

/// The application firewall's view of `exe` isn't read on macOS yet.
pub fn firewall(_exe: &Path) -> Result<Firewall, String> {
    Err(tr!("Doctor", "The macOS firewall state isn't checked yet."))
}

/// macOS has no public/private network profile notion to report.
pub fn networks() -> Result<Vec<Network>, String> {
    Err(tr!("Doctor", "Network profiles aren't checked on macOS."))
}

/// Whether this Mac has a usable IPv4 address, and whether a tunnel
/// interface (VPN) carries one too, from `ifconfig`.
pub fn adapters() -> Adapters {
    match Command::new("/sbin/ifconfig").output() {
        Ok(out) if out.status.success() => parse_ifconfig(&String::from_utf8_lossy(&out.stdout)),
        Ok(out) => {
            tracing::warn!(status = %out.status, "ifconfig failed");
            Adapters::default()
        }
        Err(e) => {
            tracing::warn!(error = %e, "can't run ifconfig");
            Adapters::default()
        }
    }
}

/// Reads `ifconfig` output: an `inet` line under a non-tunnel interface
/// that isn't loopback or link-local means the Mac is on a network; one
/// under a `utun`/`ppp`/`ipsec`/`tun`/`tap` interface means a VPN is up.
fn parse_ifconfig(text: &str) -> Adapters {
    let mut adapters = Adapters::default();
    let mut iface = String::new();
    for line in text.lines() {
        if !line.starts_with([' ', '\t']) {
            iface = line.split(':').next().unwrap_or_default().trim().to_owned();
            continue;
        }
        let mut words = line.split_whitespace();
        if words.next() != Some("inet") {
            continue;
        }
        let Some(addr) = words.next().and_then(|a| a.parse::<Ipv4Addr>().ok()) else {
            continue;
        };
        if addr.is_loopback() || addr.is_link_local() || addr.is_unspecified() {
            continue;
        }
        if is_tunnel(&iface) {
            adapters.vpn = true;
        } else {
            adapters.local_address = true;
        }
    }
    adapters
}

fn is_tunnel(iface: &str) -> bool {
    ["utun", "ppp", "ipsec", "tun", "tap"].iter().any(|p| iface.starts_with(p))
}

/// Changing the firewall needs an admin prompt that isn't implemented; the
/// shared doctor falls back to opening the firewall settings.
pub fn fix_firewall(_exe: &Path) -> Result<bool, String> {
    Err(tr!("Doctor", "Changing the macOS firewall isn't automated yet."))
}

pub fn open_firewall_settings() -> bool {
    match Command::new("open").arg("x-apple.systempreferences:com.apple.preference.security?Firewall").spawn()
    {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(error = %e, "can't open firewall settings");
            false
        }
    }
}

pub fn diagnose() -> Vec<(String, bool, String)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ifconfig_lan_address_counts_and_tunnel_counts_as_vpn() {
        let text = "lo0: flags=8049<UP,LOOPBACK,RUNNING,MULTICAST> mtu 16384\n\
                    \tinet 127.0.0.1 netmask 0xff000000\n\
                    en0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500\n\
                    \tinet6 fe80::1%en0 prefixlen 64\n\
                    \tinet 192.168.1.23 netmask 0xffffff00 broadcast 192.168.1.255\n\
                    \tstatus: active\n\
                    utun4: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1280\n\
                    \tinet 100.101.102.103 --> 100.101.102.103 netmask 0xffffffff\n";
        let a = parse_ifconfig(text);
        assert!(a.local_address);
        assert!(a.vpn);

        let loopback_only = "lo0: flags=8049<UP,LOOPBACK,RUNNING,MULTICAST> mtu 16384\n\
                             \tinet 127.0.0.1 netmask 0xff000000\n\
                             en0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500\n\
                             \tinet 169.254.10.4 netmask 0xffff0000 broadcast 169.254.255.255\n";
        let a = parse_ifconfig(loopback_only);
        assert!(!a.local_address);
        assert!(!a.vpn);
    }
}
