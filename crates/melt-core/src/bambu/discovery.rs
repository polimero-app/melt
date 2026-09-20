use std::{
    collections::BTreeMap,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

use serde::Deserialize;
use thiserror::Error;
use url::Url;

use super::validate_host;

const BAMBU_BROADCAST_PORT: u16 = 2021;
// ponytail: printers alternate their announcement destination between ports
// 1990 and 2021 on a roughly 5 s period, so a 2021-only listener sees a given
// device about every 10 s. That is inside the default scan window. Bind 1990
// too only if a scan is observed to miss a printer, rather than doubling the
// socket count against a ceiling nobody has hit.
const SSDP_MULTICAST: &str = "239.255.255.250:1900";
const SSDP_TARGET: &str = "urn:bambulab-com:device:3dprinter:1";
const POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredPrinter {
    pub driver: &'static str,
    pub host: String,
    pub serial: String,
    /// Exactly as broadcast. `DevModel.bambu.com` carries a `model_id` such
    /// as `C12`, which is the evidence and what `printer add --model` takes.
    pub model: String,
    /// The marketing name for `model`, or `model` itself when unrecognised.
    pub display_model: String,
    pub name: String,
    pub firmware: Option<String>,
    pub schema_version: Option<String>,
    pub connect_mode: Option<String>,
    pub bind_state: Option<String>,
    pub security_mode: Option<String>,
    pub interface: Option<String>,
}

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("Bambu LAN discovery is unavailable")]
    Unavailable,
}

/// Scans Bambu's SSDP and UDP-announcement channels for the requested duration.
///
/// Discovery is advisory: no profile or keychain entry is changed by this scan.
pub fn discover(timeout: Duration) -> Result<Vec<DiscoveredPrinter>, DiscoveryError> {
    let deadline = Instant::now() + timeout;
    let ssdp = open_ssdp();
    let announcements = open_announcements();
    if ssdp.is_err() && announcements.is_err() {
        return Err(DiscoveryError::Unavailable);
    }

    let mut found = Vec::new();
    let mut buffer = [0; 4096];
    let mut sockets = Vec::new();
    if let Ok(socket) = ssdp {
        sockets.push((socket, false));
    }
    if let Ok(socket) = announcements {
        sockets.push((socket, true));
    }

    while Instant::now() < deadline {
        for (socket, udp_announcement) in &sockets {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let _ = socket.set_read_timeout(Some(
                remaining.min(POLL_INTERVAL).max(Duration::from_millis(1)),
            ));
            match socket.recv_from(&mut buffer) {
                Ok((size, source)) => {
                    let entry = if *udp_announcement {
                        parse_udp_announcement(&buffer[..size], source.ip())
                    } else {
                        parse_ssdp_response(&buffer[..size], source.ip())
                    };
                    if let Some(entry) = entry {
                        found.push(entry);
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => {}
            }
        }
    }
    Ok(merge(found))
}

fn open_ssdp() -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.send_to(
        format!(
            "M-SEARCH * HTTP/1.1\r\nHOST: {SSDP_MULTICAST}\r\nMAN: \"ssdp:discover\"\r\nMX: 3\r\nST: {SSDP_TARGET}\r\n\r\n"
        ).as_bytes(),
        SSDP_MULTICAST,
    )?;
    Ok(socket)
}

/// Binds the Bambu announcement port with address reuse, so a scan can share
/// it with another listener (Bambu Studio, OrcaSlicer) that also opted into
/// reuse. Unix needs `SO_REUSEPORT` as well; Windows gets the sharing
/// semantics from `SO_REUSEADDR` alone. Bambu announcements are broadcast, so
/// the kernel delivers a copy to every socket in the reuse group rather than
/// load-balancing to one.
// ponytail: a co-bound process that never set SO_REUSEPORT still wins the
// bind and we fall back to SSDP-only, silently. Surface that in discover()'s
// return value if it turns out developers need to know why a scan came up
// short, rather than guessing from an empty printer list.
fn open_announcements() -> io::Result<UdpSocket> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, BAMBU_BROADCAST_PORT)).into())?;
    Ok(socket.into())
}

