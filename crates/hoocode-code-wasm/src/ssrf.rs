//! The SSRF block for plugin HTTP requests.
//!
//! Before a plugin's request is gated, its target host is resolved and every
//! address it maps to is checked. A request is refused when any address is
//! private, loopback, link-local, or otherwise not a public unicast address.
//! Literal IP hosts are checked the same way.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

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

/// Why [`resolve_and_check`] refused a target.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CheckError {
    /// The host did not resolve (or resolved to nothing). Reported to the
    /// plugin as a DNS error, not as a policy refusal.
    Dns(String),
    /// The host resolved to an address the block refuses.
    Blocked(String),
}

/// Resolve `target` and check every address. `Ok` returns the checked
/// addresses. Blocks on DNS, so call it on a blocking thread.
///
/// `allow_loopback` lets 127.0.0.0/8 and ::1 through. It is for tests only.
pub(crate) fn resolve_and_check(
    target: &Target,
    allow_loopback: bool,
) -> Result<Vec<SocketAddr>, CheckError> {
    let addrs: Vec<SocketAddr> = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .map_err(|e| CheckError::Dns(format!("cannot resolve {}: {e}", target.host)))?
        .collect();
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
    let first = ip.segments()[0];
    if first & 0xffc0 == 0xfe80 {
        return Some("link-local");
    }
    if first & 0xfe00 == 0xfc00 {
        return Some("unique-local");
    }
    if ip.is_multicast() {
        return Some("multicast");
    }
    if first == 0x2001 && ip.segments()[1] == 0x0db8 {
        return Some("documentation");
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
    fn literal_hosts_are_checked_without_dns() {
        let loopback = Target {
            host: "127.0.0.1".into(),
            port: 80,
        };
        assert!(resolve_and_check(&loopback, false).is_err());
        assert!(resolve_and_check(&loopback, true).is_ok());
        let private = Target {
            host: "10.0.0.1".into(),
            port: 80,
        };
        assert!(resolve_and_check(&private, true).is_err());
        let mapped = Target {
            host: "::ffff:127.0.0.1".into(),
            port: 80,
        };
        assert!(resolve_and_check(&mapped, false).is_err());
    }
}
