use crate::prelude::*;
use serde::de::Deserialize as _;
use std::convert::TryFrom as _;
use std::net::ToSocketAddrs as _;

const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:4144";
const DEFAULT_CONNECT_ADDRESS: &str = "127.0.0.1:4144";
const DEFAULT_CONNECTION_BUFFER_SIZE: usize = 4 * 1024 * 1024;
const DEFAULT_READ_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(120);
const DEFAULT_AUTH_TYPE: crate::protocol::AuthType =
    crate::protocol::AuthType::Plain;
const DEFAULT_TLS: bool = false;
const DEFAULT_TTYREC_FILENAME: &str = "teleterm.ttyrec";

pub trait Config {
    fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()>;
    fn run(&self) -> Result<()>;
}

pub fn listen_address<'a, D>(
    deserializer: D,
) -> std::result::Result<std::net::SocketAddr, D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    to_listen_address(<&str>::deserialize(deserializer)?)
        .map_err(serde::de::Error::custom)
}

pub fn default_listen_address() -> std::net::SocketAddr {
    to_listen_address(DEFAULT_LISTEN_ADDRESS).unwrap()
}

pub fn to_listen_address(address: &str) -> Result<std::net::SocketAddr> {
    address.parse().context(crate::error::ParseAddr)
}

pub fn connect_address<'a, D>(
    deserializer: D,
) -> std::result::Result<(String, std::net::SocketAddr), D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    to_connect_address(<&str>::deserialize(deserializer)?)
        .map_err(serde::de::Error::custom)
}

pub fn default_connect_address() -> (String, std::net::SocketAddr) {
    to_connect_address(DEFAULT_CONNECT_ADDRESS).unwrap()
}

// XXX this does a blocking dns lookup - should try to find an async version
pub fn to_connect_address(
    address: &str,
) -> Result<(String, std::net::SocketAddr)> {
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

pub fn default_connection_buffer_size() -> usize {
    DEFAULT_CONNECTION_BUFFER_SIZE
}

pub fn default_read_timeout() -> std::time::Duration {
    DEFAULT_READ_TIMEOUT
}

pub fn default_tls() -> bool {
    DEFAULT_TLS
}

pub fn default_command() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
}

pub fn default_args() -> Vec<String> {
    vec![]
}

pub fn default_ttyrec_filename() -> String {
    DEFAULT_TTYREC_FILENAME.to_string()
}

pub fn default_allowed_login_methods(
) -> std::collections::HashSet<crate::protocol::AuthType> {
    crate::protocol::AuthType::iter().collect()
}

pub fn allowed_login_methods<'a, D>(
    deserializer: D,
) -> std::result::Result<
    std::collections::HashSet<crate::protocol::AuthType>,
    D::Error,
>
where
    D: serde::de::Deserializer<'a>,
{
    Option::<Vec<&str>>::deserialize(deserializer)?
        .map_or_else(
            || Ok(default_allowed_login_methods()),
            |methods| {
                methods
                    .iter()
                    .copied()
                    .map(crate::protocol::AuthType::try_from)
                    .collect()
            },
        )
        .map_err(serde::de::Error::custom)
}

pub fn auth<'a, D>(
    deserializer: D,
) -> std::result::Result<crate::protocol::Auth, D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    LoginType::deserialize(deserializer).and_then(|login_type| {
        match login_type.login_type {
            crate::protocol::AuthType::Plain => login_type
                .username
                .map(std::string::ToString::to_string)
                .or_else(default_username)
                .ok_or_else(|| Error::CouldntFindUsername)
                .map(|username| crate::protocol::Auth::Plain { username })
                .map_err(serde::de::Error::custom),
            crate::protocol::AuthType::RecurseCenter => {
                Ok(crate::protocol::Auth::RecurseCenter {
                    id: login_type.id.map(std::string::ToString::to_string),
                })
            }
        }
    })
}

pub fn default_auth() -> crate::protocol::Auth {
    let username = default_username()
        .ok_or_else(|| Error::CouldntFindUsername)
        .unwrap();
    crate::protocol::Auth::Plain { username }
}

#[derive(serde::Deserialize)]
struct LoginType<'a> {
    #[serde(deserialize_with = "auth_type", default = "default_auth_type")]
    login_type: crate::protocol::AuthType,
    username: Option<&'a str>,
    id: Option<&'a str>,
}

fn auth_type<'a, D>(
    deserializer: D,
) -> std::result::Result<crate::protocol::AuthType, D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    crate::protocol::AuthType::try_from(<&str>::deserialize(deserializer)?)
        .map_err(serde::de::Error::custom)
}

fn default_auth_type() -> crate::protocol::AuthType {
    DEFAULT_AUTH_TYPE
}

fn default_username() -> Option<String> {
    std::env::var("USER").ok()
}
