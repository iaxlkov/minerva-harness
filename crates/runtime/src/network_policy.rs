//! Shared endpoint validation. Hosted Rig transport is deliberately unavailable until
//! a broker can enforce DNS, redirects and the connected peer for every request.
use std::net::IpAddr;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointPolicy {
    Local,
    Hosted,
}
impl EndpointPolicy {
    pub fn validate(&self, endpoint: &str) -> Result<(), String> {
        let u = url::Url::parse(endpoint).map_err(|_| "Invalid provider URL")?;
        let host = u.host_str().ok_or("Missing provider host")?;
        if !u.username().is_empty()
            || u.password().is_some()
            || u.query().is_some()
            || u.fragment().is_some()
        {
            return Err("Provider URL contains credentials, query or fragment".into());
        }
        if u.scheme() != "https" && !(*self == Self::Local && u.scheme() == "http") {
            return Err("HTTPS provider endpoint required".into());
        }
        if *self == Self::Hosted {
            if host.eq_ignore_ascii_case("localhost")
                || host.ends_with(".localhost")
                || host.eq_ignore_ascii_case("metadata.google.internal")
                || host.ends_with(".local")
            {
                return Err("Forbidden provider destination".into());
            }
            if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
                Self::validate_address(ip)?;
            }
        }
        Ok(())
    }
    pub fn validate_address(ip: IpAddr) -> Result<(), String> {
        let allowed = match ip {
            IpAddr::V4(v) => {
                let o = v.octets();
                !(v.is_private()
                    || v.is_loopback()
                    || v.is_link_local()
                    || v.is_unspecified()
                    || v.is_multicast()
                    || v.is_broadcast()
                    || o[0] == 0
                    || o[0] >= 240
                    || (o[0] == 100 && (64..=127).contains(&o[1]))
                    || (o[0] == 192 && o[1] == 0)
                    || (o[0] == 198 && (o[1] == 18 || o[1] == 19)))
            }
            IpAddr::V6(v) => {
                if let Some(v4) = v.to_ipv4_mapped() {
                    return Self::validate_address(v4.into());
                } else {
                    let o = v.octets();
                    !v.is_loopback()
                        && !v.is_unspecified()
                        && !v.is_multicast()
                        && (o[0] & 0xfe) != 0xfc
                        && !(o[0] == 0xfe && (o[1] & 0xc0) == 0x80)
                        && (o[0] & 0xe0) == 0x20
                }
            }
        };
        if allowed {
            Ok(())
        } else {
            Err("Forbidden provider destination".into())
        }
    }
    pub fn require_safe_transport(&self, endpoint: &str) -> Result<(), String> {
        self.validate(endpoint)?;
        if *self == Self::Hosted {
            return Err("Hosted provider transport unavailable: DNS, redirect and connected-peer enforcement requires an authenticated broker".into());
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocks_nonpublic_destinations() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "fe80::1",
            "fc00::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(
                EndpointPolicy::validate_address(ip.parse().unwrap()).is_err(),
                "{ip}"
            );
        }
    }
    #[test]
    fn hosted_dns_and_redirect_paths_fail_closed() {
        for url in [
            "https://public.example/v1",
            "https://8.8.8.8/v1",
            "https://127.0.0.1",
        ] {
            assert!(EndpointPolicy::Hosted.require_safe_transport(url).is_err());
        }
    }
}
