use crate::prelude::*;
use std::net::ToSocketAddrs as _;

pub fn program_name() -> Result<String> {
    let program =
        std::env::args().next().context(crate::error::MissingArgv)?;
    let path = std::path::Path::new(&program);
    let filename = path.file_name();
    Ok(filename
        .ok_or_else(|| Error::NotAFileName {
            path: path.to_string_lossy().to_string(),
        })?
        .to_string_lossy()
        .to_string())
}

pub fn resolve_address(
    address: Option<&str>,
) -> Result<(String, std::net::SocketAddr)> {
    let address = address.unwrap_or("0.0.0.0:4144");
    let mut address_parts = address.split(':');
    let host = address_parts.next().context(crate::error::ParseAddress)?;
    let port = address_parts.next().context(crate::error::ParseAddress)?;
    let port: u16 = port.parse().context(crate::error::ParsePort)?;
    let socket_addr = (host, port)
        .to_socket_addrs()
        .context(crate::error::ResolveAddress)?
        .next()
        .context(crate::error::HasResolvedAddr)?;
    Ok((host.to_string(), socket_addr))
}
