// SPDX-License-Identifier: MPL-2.0
//! Pure-Rust client for the local ADB server (`127.0.0.1:5037`) used to detect
//! USB-attached Android devices and set up TCP port forwarding for NectarLink's
//! USB QUIC tunnel.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;

/// Default local ADB server address (`127.0.0.1:5037`).
pub const DEFAULT_ADB_ADDR: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 5037));

/// Returns `true` if `serial` is non-empty and permitted by the optional
/// `NECTARLINK_ADB_ONLY_SERIAL` filter.
pub fn is_serial_allowed(serial: &str) -> bool {
    let trimmed = serial.trim();
    if trimmed.is_empty() {
        return false;
    }
    if let Ok(only) = std::env::var("NECTARLINK_ADB_ONLY_SERIAL") {
        let only = only.trim();
        if !only.is_empty() && trimmed != only {
            return false;
        }
    }
    true
}

/// Errors returned by ADB server operations.
#[derive(Debug, thiserror::Error)]
pub enum AdbError {
    #[error("ADB I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ADB server rejected command: {0}")]
    Remote(String),
    #[error("ADB protocol error: {0}")]
    Protocol(String),
    #[error("ADB serial {0} is blocked by safety guard")]
    BlockedSerial(String),
}

/// An Android device reported by the ADB server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbDevice {
    pub serial: String,
    pub state: String,
    pub model: Option<String>,
    pub product: Option<String>,
    pub transport_id: Option<String>,
    /// The `usb:` descriptor from `host:devices-l` (e.g. `usb:2-1`), present
    /// only for physical-USB attachments. Absent for TCP/mDNS devices and in
    /// short `host:devices` records.
    pub usb: Option<String>,
}

/// What carries an ADB tunnel: a proven physical cable, a network attachment,
/// or an origin the server records cannot prove.
///
/// Only a positive `usb:` descriptor proves a physical cable. A bare serial
/// without one (old servers, short records) is [`TunnelMedium::Unknown`],
/// never presumed wired; TCP (`ip:port`) and mDNS
/// (`adb-…._adb-tls-connect._tcp`) serials are [`TunnelMedium::Network`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TunnelMedium {
    /// `host:devices-l` reported a `usb:` descriptor for this serial.
    PhysicalUsb,
    /// The serial itself is a network address (`ip:port` or mDNS name).
    Network,
    /// No origin proof (phone-accepted tunnels, short records, bare serials).
    #[default]
    Unknown,
}

impl AdbDevice {
    /// Whether the device is online and authorized (`state == "device"`).
    pub fn is_online(&self) -> bool {
        self.state == "device"
    }

    /// Classifies the tunnel origin from the server's own records. A positive
    /// `usb:` descriptor proves a physical cable; network-shaped serials prove
    /// a network attachment; anything else stays unknown (never presumed from
    /// a bare serial or from signal bars, which this crate cannot see).
    pub fn medium(&self) -> TunnelMedium {
        if self.usb.as_deref().is_some_and(|u| !u.trim().is_empty()) {
            return TunnelMedium::PhysicalUsb;
        }
        let serial = self.serial.trim();
        if serial.contains(':') || serial.contains("._adb-tls-connect") {
            return TunnelMedium::Network;
        }
        TunnelMedium::Unknown
    }
}

/// A single mDNS service instance discovered by the ADB server
/// (`host:mdns:services`), i.e. one row of `adb mdns services`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdnsService {
    /// Service instance name, e.g. `adb-R58M123ABC-xyz`.
    pub instance: String,
    /// Service type, e.g. `_adb-tls-connect._tcp.` (trailing dot as sent).
    pub service: String,
    /// Resolved interface address of the instance.
    pub addr: std::net::IpAddr,
    /// Resolved port of the instance.
    pub port: u16,
}

