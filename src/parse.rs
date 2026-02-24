use anyhow::{bail, Context, Result};
use std::str::FromStr;

/// A validated port number (1–65535).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Port(u16);

impl Port {
    pub fn get(self) -> u16 {
        self.0
    }
}

impl FromStr for Port {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let n: u16 = s.parse().context("invalid port number")?;
        if n == 0 {
            bail!("port must be 1–65535, got 0");
        }
        Ok(Port(n))
    }
}

/// A local:remote port mapping for `punch in`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapping {
    pub local: u16,
    pub remote: u16,
}

impl FromStr for Mapping {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let (l, r) = s.split_once(':').context("mapping must be <local>:<remote>")?;
        let local: Port = l.parse().context("invalid local port")?;
        let remote: Port = r.parse().context("invalid remote port")?;
        Ok(Mapping {
            local: local.get(),
            remote: remote.get(),
        })
    }
}

pub fn parse_ports(args: &[String]) -> Result<Vec<Port>> {
    let mut ports = Vec::with_capacity(args.len());
    for arg in args {
        let port: Port = arg.parse()?;
        if ports.contains(&port) {
            bail!("duplicate port: {}", port.get());
        }
        ports.push(port);
    }
    Ok(ports)
}

pub fn parse_mappings(args: &[String]) -> Result<Vec<Mapping>> {
    let mut mappings = Vec::with_capacity(args.len());
    for arg in args {
        let mapping: Mapping = arg.parse()?;
        mappings.push(mapping);
    }
    Ok(mappings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_valid() {
        assert_eq!("8080".parse::<Port>().unwrap().get(), 8080);
        assert_eq!("22".parse::<Port>().unwrap().get(), 22);
        assert_eq!("1".parse::<Port>().unwrap().get(), 1);
        assert_eq!("65535".parse::<Port>().unwrap().get(), 65535);
    }

    #[test]
    fn port_invalid() {
        assert!("0".parse::<Port>().is_err());
        assert!("70000".parse::<Port>().is_err());
        assert!("abc".parse::<Port>().is_err());
        assert!("".parse::<Port>().is_err());
    }

    #[test]
    fn port_duplicate_detection() {
        let args: Vec<String> = vec!["80".into(), "443".into(), "80".into()];
        assert!(parse_ports(&args).is_err());
    }

    #[test]
    fn mapping_valid() {
        let m: Mapping = "4000:8080".parse().unwrap();
        assert_eq!(m.local, 4000);
        assert_eq!(m.remote, 8080);
    }

    #[test]
    fn mapping_invalid() {
        assert!("0:80".parse::<Mapping>().is_err());
        assert!("80".parse::<Mapping>().is_err());
        assert!("abc:80".parse::<Mapping>().is_err());
        assert!("80:0".parse::<Mapping>().is_err());
    }
}
