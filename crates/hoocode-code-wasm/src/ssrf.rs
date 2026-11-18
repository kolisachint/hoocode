//! The SSRF block for plugin HTTP requests.
//!
//! The check runs in two steps around the network gate:
//!
//! - Before the gate, [`parse_target`] and [`check_literal`] look only at the
//!   scheme and at a literal IP host. Nothing is resolved, so a plugin cannot
//!   leak data through DNS queries for a request the user never allowed, and
//!   cannot probe internal names by the error it gets back.
//! - After the gate allows the request, [`resolve_and_check`] resolves the host
//!   and refuses it when any address is private, loopback, link-local, or
//!   otherwise not a public unicast address.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;

use http::Uri;

/// The scheme, host and port of a request target, checked to be http or https.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    /// The host without IPv6 brackets.
    pub(crate) host: String,
    pub(crate) port: u16,
}

/// Read the scheme, host and port of `uri`. Only `http` and `https` pass.
pub(crate) fn parse_target(uri: &Uri) -> Result<Target, String> {
    let default_port = match uri.scheme_str() {
        Some("http") => 80,
        Some("https") => 443,
        Some(other) => return Err(format!("scheme {other:?} is not allowed")),
        None => return Err("request has no scheme".to_owned()),
    };
    let host = uri
        .host()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| "request has no host".to_owned())?;
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = uri.port_u16().unwrap_or(default_port);
    Ok(Target { host, port })
}

/// The check before the gate. A literal IP host is checked against the block;
/// a host name passes, because it cannot be checked without resolving it.
pub(crate) fn check_literal(target: &Target, allow_loopback: bool) -> Result<(), String> {
    match target.host.parse::<IpAddr>() {
        Ok(ip) => match blocked_reason(ip, allow_loopback) {
            Some(reason) => Err(format!("{} is {reason}", target.host)),
            None => Ok(()),
        },
        Err(_) => Ok(()),
    }
}

/// Resolves a host name to socket addresses. A type so tests can count lookups.
pub(crate) type Resolver = dyn Fn(&str, u16) -> io::Result<Vec<SocketAddr>> + Send + Sync;

/// A shared [`Resolver`].
pub(crate) type Resolve = Arc<Resolver>;

/// The resolver that asks the operating system.
pub(crate) fn system_resolver() -> Resolve {
    Arc::new(|host: &str, port: u16| -> io::Result<Vec<SocketAddr>> {
        Ok((host, port).to_socket_addrs()?.collect())
    })
}

/// Why [`resolve_and_check`] refused a target. The guest sees the same error
/// for both; the variant only names the reason in the host log.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CheckError {
    /// The host did not resolve (or resolved to nothing).
    Dns(String),
    /// The host resolved to an address the block refuses.
    Blocked(String),
}

/// Resolve `target` through `resolve` and check every address. `Ok` returns the
/// checked addresses, which the send connects to. A literal IP host is not
/// resolved. Blocks on DNS, so call it on a blocking thread.
///
/// `allow_loopback` lets 127.0.0.0/8 and ::1 through. It is for tests only.
pub(crate) fn resolve_and_check(
    target: &Target,
    allow_loopback: bool,
    resolve: &Resolver,
) -> Result<Vec<SocketAddr>, CheckError> {
    let addrs = match target.host.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, target.port)],
        Err(_) => resolve(&target.host, target.port)
            .map_err(|e| CheckError::Dns(format!("cannot resolve {}: {e}", target.host)))?,
    };
    if addrs.is_empty() {
        return Err(CheckError::Dns(format!(
            "{} resolves to no address",
            target.host
        )));
    }
    for addr in &addrs {
        if let Some(reason) = blocked_reason(addr.ip(), allow_loopback) {
            return Err(CheckError::Blocked(format!(
                "{} resolves to {} ({reason})",
                target.host,
                addr.ip()
            )));
        }
    }
    Ok(addrs)
}

/// Why `ip` may not be reached, or `None` when it is a public address.
pub(crate) fn blocked_reason(ip: IpAddr, allow_loopback: bool) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => blocked_v4(v4, allow_loopback),
        IpAddr::V6(v6) => blocked_v6(v6, allow_loopback),
    }
}