/// Parses the `host:mdns:services` payload: TAB-separated
/// `<instance>\t<service>\t<ip>:<port>` rows (AOSP adblib format; the service
/// name carries a trailing dot). Garbage rows are skipped; an IPv4-mapped or
/// plain address parses the same. Unknown/old servers are handled by the
/// caller via [`mdns_services`] failing.
pub fn parse_mdns_services(input: &str) -> Vec<MdnsService> {
    let mut out = Vec::new();
    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("List of discovered") {
            continue;
        }
        let mut cols = line.split('\t');
        let (Some(instance), Some(service), Some(addr_port)) = (cols.next(), cols.next(), cols.next()) else {
            continue;
        };
        let instance = instance.trim();
        let service = service.trim();
        if instance.is_empty() || service.is_empty() {
            continue;
        }
        let Some((ip_str, port_str)) = addr_port.trim().rsplit_once(':') else { continue };
        let (Ok(addr), Ok(port)) = (ip_str.parse(), port_str.parse()) else { continue };
        out.push(MdnsService { instance: instance.to_owned(), service: service.to_owned(), addr, port });
    }
    out
}

/// Whether `serial` is an mDNS-shaped ADB serial
/// (`adb-…._adb-tls-connect._tcp`), whose outer IP must be resolved via
/// [`mdns_services`] rather than read from the serial itself.
pub fn is_mdns_serial(serial: &str) -> bool {
    let s = serial.trim();
    s.contains("._adb-tls-connect._tcp")
}

/// The outer phone IP for a TCP-shaped serial (`192.168.1.40:5555`),
/// parsed locally. Returns `None` for bare serials, emulators and mDNS
/// serials (the latter resolve via [`mdns_services`]).
pub fn tcp_serial_ip(serial: &str) -> Option<std::net::IpAddr> {
    let s = serial.trim();
    if is_mdns_serial(s) {
        return None;
    }
    let (ip_str, _port) = s.rsplit_once(':')?;
    let ip: std::net::IpAddr = ip_str.parse().ok()?;
    (!ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast()).then_some(ip)
}

/// Matches an mDNS serial to its resolved outer IP from [`parse_mdns_services`]
/// output: the serial is `<instance>.<service>` (service trailing dot dropped).
/// Returns `None` when the instance is absent (stale, pairing-only, or unknown).
pub fn mdns_serial_ip(serial: &str, services: &[MdnsService]) -> Option<std::net::IpAddr> {
    let s = serial.trim();
    if !is_mdns_serial(s) {
        return None;
    }
    services.iter().find_map(|svc| {
        let key = format!("{}.{}", svc.instance, svc.service.trim_end_matches('.'));
        (key == s).then_some(svc.addr)
    })
}

/// Parses the output of `host:devices-l` or `host:track-devices`, filtering out
/// any serial rejected by [`is_serial_allowed`].
pub fn parse_devices_l(input: &str) -> Vec<AdbDevice> {
    let mut devices = Vec::new();
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("List of devices") {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(serial) = parts.next() else { continue };
        let Some(state) = parts.next() else { continue };
        if !is_serial_allowed(serial) {
            continue;
        }
        let mut model = None;
        let mut product = None;
        let mut transport_id = None;
        let mut usb = None;
        for token in parts {
            if let Some(v) = token.strip_prefix("model:") {
                model = Some(v.to_owned());
            } else if let Some(v) = token.strip_prefix("product:") {
                product = Some(v.to_owned());
            } else if let Some(v) = token.strip_prefix("transport_id:") {
                transport_id = Some(v.to_owned());
            } else if let Some(v) = token.strip_prefix("usb:") {
                usb = Some(v.to_owned());
            }
        }
        devices.push(AdbDevice {
            serial: serial.to_owned(),
            state: state.to_owned(),
            model,
            product,
            transport_id,
            usb,
        });
    }
    devices
}

/// Sends a length-prefixed command to the ADB server and reads the 4-byte status.
pub async fn send_command(stream: &mut TcpStream, cmd: &str) -> Result<(), AdbError> {
    let frame = format!("{:04x}{cmd}", cmd.len());
    stream.write_all(frame.as_bytes()).await?;
    stream.flush().await?;
    let mut status = [0u8; 4];
    stream.read_exact(&mut status).await?;
    match &status {
        b"OKAY" => Ok(()),
        b"FAIL" => {
            let reason = read_payload(stream).await.unwrap_or_else(|_| "unknown failure".into());
            Err(AdbError::Remote(reason))
        }
        other => {
            Err(AdbError::Protocol(format!("unexpected ADB status: {}", String::from_utf8_lossy(other))))
        }
    }
}

