//! Client IP resolution: the Rust replacement for Caddy's
//! `trusted_proxies static private_ranges` + `trusted_proxies_strict` and the `X-Client-IP`
//! header it injected (spec 3.1, 3.3, 4.1, 18.4).
//!
//! Algorithm (Caddy's, verified against its strict mode semantics):
//! 1. The peer is the socket address. If the peer is **not** trusted, the client is the peer and
//!    `X-Forwarded-For` is ignored entirely (so a direct client cannot forge it).
//! 2. Otherwise walk the `X-Forwarded-For` entries from **right to left**. Skip entries that are
//!    trusted proxies. The first untrusted address is the client.
//! 3. If every entry is trusted, the client is the **leftmost** entry. With no usable entry the
//!    client is the peer. Entries that do not parse as IP addresses are skipped.
//!
//! The trusted set is the RFC 1918 ranges, loopback (`127/8`, `::1`), `fd00::/8`, plus any
//! `TRUSTED_PROXIES` entries (IPs or CIDRs). This crate binds its own listener, so no separate
//! `X-Client-IP` header is needed.

use std::net::IpAddr;

/// One CIDR block (a bare IP is a host route).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Net {
    V4 { addr: u32, prefix: u8 },
    V6 { addr: u128, prefix: u8 },
}

impl Net {
    fn parse(text: &str) -> Result<Self, String> {
        let (ip_part, prefix_part) = match text.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (text, None),
        };
        let ip: IpAddr = ip_part
            .trim()
            .parse()
            .map_err(|_| format!("not an IP address or CIDR: {text}"))?;
        match ip {
            IpAddr::V4(v4) => {
                let prefix = match prefix_part {
                    None => 32,
                    Some(p) => p
                        .parse::<u8>()
                        .ok()
                        .filter(|p| *p <= 32)
                        .ok_or_else(|| format!("bad prefix: {text}"))?,
                };
                Ok(Self::V4 {
                    addr: u32::from(v4) & mask32(prefix),
                    prefix,
                })
            }
            IpAddr::V6(v6) => {
                let prefix = match prefix_part {
                    None => 128,
                    Some(p) => p
                        .parse::<u8>()
                        .ok()
                        .filter(|p| *p <= 128)
                        .ok_or_else(|| format!("bad prefix: {text}"))?,
                };
                Ok(Self::V6 {
                    addr: u128::from(v6) & mask128(prefix),
                    prefix,
                })
            }
        }
    }

    fn contains(&self, ip: IpAddr) -> bool {
        match (self, ip) {
            (Self::V4 { addr, prefix }, IpAddr::V4(v4)) => {
                (u32::from(v4) & mask32(*prefix)) == *addr
            }
            (Self::V6 { addr, prefix }, IpAddr::V6(v6)) => {
                (u128::from(v6) & mask128(*prefix)) == *addr
            }
            _ => false,
        }
    }
}

fn mask32(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    }
}

fn mask128(prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix))
    }
}

/// The set of addresses whose `X-Forwarded-For` entries are believed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedProxies {
    nets: Vec<Net>,
}

impl TrustedProxies {
    /// Caddy's `private_ranges` plus loopback: 10/8, 172.16/12, 192.168/16, 127/8, ::1, fd00::/8.
    #[must_use]
    pub fn private_ranges() -> Self {
        let nets = [
            "10.0.0.0/8",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "127.0.0.0/8",
            "::1/128",
            "fd00::/8",
        ]
        .iter()
        .map(|s| Net::parse(s).expect("static CIDR"))
        .collect();
        Self { nets }
    }

    /// The private ranges plus the comma-separated `extra` entries (`TRUSTED_PROXIES`).
    ///
    /// # Errors
    /// Returns the first entry that is neither an IP nor a CIDR.
    pub fn with_extra(extra: &str) -> Result<Self, String> {
        let mut base = Self::private_ranges();
        for item in extra.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            base.nets.push(Net::parse(item)?);
        }
        Ok(base)
    }

    /// True when `ip` is a trusted proxy. IPv4-mapped IPv6 addresses are compared as IPv4.
    #[must_use]
    pub fn trusts(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        self.nets.iter().any(|n| n.contains(ip))
    }
}

