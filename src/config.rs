use crate::prelude::*;
use serde::de::Deserialize as _;
use std::convert::TryFrom as _;
use std::net::ToSocketAddrs as _;

const CONFIG_FILENAME: &str = "config.toml";
const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:4144";
const DEFAULT_CONNECT_ADDRESS: &str = "127.0.0.1:4144";
const DEFAULT_BUFFER_SIZE: usize = 4 * 1024 * 1024;
const DEFAULT_READ_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(120);
const DEFAULT_AUTH_TYPE: crate::protocol::AuthType =
    crate::protocol::AuthType::Plain;
const DEFAULT_TLS: bool = false;
const DEFAULT_TTYREC_FILENAME: &str = "teleterm.ttyrec";

pub trait Config: std::fmt::Debug {
    fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()>;
    fn run(&self) -> Result<()>;
}

pub fn config() -> config::Config {
    let config_filename =
        crate::dirs::Dirs::new().config_file(CONFIG_FILENAME);

    let mut config = config::Config::default();
    if let Err(e) = config.merge(config::File::from(config_filename.clone()))
    {
        log::warn!(
            "failed to read config file {}: {}",
            config_filename.to_string_lossy(),
            e
        );
        // if merge returns an error, the config source will still have been
        // added to the config object, so the config object will likely never
        // work, so we should recreate it from scratch.
        config = config::Config::default();
    }

    config
}

#[derive(serde::Deserialize, Debug)]
pub struct Client {
    #[serde(deserialize_with = "auth_type", default = "default_auth_type")]
    pub auth: crate::protocol::AuthType,

    #[serde(default = "default_username")]
    pub username: Option<String>,

    #[serde(
        deserialize_with = "connect_address",
        default = "default_connect_address"
    )]
    pub connect_address: (String, std::net::SocketAddr),

    #[serde(default = "default_tls")]
    pub tls: bool,
}

impl Client {
    pub fn host(&self) -> &str {
        &self.connect_address.0
    }

    pub fn addr(&self) -> &std::net::SocketAddr {
        &self.connect_address.1
    }

    pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
        app.arg(
            clap::Arg::with_name("login-plain")
                .long("login-plain")
                .takes_value(true),
        )
        .arg(
            clap::Arg::with_name("login-recurse-center")
                .long("login-recurse-center")
                .conflicts_with("login-plain"),
        )
        .arg(
            clap::Arg::with_name("address")
                .long("address")
                .takes_value(true),
        )
        .arg(clap::Arg::with_name("tls").long("tls"))
    }

    pub fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        if matches.is_present("login-recurse-center") {
            self.auth = crate::protocol::AuthType::RecurseCenter;
        }
        if matches.is_present("login-plain") {
            let username = matches
                .value_of("login-plain")
                .map(std::string::ToString::to_string);
            self.auth = crate::protocol::AuthType::Plain;
            self.username = username;
        }
        if matches.is_present("address") {
            let address = matches.value_of("address").unwrap();
            self.connect_address = to_connect_address(address)?;
        }
        if matches.is_present("tls") {
            self.tls = true;
        }
        Ok(())
    }
}

impl Default for Client {
    fn default() -> Self {
        Self {
            auth: default_auth_type(),
            username: default_username(),
            connect_address: default_connect_address(),
            tls: default_tls(),
        }
    }
}

fn auth_type<'a, D>(
    deserializer: D,
) -> std::result::Result<crate::protocol::AuthType, D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    crate::protocol::AuthType::try_from(
        <String>::deserialize(deserializer)?.as_ref(),
    )
    .map_err(serde::de::Error::custom)
}

fn default_auth_type() -> crate::protocol::AuthType {
    DEFAULT_AUTH_TYPE
}

fn default_username() -> Option<String> {
    std::env::var("USER").ok()
}

fn connect_address<'a, D>(
    deserializer: D,
) -> std::result::Result<(String, std::net::SocketAddr), D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    to_connect_address(&<String>::deserialize(deserializer)?)
        .map_err(serde::de::Error::custom)
}