/// Reads a 4-hex-digit length prefix followed by the UTF-8 payload.
pub async fn read_payload(stream: &mut TcpStream) -> Result<String, AdbError> {
    let mut len_hex = [0u8; 4];
    stream.read_exact(&mut len_hex).await?;
    let len_str =
        std::str::from_utf8(&len_hex).map_err(|_| AdbError::Protocol("invalid hex length".into()))?;
    let len = usize::from_str_radix(len_str, 16)
        .map_err(|_| AdbError::Protocol(format!("invalid hex length: {len_str}")))?;
    let mut buf = vec![0u8; len];
    if len > 0 {
        stream.read_exact(&mut buf).await?;
    }
    String::from_utf8(buf).map_err(|_| AdbError::Protocol("invalid UTF-8 payload".into()))
}

/// Lists attached ADB devices via `host:devices-l`.
pub async fn list_devices(server: SocketAddr) -> Result<Vec<AdbDevice>, AdbError> {
    let mut stream = TcpStream::connect(server).await?;
    send_command(&mut stream, "host:devices-l").await?;
    let payload = read_payload(&mut stream).await?;
    Ok(parse_devices_l(&payload))
}

/// Queries resolved mDNS service instances via `host:mdns:services`
/// (AOSP `adb mdns services` source). Old servers reject the command
/// (`unknown host service 'mdns:services'`); the caller then treats mDNS
/// serials as unresolvable (unknown), never as failures of the tunnel.
pub async fn mdns_services(server: SocketAddr) -> Result<Vec<MdnsService>, AdbError> {
    let mut stream = TcpStream::connect(server).await?;
    send_command(&mut stream, "host:mdns:services").await?;
    let payload = read_payload(&mut stream).await?;
    Ok(parse_mdns_services(&payload))
}

/// Sets up TCP port forwarding (`adb -s <serial> forward tcp:<local> tcp:<remote>`)
/// and returns the bound local TCP port on `127.0.0.1`.
pub async fn forward_tcp(
    server: SocketAddr,
    serial: &str,
    local_port: u16,
    remote_port: u16,
) -> Result<u16, AdbError> {
    if !is_serial_allowed(serial) {
        return Err(AdbError::BlockedSerial(serial.to_owned()));
    }
    let mut port = if local_port == 0 { free_local_port().await? } else { local_port };
    let mut attempts = 0;
    loop {
        let mut stream = TcpStream::connect(server).await?;
        let cmd = format!("host-serial:{serial}:forward:tcp:{port};tcp:{remote_port}");
        match send_command(&mut stream, &cmd).await {
            Ok(()) => return Ok(port),
            // A port that was free a moment ago can still be taken by the time
            // adb binds it (on WSL the Windows side can hold a port the Linux
            // side just closed). Only then is another port worth trying.
            Err(AdbError::Remote(reason))
                if local_port == 0 && attempts < BIND_RETRIES && is_bind_failure(&reason) =>
            {
                attempts += 1;
                port = free_local_port().await?;
            }
            Err(e) => return Err(e),
        }
    }
}

/// How many other ports [`forward_tcp`] tries when adb can't bind the one it picked.
const BIND_RETRIES: u32 = 4;

/// A local TCP port that is free right now.
async fn free_local_port() -> Result<u16, AdbError> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    Ok(listener.local_addr()?.port())
}

/// Whether an adb `FAIL` reason means the local port couldn't be bound,
/// for example "cannot bind listener: Address already in use".
fn is_bind_failure(reason: &str) -> bool {
    let reason = reason.to_ascii_lowercase();
    reason.contains("cannot bind") || reason.contains("already in use")
}