#[derive(Deserialize)]
struct Announcement {
    #[serde(default)]
    dev_name: String,
    #[serde(default)]
    sn: String,
    #[serde(default)]
    ip: String,
    #[serde(default)]
    dev_product_name: String,
    #[serde(default)]
    dev_ver: Option<String>,
    #[serde(default)]
    dev_schema: Option<String>,
    #[serde(default)]
    connect: Option<String>,
    #[serde(default)]
    bind: Option<String>,
    #[serde(default)]
    security: Option<String>,
    #[serde(default, rename = "ifname")]
    interface: Option<String>,
}

fn parse_udp_announcement(payload: &[u8], source: IpAddr) -> Option<DiscoveredPrinter> {
    let announcement: Announcement = serde_json::from_slice(payload).ok()?;
    if announcement.sn.is_empty() && announcement.dev_product_name.is_empty() {
        return None;
    }
    let source = source.to_string();
    let host = if validate_host(&announcement.ip).is_ok() {
        announcement.ip
    } else {
        source
    };
    let mut printer = printer(
        host,
        announcement.sn,
        announcement.dev_product_name,
        announcement.dev_name,
    );
    printer.firmware = announcement.dev_ver;
    printer.schema_version = announcement.dev_schema;
    printer.connect_mode = announcement.connect;
    printer.bind_state = announcement.bind;
    printer.security_mode = announcement.security;
    printer.interface = announcement.interface;
    Some(printer)
}

fn parse_ssdp_response(payload: &[u8], source: IpAddr) -> Option<DiscoveredPrinter> {
    let text = std::str::from_utf8(payload).ok()?;
    let headers = text
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_ascii_uppercase(), value.trim()))
        .collect::<BTreeMap<_, _>>();
    let target = headers.get("ST").or_else(|| headers.get("NT"))?;
    if !target.to_ascii_lowercase().contains(SSDP_TARGET) {
        return None;
    }
    let source = source.to_string();
    // A real printer sends a bare IPv4 here with no scheme, so `Url::parse`
    // fails and the datagram's source address is used instead. That is the
    // same printer and the harder of the two to forge; the fallback is the
    // intended path, not a parser bug.
    let host = headers
        .get("LOCATION")
        .and_then(|location| Url::parse(location).ok())
        .and_then(|location| location.host_str().map(str::to_owned))
        .filter(|host| validate_host(host).is_ok())
        .unwrap_or(source);
    // Printers send the serial bare (`USN: 22E8BJ610801473`); the
    // `uuid:<serial>::<target>` form comes from emulators and from Bambu's
    // own SSDP samples. Requiring the prefix left every real printer with an
    // empty serial, which `PrinterPresence::observe` then discards.
    let serial = headers
        .get("USN")
        .map(|usn| usn.strip_prefix("uuid:").unwrap_or(usn))
        .and_then(|usn| usn.split("::").next())
        .unwrap_or_default()
        .trim()
        .to_owned();
    Some(printer(
        host,
        serial,
        headers
            .get("DEVMODEL.BAMBU.COM")
            .copied()
            .unwrap_or_default()
            .to_owned(),
        headers
            .get("DEVNAME.BAMBU.COM")
            .copied()
            .unwrap_or_default()
            .to_owned(),
    ))
}

fn printer(host: String, serial: String, model: String, name: String) -> DiscoveredPrinter {
    DiscoveredPrinter {
        driver: "bambu-lan",
        host,
        serial,
        display_model: display_model(&model),
        model,
        name,
        firmware: None,
        schema_version: None,
        connect_mode: None,
        bind_state: None,
        security_mode: None,
        interface: None,
    }
}

fn display_model(model: &str) -> String {
    match super::ModelIdentity::parse(model).canonical {
        super::CanonicalModel::Unknown => model.to_owned(),
        canonical => canonical.display_name().to_owned(),
    }
}