fn blocked_v4(ip: Ipv4Addr, allow_loopback: bool) -> Option<&'static str> {
    let [a, b, c, _] = ip.octets();
    if a == 127 {
        return if allow_loopback {
            None
        } else {
            Some("loopback")
        };
    }
    if ip.is_broadcast() {
        return Some("broadcast");
    }
    if ip.is_unspecified() || a == 0 {
        return Some("unspecified or 0.0.0.0/8");
    }
    if a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168) {
        return Some("private");
    }
    if a == 169 && b == 254 {
        return Some("link-local");
    }
    if a == 100 && (64..=127).contains(&b) {
        return Some("carrier-grade NAT");
    }
    if (a, b, c) == (192, 0, 2) || (a, b, c) == (198, 51, 100) || (a, b, c) == (203, 0, 113) {
        return Some("documentation");
    }
    if (a, b, c) == (192, 0, 0) {
        return Some("IETF protocol assignments");
    }
    if (a, b, c) == (192, 88, 99) {
        return Some("6to4 relay anycast");
    }
    if a == 198 && (b == 18 || b == 19) {
        return Some("benchmarking");
    }
    if ip.is_multicast() {
        return Some("multicast");
    }
    if a >= 240 {
        return Some("reserved");
    }
    None
}

fn blocked_v6(ip: Ipv6Addr, allow_loopback: bool) -> Option<&'static str> {
    if ip == Ipv6Addr::LOCALHOST {
        return if allow_loopback {
            None
        } else {
            Some("loopback")
        };
    }
    // IPv4-mapped (::ffff:a.b.c.d) and IPv4-compatible (::a.b.c.d) addresses
    // are checked as the IPv4 address they carry.
    if let Some(v4) = ip.to_ipv4() {
        return blocked_v4(v4, allow_loopback);
    }
    if ip.is_unspecified() {
        return Some("unspecified");
    }
    let s = ip.segments();
    let o = ip.octets();
    if s[0] & 0xffc0 == 0xfe80 {
        return Some("link-local");
    }
    if s[0] & 0xffc0 == 0xfec0 {
        return Some("site-local");
    }
    if s[0] & 0xfe00 == 0xfc00 {
        return Some("unique-local");
    }
    if ip.is_multicast() {
        return Some("multicast");
    }
    if s[0] == 0x2001 && s[1] == 0x0db8 {
        return Some("documentation");
    }
    // NAT64 well-known prefix 64:ff9b::/96 (RFC 6052): IPv4 in the last 32 bits.
    if o[0..12] == [0, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0] {
        let v4 = Ipv4Addr::new(o[12], o[13], o[14], o[15]);
        return blocked_v4(v4, allow_loopback).map(|_| "NAT64 of a blocked IPv4 address");
    }
    // NAT64 local-use prefix 64:ff9b:1::/48 (RFC 8215): IPv4 in bits 48-63 and
    // 72-87, with the reserved u octet (bits 64-71) between them.
    if o[0..6] == [0, 0x64, 0xff, 0x9b, 0, 1] {
        let v4 = Ipv4Addr::new(o[6], o[7], o[9], o[10]);
        return blocked_v4(v4, allow_loopback).map(|_| "NAT64 of a blocked IPv4 address");
    }
    // 6to4 2002::/16 (RFC 3056): IPv4 in bits 16-48.
    if s[0] == 0x2002 {
        let v4 = Ipv4Addr::new(o[2], o[3], o[4], o[5]);
        return blocked_v4(v4, allow_loopback).map(|_| "6to4 of a blocked IPv4 address");
    }
    // Teredo 2001::/32 (RFC 4380): tunnelled addresses are not checked, so the
    // whole prefix is blocked.
    if s[0] == 0x2001 && s[1] == 0 {
        return Some("Teredo");
    }
    // Discard-only 100::/64 (RFC 6666).
    if s[..4] == [0x0100, 0, 0, 0] {
        return Some("discard-only");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(s: &str) -> IpAddr {
        IpAddr::V4(s.parse().unwrap())
    }

    fn v6(s: &str) -> IpAddr {
        IpAddr::V6(s.parse().unwrap())
    }

    #[test]
    fn blocks_private_and_special_ipv4() {
        for ip in [
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "0.1.2.3",
            "100.64.0.1",
            "100.127.255.255",
            "255.255.255.255",
            "224.0.0.1",
            "192.0.2.1",
            "198.51.100.1",
            "203.0.113.7",
            "240.0.0.1",
        ] {
            assert!(
                blocked_reason(v4(ip), false).is_some(),
                "{ip} should be blocked"
            );
        }
    }

    #[test]
    fn blocks_ipv4_protocol_assignments_benchmarking_and_6to4_relay() {
        for ip in [
            "192.0.0.0",
            "192.0.0.8",
            "192.0.0.255",
            "198.18.0.1",
            "198.19.255.255",
            "192.88.99.1",
        ] {
            assert!(
                blocked_reason(v4(ip), false).is_some(),
                "{ip} should be blocked"
            );
        }
        for ip in ["192.0.1.1", "198.20.0.1", "198.17.255.255", "192.88.100.1"] {
            assert!(blocked_reason(v4(ip), false).is_none(), "{ip} should pass");
        }
    }

    #[test]
    fn allows_public_ipv4() {
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "172.15.0.1",
            "172.32.0.1",
            "100.63.0.1",
            "100.128.0.1",
        ] {
            assert!(blocked_reason(v4(ip), false).is_none(), "{ip} should pass");
        }
    }

    #[test]
    fn loopback_is_blocked_unless_allowed() {
        assert!(blocked_reason(v4("127.0.0.1"), false).is_some());
        assert!(blocked_reason(v4("127.255.0.9"), false).is_some());
        assert!(blocked_reason(v4("127.0.0.1"), true).is_none());
        assert!(blocked_reason(v6("::1"), false).is_some());
        assert!(blocked_reason(v6("::1"), true).is_none());
        // Loopback allowance covers nothing else.
        assert!(blocked_reason(v4("10.0.0.1"), true).is_some());
        assert!(blocked_reason(v6("fe80::1"), true).is_some());
        assert!(blocked_reason(v4("0.0.0.0"), true).is_some());
    }

    #[test]
    fn blocks_special_ipv6() {
        for ip in [
            "::",
            "fe80::1",
            "febf::1",
            "fc00::1",
            "fd12:3456::1",
            "ff02::1",
            "2001:db8::1",
        ] {
            assert!(
                blocked_reason(v6(ip), false).is_some(),
                "{ip} should be blocked"
            );
        }
        assert!(blocked_reason(v6("2606:4700:4700::1111"), false).is_none());
    }

    #[test]
    fn blocks_site_local_teredo_and_discard_ipv6() {
        for ip in [
            "fec0::1",
            "feff::1",
            "2001::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "100::1",
            "100::",
        ] {
            assert!(
                blocked_reason(v6(ip), false).is_some(),
                "{ip} should be blocked"
            );
        }
        // Outside the /64 discard prefix, and outside the other blocks.
        assert!(blocked_reason(v6("100:0:0:1::1"), false).is_none());
        assert!(blocked_reason(v6("2001:4860:4860::8888"), false).is_none());
    }

    #[test]
    fn blocks_nat64_wellknown_with_a_private_embedded_ipv4() {
        // 64:ff9b::a9fe:a9fe is 169.254.169.254.
        assert!(blocked_reason(v6("64:ff9b::a9fe:a9fe"), false).is_some());
        assert!(blocked_reason(v6("64:ff9b::a00:1"), false).is_some());
        assert!(blocked_reason(v6("64:ff9b::7f00:1"), false).is_some());
        assert!(blocked_reason(v6("64:ff9b::7f00:1"), true).is_none());
        // A public embedded address passes: 8.8.8.8.
        assert!(blocked_reason(v6("64:ff9b::808:808"), false).is_none());
    }

    #[test]
    fn blocks_nat64_local_use_with_a_private_embedded_ipv4() {
        // 64:ff9b:1::/48 holds the IPv4 in bits 48-63 and 72-87 (RFC 8215).
        let embedded = |v4: [u8; 4]| {
            let mut o = [0u8; 16];
            o[..6].copy_from_slice(&[0, 0x64, 0xff, 0x9b, 0, 1]);
            o[6] = v4[0];
            o[7] = v4[1];
            o[9] = v4[2];
            o[10] = v4[3];
            IpAddr::V6(Ipv6Addr::from(o))
        };
        assert!(blocked_reason(embedded([169, 254, 169, 254]), false).is_some());
        assert!(blocked_reason(embedded([10, 0, 0, 1]), false).is_some());
        assert!(blocked_reason(embedded([8, 8, 8, 8]), false).is_none());
    }

    #[test]
    fn blocks_6to4_with_a_private_embedded_ipv4() {
        // 2002:a9fe:a9fe:: embeds 169.254.169.254 in bits 16-48.
        assert!(blocked_reason(v6("2002:a9fe:a9fe::"), false).is_some());
        assert!(blocked_reason(v6("2002:7f00:1::"), false).is_some());
        assert!(blocked_reason(v6("2002:7f00:1::"), true).is_none());
        assert!(blocked_reason(v6("2002:808:808::"), false).is_none());
    }

    #[test]
    fn mapped_and_compatible_addresses_use_their_ipv4() {
        assert!(blocked_reason(v6("::ffff:127.0.0.1"), false).is_some());
        assert!(blocked_reason(v6("::ffff:10.0.0.1"), false).is_some());
        assert!(blocked_reason(v6("::ffff:169.254.1.1"), false).is_some());
        assert!(blocked_reason(v6("::10.0.0.1"), false).is_some());
        assert!(blocked_reason(v6("::ffff:8.8.8.8"), false).is_none());
        assert!(blocked_reason(v6("::ffff:127.0.0.1"), true).is_none());
    }

    #[test]
    fn parse_target_accepts_only_http_and_https() {
        let t = parse_target(&"http://example.com/x?y=1".parse().unwrap()).unwrap();
        assert_eq!(
            t,
            Target {
                host: "example.com".into(),
                port: 80
            }
        );
        let t = parse_target(&"https://example.com:8443/".parse().unwrap()).unwrap();
        assert_eq!(t.port, 8443);
        let t = parse_target(&"http://[::1]:9/".parse().unwrap()).unwrap();
        assert_eq!(
            t,
            Target {
                host: "::1".into(),
                port: 9
            }
        );
        for bad in [
            "ftp://example.com/",
            "file://localhost/etc/passwd",
            "gopher://x/",
        ] {
            let err = parse_target(&bad.parse().unwrap()).unwrap_err();
            assert!(err.contains("not allowed"), "{bad}: {err}");
        }
    }

    #[test]
    fn literal_hosts_are_checked_before_the_gate_without_dns() {
        let loopback = Target {
            host: "127.0.0.1".into(),
            port: 80,
        };
        assert!(check_literal(&loopback, false).is_err());
        assert!(check_literal(&loopback, true).is_ok());
        let private = Target {
            host: "10.0.0.1".into(),
            port: 80,
        };
        assert!(check_literal(&private, true).is_err());
        let mapped = Target {
            host: "::ffff:127.0.0.1".into(),
            port: 80,
        };
        assert!(check_literal(&mapped, false).is_err());
        let name = Target {
            host: "localhost".into(),
            port: 80,
        };
        assert!(check_literal(&name, false).is_ok());
    }

    #[test]
    fn literal_hosts_are_not_resolved() {
        let refuse: &Resolver = &|_: &str, _: u16| -> io::Result<Vec<SocketAddr>> {
            panic!("a literal host must not be resolved")
        };
        let public = Target {
            host: "8.8.8.8".into(),
            port: 443,
        };
        let addrs = resolve_and_check(&public, false, refuse).unwrap();
        assert_eq!(addrs, vec!["8.8.8.8:443".parse().unwrap()]);
    }

    #[test]
    fn every_resolved_address_is_checked() {
        let target = Target {
            host: "mixed.example".into(),
            port: 80,
        };
        let mixed: &Resolver = &|_: &str, port: u16| {
            Ok(vec![
                SocketAddr::new(v4("8.8.8.8"), port),
                SocketAddr::new(v4("10.0.0.5"), port),
            ])
        };
        assert!(matches!(
            resolve_and_check(&target, false, mixed),
            Err(CheckError::Blocked(_))
        ));
        let nothing: &Resolver = &|_: &str, _: u16| Ok(Vec::new());
        assert!(matches!(
            resolve_and_check(&target, false, nothing),
            Err(CheckError::Dns(_))
        ));
    }
}
