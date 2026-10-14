//! Trusted reverse-proxy address resolution.

use std::net::IpAddr;

use forge_config::IpNetwork;
use http::{HeaderMap, header::HeaderName};
use thiserror::Error;

const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
const MAX_FORWARDED_FOR_BYTES: usize = 4096;
const MAX_FORWARDED_HOPS: usize = 32;

/// Resolves the client address from an explicitly trusted proxy chain.
///
/// Forwarded values are completely ignored when the direct TCP peer is not in
/// `trusted_proxies`. When it is trusted, the chain is parsed strictly and
/// walked from right to left, stopping at the first untrusted address.
pub(crate) fn resolve_client_ip(
    peer: IpAddr,
    headers: &HeaderMap,
    trusted_proxies: &[IpNetwork],
) -> Result<IpAddr, ClientIpError> {
    if !is_trusted(peer, trusted_proxies) {
        return Ok(peer);
    }

    let values = headers.get_all(&X_FORWARDED_FOR);
    if values.iter().next().is_none() {
        return Err(ClientIpError::MissingForwardedFor);
    }

    let mut chain = Vec::new();
    let mut total_bytes = 0_usize;
    for value in values.iter() {
        let value = value
            .to_str()
            .map_err(|_| ClientIpError::MalformedForwardedFor)?;
        total_bytes = total_bytes
            .checked_add(value.len())
            .ok_or(ClientIpError::ForwardedForTooLarge)?;
        if total_bytes > MAX_FORWARDED_FOR_BYTES {
            return Err(ClientIpError::ForwardedForTooLarge);
        }
        for address in value.split(',') {
            if chain.len() == MAX_FORWARDED_HOPS {
                return Err(ClientIpError::TooManyForwardedHops);
            }
            let address = address.trim();
            if address.is_empty() {
                return Err(ClientIpError::MalformedForwardedFor);
            }
            chain.push(
                address
                    .parse::<IpAddr>()
                    .map_err(|_| ClientIpError::MalformedForwardedFor)?,
            );
        }
    }

    let mut client = peer;
    for forwarded in chain.into_iter().rev() {
        if !is_trusted(client, trusted_proxies) {
            break;
        }
        client = forwarded;
    }
    Ok(client)
}

fn is_trusted(address: IpAddr, trusted_proxies: &[IpNetwork]) -> bool {
    trusted_proxies
        .iter()
        .any(|network| network.contains(address))
}

/// Forwarded address metadata from a configured trusted proxy was unusable.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ClientIpError {
    /// A trusted proxy did not provide the required forwarding chain.
    #[error("trusted proxy did not provide X-Forwarded-For")]
    MissingForwardedFor,
    /// The forwarding chain contained an invalid address or header value.
    #[error("invalid X-Forwarded-For chain")]
    MalformedForwardedFor,
    /// The forwarding chain exceeded the configured parser bound.
    #[error("X-Forwarded-For header is too large")]
    ForwardedForTooLarge,
    /// The forwarding chain exceeded the configured hop bound.
    #[error("X-Forwarded-For chain has too many hops")]
    TooManyForwardedHops,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(value: &str) -> IpAddr {
        value.parse().expect("test IP address is valid")
    }

    fn networks(values: &[&str]) -> Vec<IpNetwork> {
        values
            .iter()
            .map(|value| value.parse().expect("test network is valid"))
            .collect()
    }

    #[test]
    fn ignores_forwarding_headers_from_untrusted_peers() {
        let headers = HeaderMap::from_iter([(
            X_FORWARDED_FOR,
            "203.0.113.9".parse().expect("header value is valid"),
        )]);

        let client = resolve_client_ip(ip("198.51.100.4"), &headers, &networks(&["10.0.0.0/8"]))
            .expect("untrusted peer metadata is ignored");

        assert_eq!(client, ip("198.51.100.4"));
    }

    #[test]
    fn trusts_forwarded_client_only_through_configured_proxy() {
        let headers = HeaderMap::from_iter([(
            X_FORWARDED_FOR,
            "203.0.113.9".parse().expect("header value is valid"),
        )]);

        let client = resolve_client_ip(ip("10.0.0.4"), &headers, &networks(&["10.0.0.0/8"]))
            .expect("trusted peer chain is valid");

        assert_eq!(client, ip("203.0.113.9"));
    }

    #[test]
    fn walks_multi_proxy_chain_from_the_right_and_ignores_spoofed_prefix() {
        let headers = HeaderMap::from_iter([(
            X_FORWARDED_FOR,
            "192.0.2.200, 203.0.113.9, 10.0.0.8"
                .parse()
                .expect("header value is valid"),
        )]);

        let client = resolve_client_ip(ip("10.0.0.9"), &headers, &networks(&["10.0.0.0/8"]))
            .expect("trusted peer chain is valid");

        assert_eq!(client, ip("203.0.113.9"));
    }

    #[test]
    fn rejects_missing_malformed_and_unbounded_trusted_proxy_metadata() {
        let trusted = networks(&["10.0.0.0/8"]);
        let empty = HeaderMap::new();
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), &empty, &trusted),
            Err(ClientIpError::MissingForwardedFor)
        );

        let malformed = HeaderMap::from_iter([(
            X_FORWARDED_FOR,
            "not-an-ip".parse().expect("header value is valid"),
        )]);
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), &malformed, &trusted),
            Err(ClientIpError::MalformedForwardedFor)
        );

        let too_many = HeaderMap::from_iter([(
            X_FORWARDED_FOR,
            (0..=MAX_FORWARDED_HOPS)
                .map(|_| "192.0.2.1")
                .collect::<Vec<_>>()
                .join(",")
                .parse()
                .expect("header value is valid"),
        )]);
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), &too_many, &trusted),
            Err(ClientIpError::TooManyForwardedHops)
        );

        let too_large = HeaderMap::from_iter([(
            X_FORWARDED_FOR,
            " ".repeat(MAX_FORWARDED_FOR_BYTES + 1)
                .parse()
                .expect("header value is valid"),
        )]);
        assert_eq!(
            resolve_client_ip(ip("10.0.0.1"), &too_large, &trusted),
            Err(ClientIpError::ForwardedForTooLarge)
        );
    }
}