fn merge(entries: Vec<DiscoveredPrinter>) -> Vec<DiscoveredPrinter> {
    let mut merged = Vec::<DiscoveredPrinter>::new();
    for entry in entries {
        if let Some(existing) = merged.iter_mut().find(|existing| {
            existing.host == entry.host
                || (!entry.serial.is_empty() && existing.serial == entry.serial)
        }) {
            if existing.serial.is_empty() {
                existing.serial = entry.serial;
            }
            if existing.model.is_empty() {
                existing.display_model = entry.display_model;
                existing.model = entry.model;
            }
            if existing.name.is_empty() {
                existing.name = entry.name;
            }
        } else {
            merged.push(entry);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn parses_bambu_udp_announcements_and_rejects_unrelated_json() {
        let printer = parse_udp_announcement(
            br#"{"dev_name":"My P1S","sn":"SN001","ip":"192.0.2.10","dev_product_name":"P1S"}"#,
            IpAddr::from_str("192.0.2.20").unwrap(),
        )
        .unwrap();
        assert_eq!(printer.host, "192.0.2.10");
        assert_eq!(printer.serial, "SN001");
        assert!(parse_udp_announcement(b"{}", IpAddr::V4(Ipv4Addr::LOCALHOST)).is_none());
    }

    #[test]
    fn parses_bambu_ssdp_and_merges_partial_results() {
        // The shape a printer actually sends: bare serial, bare IPv4
        // location, a `model_id` rather than a marketing name.
        let discovered = parse_ssdp_response(
            b"HTTP/1.1 200 OK\r\nST: urn:bambulab-com:device:3dprinter:1\r\nLOCATION: 192.0.2.10\r\nUSN: SN001\r\nDevModel.bambu.com: C12\r\n\r\n",
            IpAddr::from_str("192.0.2.10").unwrap(),
        )
        .unwrap();
        assert_eq!(discovered.host, "192.0.2.10");
        assert_eq!(discovered.serial, "SN001");
        assert_eq!(discovered.model, "C12");
        assert_eq!(discovered.display_model, "Bambu Lab P1S");

        // The `uuid:<serial>::<target>` form still parses; emulators and
        // Bambu's own SSDP samples use it.
        let prefixed = parse_ssdp_response(
            b"HTTP/1.1 200 OK\r\nST: urn:bambulab-com:device:3dprinter:1\r\nLOCATION: http://192.0.2.10/\r\nUSN: uuid:SN001::urn:bambulab-com:device:3dprinter:1\r\nDevModel.bambu.com: P1S\r\n\r\n",
            IpAddr::from_str("192.0.2.20").unwrap(),
        )
        .unwrap();
        assert_eq!(prefixed.host, "192.0.2.10");
        assert_eq!(prefixed.serial, "SN001");

        let entries = merge(vec![
            printer("192.0.2.10".into(), "".into(), "C12".into(), "".into()),
            printer(
                "192.0.2.10".into(),
                "SN001".into(),
                "".into(),
                "My P1S".into(),
            ),
        ]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].serial, "SN001");
        assert_eq!(entries[0].model, "C12");
        assert_eq!(entries[0].display_model, "Bambu Lab P1S");
        assert_eq!(entries[0].name, "My P1S");
    }

    /// An unrecognised broadcast still has to reach the operator; the raw
    /// value is what `printer add --model` consumes either way.
    #[test]
    fn an_unknown_model_code_is_displayed_verbatim() {
        let discovered = printer(
            "192.0.2.10".into(),
            "SN001".into(),
            "Z9".into(),
            "Future".into(),
        );
        assert_eq!(discovered.model, "Z9");
        assert_eq!(discovered.display_model, "Z9");
    }

    #[test]
    fn announcement_socket_shares_the_port_with_another_slicer() {
        // Bambu Studio and OrcaSlicer hold this port; a scan must not go
        // silently SSDP-only just because one of them is open. What actually
        // matters is that a broadcast announcement reaches both sockets, not
        // just that both binds succeed.
        let first = open_announcements().expect(
            "first bind (if this fails, something else already holds UDP 2021 without SO_REUSEPORT)",
        );
        let second = open_announcements().expect("second bind while the first is held");
        assert_eq!(first.local_addr().unwrap().port(), BAMBU_BROADCAST_PORT);
        assert_eq!(second.local_addr().unwrap().port(), BAMBU_BROADCAST_PORT);

        let read_timeout = Some(Duration::from_secs(1));
        first.set_read_timeout(read_timeout).unwrap();
        second.set_read_timeout(read_timeout).unwrap();

        let sender = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        sender.set_broadcast(true).unwrap();
        sender
            .send_to(b"announcement", (Ipv4Addr::BROADCAST, BAMBU_BROADCAST_PORT))
            .unwrap();

        let mut buffer = [0; 32];
        let (size, _) = first
            .recv_from(&mut buffer)
            .expect("first socket must receive the broadcast");
        assert_eq!(&buffer[..size], b"announcement");
        let (size, _) = second
            .recv_from(&mut buffer)
            .expect("second socket must receive the broadcast");
        assert_eq!(&buffer[..size], b"announcement");
    }
}