/// Removes a TCP port forward (`adb -s <serial> forward --remove tcp:<local>`).
pub async fn kill_forward_tcp(server: SocketAddr, serial: &str, local_port: u16) -> Result<(), AdbError> {
    if !is_serial_allowed(serial) {
        return Err(AdbError::BlockedSerial(serial.to_owned()));
    }
    let mut stream = TcpStream::connect(server).await?;
    let cmd = format!("host-serial:{serial}:killforward:tcp:{local_port}");
    send_command(&mut stream, &cmd).await?;
    Ok(())
}

/// Streams device list changes until `cancel` is triggered or the ADB server
/// closes the connection. Prefers `host:track-devices-l`: only the long
/// format carries the `usb:` descriptor that proves a physical cable. Short
/// `host:track-devices` records have no medium tokens, so old servers that
/// reject `-l` fall back to short records (bare serials then classify as
/// unknown, never as wired).
pub async fn track_devices<F>(
    server: SocketAddr,
    cancel: CancellationToken,
    mut on_change: F,
) -> Result<(), AdbError>
where
    F: FnMut(Vec<AdbDevice>),
{
    let mut stream = tokio::select! {
        _ = cancel.cancelled() => return Ok(()),
        res = TcpStream::connect(server) => res?,
    };
    if send_command(&mut stream, "host:track-devices-l").await.is_err() {
        drop(stream);
        stream = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            res = TcpStream::connect(server) => res?,
        };
        send_command(&mut stream, "host:track-devices").await?;
    }
    loop {
        let payload = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            res = read_payload(&mut stream) => res?,
        };
        on_change(parse_devices_l(&payload));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn empty_serial_is_rejected_and_valid_serials_are_allowed() {
        assert!(!is_serial_allowed(""));
        assert!(!is_serial_allowed("   "));
        assert!(is_serial_allowed("emulator-5554"));
        assert!(is_serial_allowed("SYNTH_SERIAL_001"));
    }

    #[test]
    fn parse_devices_l_parses_metadata() {
        let sample = "\
List of devices attached
SYNTH_SERIAL_001       device product:dm1q model:Test_Phone device:dm1q transport_id:1
emulator-5554          device product:sdk_gphone64_x86_64 model:Pixel_9 device:emu64x transport_id:2
192.168.1.40:5555      offline transport_id:3
";
        let devices = parse_devices_l(sample);
        assert_eq!(devices.len(), 3);
        assert_eq!(
            devices[0],
            AdbDevice {
                serial: "SYNTH_SERIAL_001".into(),
                state: "device".into(),
                model: Some("Test_Phone".into()),
                product: Some("dm1q".into()),
                transport_id: Some("1".into()),
                usb: None,
            }
        );
        assert!(devices[0].is_online());
        // A bare serial without a `usb:` descriptor proves nothing: it must
        // not be presumed a wired cable.
        assert_eq!(devices[0].medium(), TunnelMedium::Unknown);
        assert_eq!(
            devices[1],
            AdbDevice {
                serial: "emulator-5554".into(),
                state: "device".into(),
                model: Some("Pixel_9".into()),
                product: Some("sdk_gphone64_x86_64".into()),
                transport_id: Some("2".into()),
                usb: None,
            }
        );
        assert!(devices[1].is_online());
        assert_eq!(devices[1].medium(), TunnelMedium::Unknown);
        assert_eq!(
            devices[2],
            AdbDevice {
                serial: "192.168.1.40:5555".into(),
                state: "offline".into(),
                model: None,
                product: None,
                transport_id: Some("3".into()),
                usb: None,
            }
        );
        assert!(!devices[2].is_online());
        // A TCP serial is a network attachment even without extra tokens.
        assert_eq!(devices[2].medium(), TunnelMedium::Network);
    }

    #[test]
    fn parse_devices_l_proves_physical_usb_only_from_usb_descriptor() {
        let sample = "\
List of devices attached
R58M123ABC            device product:dm1q model:Test_Phone device:dm1q usb:2-1 transport_id:4
192.168.1.40:5555      device product:dm1q model:Test_Phone device:dm1q transport_id:5
adb-R58M123ABC-xyz._adb-tls-connect._tcp device product:dm1q model:Test_Phone transport_id:6
";
        let devices = parse_devices_l(sample);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].medium(), TunnelMedium::PhysicalUsb);
        assert_eq!(devices[1].medium(), TunnelMedium::Network);
        assert_eq!(devices[2].medium(), TunnelMedium::Network);
    }

    #[test]
    fn mdns_services_parse_tab_rows_and_skip_garbage() {
        let sample = "List of discovered mdns services\n\
adb-R58M123ABC-xyz\t_adb-tls-connect._tcp.\t192.168.1.40:39149\n\
adb-R58M123ABC-xyz\t_adb-tls-pairing._tcp.\t192.168.1.40:37313\n\
garbage without tabs\n\
adb-BROKEN\t_adb-tls-connect._tcp.\tnope:notaport\n";
        let services = parse_mdns_services(sample);
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].instance, "adb-R58M123ABC-xyz");
        assert_eq!(services[0].service, "_adb-tls-connect._tcp.");
        assert_eq!(services[0].addr.to_string(), "192.168.1.40");
        assert_eq!(services[0].port, 39149);
        // The mDNS serial resolves to the connect instance IP, not the pairing one.
        let serial = "adb-R58M123ABC-xyz._adb-tls-connect._tcp";
        assert!(is_mdns_serial(serial));
        assert_eq!(
            mdns_serial_ip(serial, &services).map(|ip| ip.to_string()),
            Some("192.168.1.40".to_owned())
        );
        // TCP serials resolve locally; bare/emulator serials resolve nowhere.
        assert_eq!(
            tcp_serial_ip("192.168.1.40:5555").map(|ip| ip.to_string()),
            Some("192.168.1.40".to_owned())
        );
        assert_eq!(tcp_serial_ip(serial), None);
        assert_eq!(tcp_serial_ip("emulator-5554"), None);
        assert_eq!(tcp_serial_ip("R58M123ABC"), None);
    }

    #[test]
    fn mdns_serial_without_instance_stays_unknown() {
        let services = parse_mdns_services("adb-OTHER-xyz\t_adb-tls-connect._tcp.\t192.168.1.41:39149\n");
        // Stale, pairing-only, or unknown instances never match: no invented IP.
        assert_eq!(mdns_serial_ip("adb-R58M123ABC-xyz._adb-tls-connect._tcp", &services), None);
        assert_eq!(mdns_serial_ip("adb-R58M123ABC-xyz._adb-tls-pairing._tcp", &services), None);
        assert_eq!(mdns_serial_ip("192.168.1.40:5555", &services), None);
        assert_eq!(mdns_serial_ip("R58M123ABC", &services), None);
        assert!(parse_mdns_services("nothing here\n").is_empty());
    }

    #[test]
    fn bare_serial_without_usb_descriptor_is_never_presumed_wired() {
        for serial in ["R58M123ABC", "emulator-5554", "  SYNTH_SERIAL_001  "] {
            let sample = format!("List of devices attached\n{serial} device transport_id:1\n");
            let devices = parse_devices_l(&sample);
            assert_eq!(devices.len(), 1, "serial {serial:?}");
            assert_eq!(devices[0].medium(), TunnelMedium::Unknown, "serial {serial:?}");
        }
        // An empty `usb:` token carries no proof either.
        let devices = parse_devices_l("List of devices attached\nR58M123ABC device usb: transport_id:1\n");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].usb, Some(String::new()));
        assert_eq!(devices[0].medium(), TunnelMedium::Unknown);
    }

    /// A mock adb server that answers each `forward` with the next of
    /// `replies` (`None` = OKAY, `Some(reason)` = FAIL) and records the commands.
    async fn forward_server(replies: Vec<Option<&'static str>>) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let commands = Arc::new(Mutex::new(Vec::new()));
        let seen = commands.clone();
        tokio::spawn(async move {
            let mut replies = replies.into_iter();
            while let Ok((mut socket, _)) = listener.accept().await {
                let Ok(cmd) = read_payload(&mut socket).await else { continue };
                seen.lock().unwrap().push(cmd);
                match replies.next().flatten() {
                    None => {
                        let _ = socket.write_all(b"OKAY").await;
                    }
                    Some(reason) => {
                        let frame = format!("FAIL{:04x}{reason}", reason.len());
                        let _ = socket.write_all(frame.as_bytes()).await;
                    }
                }
                let _ = socket.flush().await;
            }
        });
        (addr, commands)
    }

    #[tokio::test]
    async fn forward_tries_another_port_only_when_adb_cannot_bind() {
        let (addr, commands) =
            forward_server(vec![Some("cannot bind listener: Address already in use"), None]).await;
        let port = forward_tcp(addr, "emulator-5554", 0, 53319).await.unwrap();
        let commands = commands.lock().unwrap().clone();
        assert_eq!(commands.len(), 2, "one retry after the bind failure");
        assert!(commands[1].contains(&format!("forward:tcp:{port};tcp:53319")));

        let (addr, commands) = forward_server(vec![Some("device offline")]).await;
        let err = forward_tcp(addr, "emulator-5554", 0, 53319).await.unwrap_err();
        assert!(matches!(err, AdbError::Remote(ref r) if r == "device offline"));
        assert_eq!(commands.lock().unwrap().len(), 1, "other errors are not retried");

        let (addr, commands) = forward_server(vec![Some("cannot bind listener"); 10]).await;
        assert!(forward_tcp(addr, "emulator-5554", 0, 53319).await.is_err());
        assert_eq!(commands.lock().unwrap().len(), 1 + BIND_RETRIES as usize, "retries are bounded");

        let (addr, commands) = forward_server(vec![Some("cannot bind listener")]).await;
        assert!(forward_tcp(addr, "emulator-5554", 5000, 53319).await.is_err());
        assert_eq!(commands.lock().unwrap().len(), 1, "a port the caller chose is not swapped");
    }


    #[tokio::test]
    async fn mock_adb_server_forward_killforward_and_track_devices() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let commands = Arc::new(Mutex::new(Vec::<String>::new()));
        let commands_srv = commands.clone();

        let srv = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                let commands_srv = commands_srv.clone();
                tokio::spawn(async move {
                    let Ok(cmd) = read_payload(&mut socket).await else { return };
                    commands_srv.lock().unwrap().push(cmd.clone());
                    if cmd == "host:track-devices-l" {
                        let _ = socket.write_all(b"OKAY").await;
                        let update1 = "emulator-5554\tdevice\n";
                        let frame1 = format!("{:04x}{update1}", update1.len());
                        let _ = socket.write_all(frame1.as_bytes()).await;
                        let _ = socket.flush().await;
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    } else if cmd.starts_with("host-serial:") {
                        let _ = socket.write_all(b"OKAY").await;
                        let _ = socket.flush().await;
                    } else if cmd == "host:devices-l" {
                        let _ = socket.write_all(b"OKAY").await;
                        let body =
                            "List of devices attached\nemulator-5554 device model:Pixel_9 transport_id:4\n";
                        let frame = format!("{:04x}{body}", body.len());
                        let _ = socket.write_all(frame.as_bytes()).await;
                        let _ = socket.flush().await;
                    }
                });
            }
        });

        // Empty serial must be rejected before even connecting.
        let blocked_res = forward_tcp(addr, "   ", 0, 53319).await;
        assert!(matches!(blocked_res, Err(AdbError::BlockedSerial(_))));
        let blocked_kill = kill_forward_tcp(addr, "", 12345).await;
        assert!(matches!(blocked_kill, Err(AdbError::BlockedSerial(_))));

        let devs = list_devices(addr).await.unwrap();
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].serial, "emulator-5554");

        let port = forward_tcp(addr, "emulator-5554", 0, 53319).await.unwrap();
        assert_ne!(port, 0);
        kill_forward_tcp(addr, "emulator-5554", port).await.unwrap();

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let mut tracked = Vec::new();
        track_devices(addr, cancel_clone, |list| {
            tracked = list;
            cancel.cancel();
        })
        .await
        .unwrap();

        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].serial, "emulator-5554");
        assert!(tracked[0].is_online());

        let recorded = commands.lock().unwrap().clone();
        assert!(recorded.iter().any(|c| c == "host:devices-l"));
        assert!(
            recorded.iter().any(|c| c == &format!("host-serial:emulator-5554:forward:tcp:{port};tcp:53319"))
        );
        assert!(recorded.iter().any(|c| c == &format!("host-serial:emulator-5554:killforward:tcp:{port}")));
        assert!(recorded.iter().any(|c| c == "host:track-devices-l"));

        srv.abort();
    }

    #[tokio::test]
    async fn track_devices_falls_back_when_server_rejects_long_format() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let commands = Arc::new(Mutex::new(Vec::<String>::new()));
        let commands_srv = commands.clone();

        let srv = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                let commands_srv = commands_srv.clone();
                tokio::spawn(async move {
                    let Ok(cmd) = read_payload(&mut socket).await else { return };
                    commands_srv.lock().unwrap().push(cmd.clone());
                    if cmd == "host:track-devices-l" {
                        // An old server that does not know the long format.
                        let _ = socket.write_all(b"FAIL").await;
                        let reason = "unknown command";
                        let frame = format!("{:04x}{reason}", reason.len());
                        let _ = socket.write_all(frame.as_bytes()).await;
                        let _ = socket.flush().await;
                    } else if cmd == "host:track-devices" {
                        let _ = socket.write_all(b"OKAY").await;
                        let update1 = "R58M123ABC\tdevice\n";
                        let frame1 = format!("{:04x}{update1}", update1.len());
                        let _ = socket.write_all(frame1.as_bytes()).await;
                        let _ = socket.flush().await;
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                });
            }
        });

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let mut tracked = Vec::new();
        track_devices(addr, cancel_clone, |list| {
            tracked = list;
            cancel.cancel();
        })
        .await
        .unwrap();

        // The short record has no medium tokens: honest unknown, not wired.
        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].serial, "R58M123ABC");
        assert_eq!(tracked[0].medium(), TunnelMedium::Unknown);

        let recorded = commands.lock().unwrap().clone();
        assert!(recorded.iter().any(|c| c == "host:track-devices-l"));
        assert!(recorded.iter().any(|c| c == "host:track-devices"));

        srv.abort();
    }

    #[tokio::test]
    async fn mdns_services_query_parses_and_old_server_rejection_is_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let srv = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                tokio::spawn(async move {
                    let Ok(cmd) = read_payload(&mut socket).await else { return };
                    if cmd == "host:mdns:services" {
                        let _ = socket.write_all(b"OKAY").await;
                        let body = "adb-R58M123ABC-xyz\t_adb-tls-connect._tcp.\t192.168.1.40:39149\n";
                        let frame = format!("{:04x}{body}", body.len());
                        let _ = socket.write_all(frame.as_bytes()).await;
                        let _ = socket.flush().await;
                    } else {
                        // Old server without mDNS support.
                        let _ = socket.write_all(b"FAIL").await;
                        let reason = "unknown host service 'mdns:services'";
                        let frame = format!("{:04x}{reason}", reason.len());
                        let _ = socket.write_all(frame.as_bytes()).await;
                        let _ = socket.flush().await;
                    }
                });
            }
        });

        let services = mdns_services(addr).await.unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].instance, "adb-R58M123ABC-xyz");
        assert_eq!(
            mdns_serial_ip("adb-R58M123ABC-xyz._adb-tls-connect._tcp", &services).map(|ip| ip.to_string()),
            Some("192.168.1.40".to_owned())
        );

        // A rejection surfaces as an error so the caller falls back to
        // unknown instead of treating the serial as unresolvable-by-bug.
        async fn rejected(addr: SocketAddr) -> Result<Vec<MdnsService>, AdbError> {
            let mut stream = TcpStream::connect(addr).await?;
            send_command(&mut stream, "host:mdns:nope").await?;
            Ok(Vec::new())
        }
        assert!(matches!(rejected(addr).await, Err(AdbError::Remote(_))));

        srv.abort();
    }
}