/// Resolve the client key for one request (see the module docs).
///
/// `peer` is the socket address, or `None` when the connection carries none (tests without
/// `ConnectInfo`); that case yields `"unknown"`, as Python does. `xff` holds every
/// `X-Forwarded-For` header value in arrival order.
#[must_use]
pub fn resolve(peer: Option<IpAddr>, xff: &[String], trusted: &TrustedProxies) -> String {
    let Some(peer) = peer else {
        return "unknown".to_string();
    };
    let peer = peer.to_canonical();
    if !trusted.trusts(peer) {
        return peer.to_string();
    }
    let mut entries: Vec<IpAddr> = Vec::new();
    for value in xff {
        for part in value.split(',') {
            if let Ok(ip) = part.trim().parse::<IpAddr>() {
                entries.push(ip.to_canonical());
            }
        }
    }
    if entries.is_empty() {
        return peer.to_string();
    }
    for ip in entries.iter().rev() {
        if !trusted.trusts(*ip) {
            return ip.to_string();
        }
    }
    entries[0].to_string()
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    /// IPv6 literal for the tests.
    fn v6(text: &str) -> IpAddr {
        IpAddr::V6(text.parse::<Ipv6Addr>().expect("valid IPv6 literal"))
    }

    /// IPv4 literal for the tests.
    fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    use super::*;

    fn xff(v: &str) -> Vec<String> {
        vec![v.to_string()]
    }

    #[test]
    fn untrusted_peer_ignores_forwarded_for() {
        let t = TrustedProxies::private_ranges();
        // A public peer sending a spoofed header is the client itself.
        assert_eq!(
            resolve(Some(v4(198, 51, 100, 9)), &xff("10.1.1.1"), &t),
            "198.51.100.9"
        );
    }

    #[test]
    fn rightmost_untrusted_hop_is_the_client() {
        // Test checklist 15.1 #2: the rightmost public hop, not the leftmost.
        let t = TrustedProxies::private_ranges();
        let got = resolve(Some(v4(10, 0, 0, 9)), &xff("198.51.100.1, 203.0.113.7"), &t);
        assert_eq!(got, "203.0.113.7");
    }

    #[test]
    fn trusted_hops_on_the_right_are_skipped() {
        let t = TrustedProxies::private_ranges();
        // Cloudflare-like chain: client, public edge, then private hops.
        let got = resolve(
            Some(v4(127, 0, 0, 1)),
            &xff("203.0.113.7, 172.18.0.4, 10.2.0.1"),
            &t,
        );
        assert_eq!(got, "203.0.113.7");
    }

    #[test]
    fn all_trusted_chain_yields_leftmost() {
        let t = TrustedProxies::private_ranges();
        assert_eq!(
            resolve(Some(v4(10, 0, 0, 1)), &xff("192.168.1.5, 10.9.9.9"), &t),
            "192.168.1.5"
        );
    }

    #[test]
    fn no_forwarded_for_falls_back_to_peer() {
        // Test checklist 15.1 #3.
        let t = TrustedProxies::private_ranges();
        assert_eq!(resolve(Some(v4(10, 0, 0, 9)), &[], &t), "10.0.0.9");
    }

    #[test]
    fn unparseable_entries_are_skipped() {
        let t = TrustedProxies::private_ranges();
        assert_eq!(
            resolve(Some(v4(10, 0, 0, 1)), &xff("garbage, 203.0.113.7, ,"), &t),
            "203.0.113.7"
        );
    }

    #[test]
    fn multiple_header_lines_are_joined_in_order() {
        let t = TrustedProxies::private_ranges();
        let got = resolve(
            Some(v4(10, 0, 0, 1)),
            &["198.51.100.1".to_string(), "203.0.113.7".to_string()],
            &t,
        );
        assert_eq!(got, "203.0.113.7");
    }

    #[test]
    fn ipv4_mapped_peer_is_canonicalized() {
        let t = TrustedProxies::private_ranges();
        let mapped = IpAddr::V6("::ffff:10.0.0.9".parse().expect("literal"));
        assert_eq!(
            resolve(Some(mapped), &xff("203.0.113.7"), &t),
            "203.0.113.7"
        );
    }

    #[test]
    fn ipv6_unique_local_is_trusted() {
        let t = TrustedProxies::private_ranges();
        assert!(t.trusts(v6("fd12:3456::1")));
        assert!(!t.trusts(v6("2001:db8::1")));
    }

    #[test]
    fn extra_trusted_cidr_is_honoured() {
        let t = TrustedProxies::with_extra("100.64.0.0/10").expect("valid");
        assert!(t.trusts(v4(100, 127, 1, 1)));
        assert_eq!(
            resolve(Some(v4(100, 64, 0, 1)), &xff("203.0.113.7"), &t),
            "203.0.113.7"
        );
        assert!(TrustedProxies::with_extra("not-an-ip").is_err());
    }

    #[test]
    fn missing_peer_is_unknown() {
        let t = TrustedProxies::private_ranges();
        assert_eq!(resolve(None, &xff("203.0.113.7"), &t), "unknown");
    }
}
