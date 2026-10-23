use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Addresses the stream proxy must never connect to, so that an extension cannot use it
/// to reach the user's router or other devices on the local network.
pub fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => forbidden_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return forbidden_v4(v4);
            }
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
        }
    }
}

fn forbidden_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast()
        || o[0] == 0
        || (o[0] == 100 && (o[1] & 0xc0) == 64)
}

/// DNS for requests that must stay on the public internet. Local answers are dropped when the
/// connection is made, so a name cannot pass a check and then resolve to the local network.
pub struct PublicResolver;

impl reqwest::dns::Resolve for PublicResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|a| !is_forbidden_ip(a.ip()))
                .collect();
            if addrs.is_empty() {
                return Err(format!("{host} resolves to a private address").into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The address of a URL whose host is an IP literal (those are connected to without DNS).
pub fn literal_ip(url: &url::Url) -> Option<IpAddr> {
    match url.host()? {
        url::Host::Ipv4(ip) => Some(IpAddr::V4(ip)),
        url::Host::Ipv6(ip) => Some(IpAddr::V6(ip)),
        url::Host::Domain(_) => None,
    }
}

/// Resolves the host of `url` and fails if any of its addresses is private or local.
pub async fn ensure_public(url: &url::Url) -> Result<(), String> {
    let host = url.host_str().ok_or("no host")?;
    let port = url.port_or_known_default().unwrap_or(443);
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    let addrs = tokio::net::lookup_host((bare, port))
        .await
        .map_err(|e| format!("dns {host}: {e}"))?;
    for a in addrs {
        if is_forbidden_ip(a.ip()) {
            return Err(format!("{host} resolves to a private address"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn urls_to_local_hosts_are_not_public() {
        for u in [
            "http://127.0.0.1:1/x",
            "http://localhost/x",
            "http://192.168.1.1/",
        ] {
            assert!(
                ensure_public(&url::Url::parse(u).unwrap()).await.is_err(),
                "{u}"
            );
        }
    }

    #[tokio::test]
    async fn resolver_refuses_names_that_point_to_local_addresses() {
        use reqwest::dns::Resolve;
        let resolved = PublicResolver.resolve("localhost".parse().unwrap()).await;
        assert!(resolved.is_err());
    }

    #[test]
    fn forbids_private_ranges() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.1",
            "172.16.5.4",
            "169.254.1.1",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "::1",
            "fe80::1",
            "fc00::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_forbidden_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["1.1.1.1", "95.213.1.1", "172.32.0.1", "2606:4700::1111"] {
            assert!(!is_forbidden_ip(ip.parse().unwrap()), "{ip}");
        }
    }
}
