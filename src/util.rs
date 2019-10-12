use snafu::{OptionExt as _, ResultExt as _};
use std::net::ToSocketAddrs as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("couldn't find name in argv"))]
    MissingArgv,

    #[snafu(display(
        "detected argv path was not a valid filename: {}",
        path
    ))]
    NotAFileName { path: String },

    #[snafu(display("failed to parse address"))]
    ParseAddress,

    #[snafu(display("failed to parse address: {}", source))]
    ParsePort { source: std::num::ParseIntError },

    #[snafu(display("failed to resolve address: {}", source))]
    ResolveAddress { source: std::io::Error },

    #[snafu(display("failed to find any resolved addresses"))]
    HasResolvedAddr,
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn program_name() -> Result<String> {
    let program = std::env::args().next().context(MissingArgv)?;
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
    let host = address_parts.next().context(ParseAddress)?;
    let port = address_parts.next().context(ParseAddress)?;
    let port: u16 = port.parse().context(ParsePort)?;
    let socket_addr = (host, port)
        .to_socket_addrs()
        .context(ResolveAddress)?
        .next()
        .context(HasResolvedAddr)?;
    Ok((host.to_string(), socket_addr))
}
