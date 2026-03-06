use anyhow::{Context, Result, bail};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    Tcp,
    Udp,
}

impl Protocol {
    pub fn suffix(self) -> &'static str {
        match self {
            Protocol::Tcp => "tcp",
            Protocol::Udp => "udp",
        }
    }
}

impl FromStr for Protocol {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "tcp" => Ok(Protocol::Tcp),
            "udp" => Ok(Protocol::Udp),
            _ => bail!("protocol must be tcp or udp"),
        }
    }
}

/// A port exposed by `punch out`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortSpec {
    port: Port,
    pub protocol: Protocol,
}

impl PortSpec {
    pub fn port(self) -> u16 {
        self.port.get()
    }
}

impl FromStr for PortSpec {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let (port, protocol) = split_protocol_suffix(s)?;
        let port: Port = port.parse()?;
        Ok(PortSpec { port, protocol })
    }
}

/// A local:remote port mapping for `punch in`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Mapping {
    pub local: u16,
    pub remote: u16,
    pub protocol: Protocol,
}

impl FromStr for Mapping {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let (l, r) = s
            .split_once(':')
            .context("mapping must be <local>:<remote>")?;
        if l == "-" {
            bail!("stdio mappings are not supported yet");
        }

        let local: Port = l.parse().context("invalid local port")?;
        let (remote, protocol) = split_protocol_suffix(r)?;
        let remote: Port = remote.parse().context("invalid remote port")?;
        Ok(Mapping {
            local: local.get(),
            remote: remote.get(),
            protocol,
        })
    }
}

fn split_protocol_suffix(s: &str) -> Result<(&str, Protocol)> {
    match s.rsplit_once('/') {
        Some((value, protocol)) => Ok((value, protocol.parse().context("invalid protocol")?)),
        None => Ok((s, Protocol::Tcp)),
    }
}

pub fn parse_ports(args: &[String]) -> Result<Vec<PortSpec>> {
    let mut ports = Vec::with_capacity(args.len());
    for arg in args {
        let port: PortSpec = arg.parse()?;
        if ports.contains(&port) {
            bail!("duplicate port: {}/{}", port.port(), port.protocol.suffix());
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
    fn port_spec_valid() {
        let spec: PortSpec = "53/udp".parse().unwrap();
        assert_eq!(spec.port(), 53);
        assert_eq!(spec.protocol, Protocol::Udp);

        let spec: PortSpec = "80".parse().unwrap();
        assert_eq!(spec.port(), 80);
        assert_eq!(spec.protocol, Protocol::Tcp);
    }

    #[test]
    fn port_spec_duplicate_detection_is_per_protocol() {
        let args: Vec<String> = vec!["53/tcp".into(), "53/udp".into()];
        assert!(parse_ports(&args).is_ok());

        let args: Vec<String> = vec!["53/udp".into(), "53/udp".into()];
        assert!(parse_ports(&args).is_err());
    }

    #[test]
    fn mapping_valid() {
        let m: Mapping = "4000:8080".parse().unwrap();
        assert_eq!(m.local, 4000);
        assert_eq!(m.remote, 8080);
        assert_eq!(m.protocol, Protocol::Tcp);
    }

    #[test]
    fn mapping_udp_valid() {
        let m: Mapping = "5300:53/udp".parse().unwrap();
        assert_eq!(m.local, 5300);
        assert_eq!(m.remote, 53);
        assert_eq!(m.protocol, Protocol::Udp);
    }

    #[test]
    fn mapping_invalid() {
        assert!("0:80".parse::<Mapping>().is_err());
        assert!("80".parse::<Mapping>().is_err());
        assert!("abc:80".parse::<Mapping>().is_err());
        assert!("80:0".parse::<Mapping>().is_err());
        assert!("-:22".parse::<Mapping>().is_err());
        assert!("5300:53/sctp".parse::<Mapping>().is_err());
    }
}
