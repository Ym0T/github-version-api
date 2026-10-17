//! Client IP resolution for rate limiting.
//!
//! `X-Forwarded-For` is only honoured when the TCP peer is a configured
//! trusted proxy. The header is then walked from right to left and the first
//! address that is not itself a trusted proxy is used. Entries further to the
//! left are client-controlled and are never trusted.

use std::fmt;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::Request;
use axum::http::request::Parts;
use tower_governor::GovernorError;
use tower_governor::key_extractor::KeyExtractor;

use crate::error::ApiError;
use crate::state::AppState;

/// Upper bound for `X-Forwarded-For` hops that are inspected.
const MAX_FORWARDED_HOPS: usize = 32;

/// An IP network in CIDR notation, e.g. `10.0.0.0/8` or `::1/128`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpNet {
    addr: IpAddr,
    prefix: u8,
}

impl IpNet {
    /// Networks larger than a /24 (IPv4) or /64 (IPv6).
    pub fn is_broad(&self) -> bool {
        match self.addr {
            IpAddr::V4(_) => self.prefix < 24,
            IpAddr::V6(_) => self.prefix < 64,
        }
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, canonical(ip)) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                prefix_matches(u32::from(net).into(), u32::from(ip).into(), self.prefix, 32)
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                prefix_matches(u128::from(net), u128::from(ip), self.prefix, 128)
            }
            _ => false,
        }
    }
}

fn prefix_matches(net: u128, ip: u128, prefix: u8, bits: u8) -> bool {
    if prefix == 0 {
        return true;
    }
    let shift = u32::from(bits - prefix);
    (net >> shift) == (ip >> shift)
}

impl FromStr for IpNet {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (addr, prefix) = match s.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (s, None),
        };
        let addr = canonical(
            addr.parse::<IpAddr>()
                .map_err(|_| format!("'{s}' is not a valid IP address or CIDR network"))?,
        );
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p
                .parse::<u8>()
                .ok()
                .filter(|p| *p <= max)
                .ok_or_else(|| format!("'{s}' has an invalid prefix length"))?,
            None => max,
        };
        Ok(Self { addr, prefix })
    }
}

impl fmt::Display for IpNet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

/// Maps IPv4-mapped IPv6 addresses (`::ffff:1.2.3.4`) to plain IPv4.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 @ IpAddr::V4(_) => v4,
    }
}

fn parse_forwarded_ip(entry: &str) -> Option<IpAddr> {
    let entry = entry.trim();
    entry
        .parse::<IpAddr>()
        .or_else(|_| entry.parse::<SocketAddr>().map(|s| s.ip()))
        .ok()
        .map(canonical)
}

/// Determines the client address for a request that arrived from `peer`.
pub fn resolve_client_ip<'a>(
    peer: IpAddr,
    forwarded_for: impl Iterator<Item = &'a str>,
    trusted_proxies: &[IpNet],
) -> IpAddr {
    let is_trusted = |ip: IpAddr| trusted_proxies.iter().any(|net| net.contains(ip));

    let peer = canonical(peer);
    if !is_trusted(peer) {
        return peer;
    }

    let hops: Vec<&str> = forwarded_for
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|hop| !hop.is_empty())
        .collect();

    let mut client = peer;
    for hop in hops.iter().rev().take(MAX_FORWARDED_HOPS) {
        match parse_forwarded_ip(hop) {
            Some(ip) => {
                client = ip;
                if !is_trusted(ip) {
                    return ip;
                }
            }
            // A malformed entry ends the trusted chain.
            None => return client,
        }
    }
    client
}

/// Rate limit bucket for an address. IPv6 clients usually control a whole
/// /64, so they share one bucket per /64 to prevent trivial address rotation.
pub fn rate_limit_key(ip: IpAddr) -> IpAddr {
    match canonical(ip) {
        IpAddr::V6(v6) => {
            let masked = u128::from(v6) & !((1u128 << 64) - 1);
            IpAddr::V6(Ipv6Addr::from(masked))
        }
        v4 @ IpAddr::V4(_) => v4,
    }
}

/// The rate limit key (client address, IPv6 per /64) of a request. The
/// server adds `ConnectInfo<SocketAddr>` to every request.
pub fn client_key<T>(req: &Request<T>, trusted_proxies: &[IpNet]) -> Option<IpAddr> {
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>()?.0.ip();
    Some(client_key_from_parts(peer, req.headers(), trusted_proxies))
}