fn default_connect_address() -> (String, std::net::SocketAddr) {
    to_connect_address(DEFAULT_CONNECT_ADDRESS).unwrap()
}

// XXX this does a blocking dns lookup - should try to find an async version
fn to_connect_address(
    address: &str,
) -> Result<(String, std::net::SocketAddr)> {
    let mut address_parts = address.split(':');
    let host = address_parts.next().context(crate::error::ParseAddress)?;
    let port_str =
        address_parts.next().context(crate::error::ParseAddress)?;
    let port: u16 = port_str
        .parse()
        .context(crate::error::ParsePort { string: port_str })?;
    let socket_addr = (host, port)
        .to_socket_addrs()
        .context(crate::error::ResolveAddress { host, port })?
        .next()
        .context(crate::error::HasResolvedAddr)?;
    Ok((host.to_string(), socket_addr))
}

fn default_tls() -> bool {
    DEFAULT_TLS
}

#[derive(serde::Deserialize, Debug)]
pub struct Server {
    #[serde(
        deserialize_with = "listen_address",
        default = "default_listen_address"
    )]
    pub listen_address: std::net::SocketAddr,

    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,

    #[serde(
        rename = "read_timeout_secs",
        deserialize_with = "read_timeout",
        default = "default_read_timeout"
    )]
    pub read_timeout: std::time::Duration,

    pub tls_identity_file: Option<String>,

    #[serde(
        deserialize_with = "allowed_login_methods",
        default = "default_allowed_login_methods"
    )]
    pub allowed_login_methods:
        std::collections::HashSet<crate::protocol::AuthType>,
}

impl Server {
    pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
        app.arg(
            clap::Arg::with_name("address")
                .long("address")
                .takes_value(true),
        )
        .arg(
            clap::Arg::with_name("buffer-size")
                .long("buffer-size")
                .takes_value(true),
        )
        .arg(
            clap::Arg::with_name("read-timeout")
                .long("read-timeout")
                .takes_value(true),
        )
        .arg(
            clap::Arg::with_name("tls-identity-file")
                .long("tls-identity-file")
                .takes_value(true),
        )
        .arg(
            clap::Arg::with_name("allowed-login-methods")
                .long("allowed-login-methods")
                .use_delimiter(true)
                .takes_value(true),
        )
    }

    pub fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        if matches.is_present("address") {
            self.listen_address = matches
                .value_of("address")
                .unwrap()
                .parse()
                .context(crate::error::ParseAddr)?;
        }
        if matches.is_present("buffer-size") {
            let s = matches.value_of("buffer-size").unwrap();
            self.buffer_size = s
                .parse()
                .context(crate::error::ParseBufferSize { input: s })?;
        }
        if matches.is_present("read-timeout") {
            let s = matches.value_of("read-timeout").unwrap();
            self.read_timeout = s
                .parse()
                .map(std::time::Duration::from_secs)
                .context(crate::error::ParseReadTimeout { input: s })?;
        }
        if matches.is_present("tls-identity-file") {
            self.tls_identity_file = Some(
                matches.value_of("tls-identity-file").unwrap().to_string(),
            );
        }
        if matches.is_present("allowed-login-methods") {
            self.allowed_login_methods = matches
                .values_of("allowed-login-methods")
                .unwrap()
                .map(crate::protocol::AuthType::try_from)
                .collect::<Result<
                    std::collections::HashSet<crate::protocol::AuthType>,
                >>()?;
        }
        Ok(())
    }
}

impl Default for Server {
    fn default() -> Self {
        Self {
            listen_address: default_listen_address(),
            buffer_size: default_buffer_size(),
            read_timeout: default_read_timeout(),
            tls_identity_file: None,
            allowed_login_methods: default_allowed_login_methods(),
        }
    }
}

fn listen_address<'a, D>(
    deserializer: D,
) -> std::result::Result<std::net::SocketAddr, D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    to_listen_address(&<String>::deserialize(deserializer)?)
        .map_err(serde::de::Error::custom)
}