fn client_key_from_parts(
    peer: IpAddr,
    headers: &axum::http::HeaderMap,
    trusted_proxies: &[IpNet],
) -> IpAddr {
    let forwarded = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok());
    rate_limit_key(resolve_client_ip(peer, forwarded, trusted_proxies))
}

/// Extractor for the client key, e.g. for per-client quotas.
#[derive(Debug, Clone, Copy)]
pub struct ClientAddr(pub IpAddr);

impl FromRequestParts<Arc<AppState>> for ClientAddr {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| {
                Self(client_key_from_parts(
                    info.0.ip(),
                    &parts.headers,
                    &state.config.trusted_proxies,
                ))
            })
            .ok_or_else(|| ApiError::internal("client address", "missing connection info"));
        std::future::ready(result)
    }
}

/// Rate limit key extractor based on [`client_key`].
#[derive(Debug, Clone)]
pub struct ClientIpKeyExtractor {
    trusted_proxies: Arc<[IpNet]>,
}

impl ClientIpKeyExtractor {
    pub fn new(trusted_proxies: &[IpNet]) -> Self {
        Self {
            trusted_proxies: trusted_proxies.into(),
        }
    }
}

impl KeyExtractor for ClientIpKeyExtractor {
    type Key = IpAddr;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        client_key(req, &self.trusted_proxies).ok_or(GovernorError::UnableToExtractKey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn nets(list: &[&str]) -> Vec<IpNet> {
        list.iter().map(|s| s.parse().unwrap()).collect()
    }

    #[test]
    fn parses_networks() {
        assert!(
            "10.0.0.0/8"
                .parse::<IpNet>()
                .unwrap()
                .contains(ip("10.1.2.3"))
        );
        assert!(
            !"10.0.0.0/8"
                .parse::<IpNet>()
                .unwrap()
                .contains(ip("11.0.0.1"))
        );
        assert!(
            "127.0.0.1"
                .parse::<IpNet>()
                .unwrap()
                .contains(ip("::ffff:127.0.0.1"))
        );
        assert!("fd00::/8".parse::<IpNet>().unwrap().contains(ip("fd12::1")));
        assert!(
            "0.0.0.0/0"
                .parse::<IpNet>()
                .unwrap()
                .contains(ip("8.8.8.8"))
        );
        assert!("10.0.0.0/33".parse::<IpNet>().is_err());
        assert!("nonsense".parse::<IpNet>().is_err());
    }

    #[test]
    fn ignores_forwarded_header_from_untrusted_peer() {
        let client = resolve_client_ip(ip("203.0.113.9"), ["1.2.3.4"].into_iter(), &[]);
        assert_eq!(client, ip("203.0.113.9"));

        let trusted = nets(&["10.0.0.1"]);
        let client = resolve_client_ip(ip("203.0.113.9"), ["1.2.3.4"].into_iter(), &trusted);
        assert_eq!(client, ip("203.0.113.9"));
    }

    #[test]
    fn uses_rightmost_untrusted_hop_behind_proxy() {
        let trusted = nets(&["10.0.0.0/8"]);
        // The client prepended a fake address; the proxy appended the real one.
        let client = resolve_client_ip(
            ip("10.0.0.1"),
            ["6.6.6.6, 198.51.100.7"].into_iter(),
            &trusted,
        );
        assert_eq!(client, ip("198.51.100.7"));

        // Multiple trusted proxies in a chain.
        let client = resolve_client_ip(
            ip("10.0.0.1"),
            ["6.6.6.6", "198.51.100.7, 10.0.0.2"].into_iter(),
            &trusted,
        );
        assert_eq!(client, ip("198.51.100.7"));
    }

    #[test]
    fn malformed_or_missing_header_falls_back_safely() {
        let trusted = nets(&["10.0.0.0/8"]);
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), std::iter::empty(), &trusted),
            ip("10.0.0.1")
        );
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), ["6.6.6.6, garbage"].into_iter(), &trusted),
            ip("10.0.0.1")
        );
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), ["198.51.100.7:4711"].into_iter(), &trusted),
            ip("198.51.100.7")
        );
    }

    #[test]
    fn ipv6_clients_share_a_slash_64_bucket() {
        assert_eq!(
            rate_limit_key(ip("2001:db8:1:2:aaaa::1")),
            rate_limit_key(ip("2001:db8:1:2:bbbb::2"))
        );
        assert_ne!(
            rate_limit_key(ip("2001:db8:1:2::1")),
            rate_limit_key(ip("2001:db8:1:3::1"))
        );
        assert_eq!(rate_limit_key(ip("192.0.2.1")), ip("192.0.2.1"));
    }
}