fn default_listen_address() -> std::net::SocketAddr {
    to_listen_address(DEFAULT_LISTEN_ADDRESS).unwrap()
}

fn to_listen_address(address: &str) -> Result<std::net::SocketAddr> {
    address.parse().context(crate::error::ParseAddr)
}

fn default_buffer_size() -> usize {
    DEFAULT_BUFFER_SIZE
}

fn read_timeout<'a, D>(
    deserializer: D,
) -> std::result::Result<std::time::Duration, D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    Ok(std::time::Duration::from_secs(u64::deserialize(
        deserializer,
    )?))
}

fn default_read_timeout() -> std::time::Duration {
    DEFAULT_READ_TIMEOUT
}

fn allowed_login_methods<'a, D>(
    deserializer: D,
) -> std::result::Result<
    std::collections::HashSet<crate::protocol::AuthType>,
    D::Error,
>
where
    D: serde::de::Deserializer<'a>,
{
    struct StringOrVec;

    impl<'a> serde::de::Visitor<'a> for StringOrVec {
        type Value = Vec<String>;

        fn expecting(
            &self,
            formatter: &mut std::fmt::Formatter,
        ) -> std::fmt::Result {
            formatter.write_str("string or list")
        }

        fn visit_str<E>(
            self,
            value: &str,
        ) -> std::result::Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(value
                .split(',')
                .map(std::string::ToString::to_string)
                .collect())
        }

        fn visit_seq<A>(
            self,
            seq: A,
        ) -> std::result::Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'a>,
        {
            serde::de::Deserialize::deserialize(
                serde::de::value::SeqAccessDeserializer::new(seq),
            )
        }
    }

    deserializer
        .deserialize_any(StringOrVec)?
        .iter()
        .map(|s| {
            crate::protocol::AuthType::try_from(s.as_str())
                .map_err(serde::de::Error::custom)
        })
        .collect()
}

fn default_allowed_login_methods(
) -> std::collections::HashSet<crate::protocol::AuthType> {
    crate::protocol::AuthType::iter().collect()
}

#[derive(serde::Deserialize, Debug)]
pub struct Command {
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,

    #[serde(skip, default = "default_command")]
    pub command: String,

    #[serde(skip, default = "default_args")]
    pub args: Vec<String>,
}

impl Command {
    pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
        app.arg(
            clap::Arg::with_name("buffer-size")
                .long("buffer-size")
                .takes_value(true),
        )
        .arg(clap::Arg::with_name("command").index(1))
        .arg(clap::Arg::with_name("args").index(2).multiple(true))
    }
    pub fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        if matches.is_present("buffer-size") {
            let buffer_size = matches.value_of("buffer-size").unwrap();
            self.buffer_size = buffer_size.parse().context(
                crate::error::ParseBufferSize { input: buffer_size },
            )?;
        }
        if matches.is_present("command") {
            self.command = matches.value_of("command").unwrap().to_string();
        }
        if matches.is_present("args") {
            self.args = matches
                .values_of("args")
                .unwrap()
                .map(std::string::ToString::to_string)
                .collect();
        }
        Ok(())
    }
}

impl Default for Command {
    fn default() -> Self {
        Self {
            buffer_size: default_buffer_size(),
            command: default_command(),
            args: default_args(),
        }
    }
}

fn default_command() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
}

fn default_args() -> Vec<String> {
    vec![]
}

#[derive(serde::Deserialize, Debug)]
pub struct Ttyrec {
    #[serde(default = "default_ttyrec_filename")]
    pub filename: String,
}

impl Ttyrec {
    pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
        app.arg(
            clap::Arg::with_name("filename")
                .long("filename")
                .takes_value(true),
        )
    }

    pub fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        if matches.is_present("filename") {
            self.filename = matches.value_of("filename").unwrap().to_string();
        }
        Ok(())
    }
}

impl Default for Ttyrec {
    fn default() -> Self {
        Self {
            filename: default_ttyrec_filename(),
        }
    }
}

fn default_ttyrec_filename() -> String {
    DEFAULT_TTYREC_FILENAME.to_string()
}
