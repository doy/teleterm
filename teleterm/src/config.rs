use crate::prelude::*;
use serde::de::Deserialize as _;
use std::convert::TryFrom as _;
use std::net::ToSocketAddrs as _;

pub mod wizard;

const CONFIG_FILENAME: &str = "config.toml";

const ALLOWED_LOGIN_METHODS_OPTION: &str = "allowed-login-methods";
const ARGS_OPTION: &str = "args";
const COMMAND_OPTION: &str = "command";
const CONNECT_ADDRESS_OPTION: &str = "connect-address";
const FILENAME_OPTION: &str = "filename";
const LISTEN_ADDRESS_OPTION: &str = "listen-address";
const LOGIN_PLAIN_OPTION: &str = "login-plain";
const LOGIN_RECURSE_CENTER_OPTION: &str = "login-recurse-center";
const MAX_FRAME_LENGTH_OPTION: &str = "max-frame-length";
const PLAY_AT_START_OPTION: &str = "play-at-start";
const PLAYBACK_RATIO_OPTION: &str = "playback-ratio";
const READ_TIMEOUT_OPTION: &str = "read-timeout-secs";
const TLS_IDENTITY_FILE_OPTION: &str = "tls-identity-file";
const TLS_OPTION: &str = "tls";

const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:4144";
const DEFAULT_CONNECT_ADDRESS: &str = "127.0.0.1:4144";
const DEFAULT_WEB_LISTEN_ADDRESS: &str = "127.0.0.1:4145";
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
    fn run(
        &self,
    ) -> Box<dyn futures::Future<Item = (), Error = Error> + Send>;
}

pub fn config(
    filename: Option<&std::path::Path>,
) -> Result<Option<config::Config>> {
    let config_filename = if let Some(filename) = filename {
        if !filename.exists() {
            return Err(Error::ConfigFileDoesntExist {
                name: filename.to_string_lossy().to_string(),
            });
        }
        Some(filename.to_path_buf())
    } else {
        crate::dirs::Dirs::new().config_file(CONFIG_FILENAME, true)
    };
    config_filename
        .map(|config_filename| config_from_filename(&config_filename))
        .transpose()
}

fn config_from_filename(
    filename: &std::path::Path,
) -> Result<config::Config> {
    let mut config = config::Config::default();
    config
        .merge(config::File::from(filename))
        .context(crate::error::ParseConfigFile)?;
    Ok(config)
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
        let login_plain_help = "Use the 'plain' authentication method (default), with username USERNAME (defaults to $USER)";
        let login_recurse_center_help =
            "Use the 'recurse_center' authentication method";
        let connect_address_help =
            "Host and port to connect to (defaults to localhost:4144)";
        let tls_help = "Connect to the server using TLS";

        app.arg(
            clap::Arg::with_name(LOGIN_PLAIN_OPTION)
                .long(LOGIN_PLAIN_OPTION)
                .takes_value(true)
                .value_name("USERNAME")
                .help(login_plain_help),
        )
        .arg(
            clap::Arg::with_name(LOGIN_RECURSE_CENTER_OPTION)
                .long(LOGIN_RECURSE_CENTER_OPTION)
                .conflicts_with(LOGIN_PLAIN_OPTION)
                .help(login_recurse_center_help),
        )
        .arg(
            clap::Arg::with_name(CONNECT_ADDRESS_OPTION)
                .long(CONNECT_ADDRESS_OPTION)
                .takes_value(true)
                .value_name("HOST:PORT")
                .help(connect_address_help),
        )
        .arg(
            clap::Arg::with_name(TLS_OPTION)
                .long(TLS_OPTION)
                .help(tls_help),
        )
    }

    pub fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        if matches.is_present(LOGIN_RECURSE_CENTER_OPTION) {
            self.auth = crate::protocol::AuthType::RecurseCenter;
        }
        if matches.is_present(LOGIN_PLAIN_OPTION) {
            let username = matches
                .value_of(LOGIN_PLAIN_OPTION)
                .map(std::string::ToString::to_string);
            self.auth = crate::protocol::AuthType::Plain;
            self.username = username;
        }
        if matches.is_present(CONNECT_ADDRESS_OPTION) {
            let address = matches.value_of(CONNECT_ADDRESS_OPTION).unwrap();
            self.connect_address = to_connect_address(address)?;
        }
        if matches.is_present(TLS_OPTION) {
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
// XXX shouldn't need to be pub
pub fn to_connect_address(
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

    #[serde(deserialize_with = "uid", default)]
    pub uid: Option<users::uid_t>,

    #[serde(deserialize_with = "gid", default)]
    pub gid: Option<users::gid_t>,
}

impl Server {
    pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
        let listen_address_help =
            "Host and port to listen on (defaults to localhost:4144)";
        let read_timeout_help = "Number of idle seconds to wait before disconnecting a client (defaults to 30)";
        let tls_identity_file_help = "File containing the TLS certificate and private key to use for accepting TLS connections. Must be in pfx format. The server will only allow connections over TLS if this option is set.";
        let allowed_login_methods_help = "Comma separated list containing the auth methods this server should allow. Allows everything by default, valid values are plain, recurse_center";
        app.arg(
            clap::Arg::with_name(LISTEN_ADDRESS_OPTION)
                .long(LISTEN_ADDRESS_OPTION)
                .takes_value(true)
                .value_name("HOST:PORT")
                .help(listen_address_help),
        )
        .arg(
            clap::Arg::with_name(READ_TIMEOUT_OPTION)
                .long(READ_TIMEOUT_OPTION)
                .takes_value(true)
                .value_name("SECS")
                .help(read_timeout_help),
        )
        .arg(
            clap::Arg::with_name(TLS_IDENTITY_FILE_OPTION)
                .long(TLS_IDENTITY_FILE_OPTION)
                .takes_value(true)
                .value_name("FILE")
                .help(tls_identity_file_help),
        )
        .arg(
            clap::Arg::with_name(ALLOWED_LOGIN_METHODS_OPTION)
                .long(ALLOWED_LOGIN_METHODS_OPTION)
                .use_delimiter(true)
                .takes_value(true)
                .value_name("AUTH_METHODS")
                .help(allowed_login_methods_help),
        )
    }

    pub fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        if matches.is_present(LISTEN_ADDRESS_OPTION) {
            self.listen_address = matches
                .value_of(LISTEN_ADDRESS_OPTION)
                .unwrap()
                .parse()
                .context(crate::error::ParseAddr)?;
        }
        if matches.is_present(READ_TIMEOUT_OPTION) {
            let s = matches.value_of(READ_TIMEOUT_OPTION).unwrap();
            self.read_timeout = s
                .parse()
                .map(std::time::Duration::from_secs)
                .context(crate::error::ParseReadTimeout { input: s })?;
        }
        if matches.is_present(TLS_IDENTITY_FILE_OPTION) {
            self.tls_identity_file = Some(
                matches
                    .value_of(TLS_IDENTITY_FILE_OPTION)
                    .unwrap()
                    .to_string(),
            );
        }
        if matches.is_present(ALLOWED_LOGIN_METHODS_OPTION) {
            self.allowed_login_methods = matches
                .values_of(ALLOWED_LOGIN_METHODS_OPTION)
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
            read_timeout: default_read_timeout(),
            tls_identity_file: None,
            allowed_login_methods: default_allowed_login_methods(),
            uid: None,
            gid: None,
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

fn uid<'a, D>(
    deserializer: D,
) -> std::result::Result<Option<users::uid_t>, D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    struct StringOrInt;

    impl<'a> serde::de::Visitor<'a> for StringOrInt {
        type Value = Option<u32>;

        fn expecting(
            &self,
            formatter: &mut std::fmt::Formatter,
        ) -> std::fmt::Result {
            formatter.write_str("string or int")
        }

        fn visit_str<E>(
            self,
            value: &str,
        ) -> std::result::Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(Some(
                users::get_user_by_name(value)
                    .context(crate::error::UnknownUser { name: value })
                    .map_err(serde::de::Error::custom)?
                    .uid(),
            ))
        }

        fn visit_u32<E>(
            self,
            value: u32,
        ) -> std::result::Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if users::get_user_by_uid(value).is_none() {
                return Err(serde::de::Error::custom(Error::UnknownUid {
                    uid: value,
                }));
            }
            Ok(Some(value))
        }
    }

    deserializer.deserialize_any(StringOrInt)
}

fn gid<'a, D>(
    deserializer: D,
) -> std::result::Result<Option<users::gid_t>, D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    struct StringOrInt;

    impl<'a> serde::de::Visitor<'a> for StringOrInt {
        type Value = Option<u32>;

        fn expecting(
            &self,
            formatter: &mut std::fmt::Formatter,
        ) -> std::fmt::Result {
            formatter.write_str("string or int")
        }

        fn visit_none<E>(self) -> std::result::Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_str<E>(
            self,
            value: &str,
        ) -> std::result::Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(Some(
                users::get_group_by_name(value)
                    .context(crate::error::UnknownGroup { name: value })
                    .map_err(serde::de::Error::custom)?
                    .gid(),
            ))
        }

        fn visit_u32<E>(
            self,
            value: u32,
        ) -> std::result::Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if users::get_group_by_gid(value).is_none() {
                return Err(serde::de::Error::custom(Error::UnknownGid {
                    gid: value,
                }));
            }
            Ok(Some(value))
        }
    }

    deserializer.deserialize_any(StringOrInt)
}

#[derive(serde::Deserialize, Debug)]
pub struct Web {
    #[serde(
        deserialize_with = "listen_address",
        default = "default_web_listen_address"
    )]
    pub listen_address: std::net::SocketAddr,
}

impl Web {
    pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
        let listen_address_help =
            "Host and port to listen on (defaults to localhost:4144)";
        app.arg(
            clap::Arg::with_name(LISTEN_ADDRESS_OPTION)
                .long(LISTEN_ADDRESS_OPTION)
                .takes_value(true)
                .value_name("HOST:PORT")
                .help(listen_address_help),
        )
    }
    pub fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        if matches.is_present(LISTEN_ADDRESS_OPTION) {
            self.listen_address = matches
                .value_of(LISTEN_ADDRESS_OPTION)
                .unwrap()
                .parse()
                .context(crate::error::ParseAddr)?;
        }
        Ok(())
    }
}

impl Default for Web {
    fn default() -> Self {
        Self {
            listen_address: default_web_listen_address(),
        }
    }
}

fn default_web_listen_address() -> std::net::SocketAddr {
    to_listen_address(DEFAULT_WEB_LISTEN_ADDRESS).unwrap()
}

#[derive(serde::Deserialize, Debug)]
pub struct Command {
    #[serde(default = "default_command")]
    pub command: String,

    #[serde(default = "default_args")]
    pub args: Vec<String>,
}

impl Command {
    pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
        let command_help = "Command to run";
        let args_help = "Arguments for the command";

        app.arg(
            clap::Arg::with_name(COMMAND_OPTION)
                .index(1)
                .help(command_help),
        )
        .arg(
            clap::Arg::with_name(ARGS_OPTION)
                .index(2)
                .multiple(true)
                .help(args_help),
        )
    }
    pub fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        if matches.is_present(COMMAND_OPTION) {
            self.command =
                matches.value_of(COMMAND_OPTION).unwrap().to_string();
        }
        if matches.is_present(ARGS_OPTION) {
            self.args = matches
                .values_of(ARGS_OPTION)
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
        let filename_help =
            "TTYrec file to use (defaults to teleterm.ttyrec)";
        app.arg(
            clap::Arg::with_name(FILENAME_OPTION)
                .long(FILENAME_OPTION)
                .takes_value(true)
                .value_name("FILE")
                .help(filename_help),
        )
    }

    pub fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        if matches.is_present(FILENAME_OPTION) {
            self.filename =
                matches.value_of(FILENAME_OPTION).unwrap().to_string();
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

#[derive(serde::Deserialize, Debug)]
pub struct Play {
    #[serde(default)]
    pub play_at_start: bool,

    #[serde(default = "default_playback_ratio")]
    pub playback_ratio: f32,

    #[serde(default, deserialize_with = "max_frame_length")]
    pub max_frame_length: Option<std::time::Duration>,
}

impl Play {
    pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
        let play_at_start_help = "Start the player unpaused";
        let playback_ratio_help =
            "Speed to play back the ttyrec at (defaults to 1.0)";
        let max_frame_length_help =
            "Clamp frame duration at this number of seconds";
        app.arg(
            clap::Arg::with_name(PLAY_AT_START_OPTION)
                .long(PLAY_AT_START_OPTION)
                .help(play_at_start_help),
        )
        .arg(
            clap::Arg::with_name(PLAYBACK_RATIO_OPTION)
                .long(PLAYBACK_RATIO_OPTION)
                .takes_value(true)
                .value_name("RATIO")
                .help(playback_ratio_help),
        )
        .arg(
            clap::Arg::with_name(MAX_FRAME_LENGTH_OPTION)
                .long(MAX_FRAME_LENGTH_OPTION)
                .takes_value(true)
                .value_name("SECS")
                .help(max_frame_length_help),
        )
    }

    pub fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        self.play_at_start = matches.is_present(PLAY_AT_START_OPTION);
        if matches.is_present(PLAYBACK_RATIO_OPTION) {
            self.playback_ratio = matches
                .value_of(PLAYBACK_RATIO_OPTION)
                .unwrap()
                .to_string()
                .parse()
                .context(crate::error::ParseFloat {
                    name: PLAYBACK_RATIO_OPTION,
                })?;
        }
        self.max_frame_length = matches
            .value_of(MAX_FRAME_LENGTH_OPTION)
            .map(|len| len.parse().map(std::time::Duration::from_secs))
            .transpose()
            .context(crate::error::ParseMaxFrameLength)?;
        Ok(())
    }
}

impl Default for Play {
    fn default() -> Self {
        Self {
            play_at_start: false,
            playback_ratio: default_playback_ratio(),
            max_frame_length: None,
        }
    }
}

fn default_playback_ratio() -> f32 {
    1.0
}

fn max_frame_length<'a, D>(
    deserializer: D,
) -> std::result::Result<Option<std::time::Duration>, D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    Ok(Some(std::time::Duration::from_secs(u64::deserialize(
        deserializer,
    )?)))
}

pub fn oauth_configs<'a, D>(
    deserializer: D,
) -> std::result::Result<
    std::collections::HashMap<
        crate::protocol::AuthType,
        crate::oauth::Config,
    >,
    D::Error,
>
where
    D: serde::de::Deserializer<'a>,
{
    let configs =
        <std::collections::HashMap<String, OauthConfig>>::deserialize(
            deserializer,
        )?;
    let mut ret = std::collections::HashMap::new();
    for (key, config) in configs {
        let auth_type = crate::protocol::AuthType::try_from(key.as_str())
            .map_err(serde::de::Error::custom)?;
        let real_config = match auth_type {
            crate::protocol::AuthType::RecurseCenter => {
                let client_id = config
                    .client_id
                    .context(crate::error::OauthMissingConfiguration {
                        field: "client_id",
                        auth_type,
                    })
                    .map_err(serde::de::Error::custom)?;
                let client_secret = config
                    .client_secret
                    .context(crate::error::OauthMissingConfiguration {
                        field: "client_secret",
                        auth_type,
                    })
                    .map_err(serde::de::Error::custom)?;
                crate::oauth::RecurseCenter::config(
                    &client_id,
                    &client_secret,
                )
            }
            ty if !ty.is_oauth() => {
                return Err(Error::AuthTypeNotOauth { ty: auth_type })
                    .map_err(serde::de::Error::custom);
            }
            _ => unreachable!(),
        };
        ret.insert(auth_type, real_config);
    }
    Ok(ret)
}

#[derive(serde::Deserialize, Debug)]
struct OauthConfig {
    #[serde(default)]
    client_id: Option<String>,

    #[serde(default)]
    client_secret: Option<String>,

    #[serde(deserialize_with = "url", default)]
    auth_url: Option<url::Url>,

    #[serde(deserialize_with = "url", default)]
    token_url: Option<url::Url>,

    #[serde(deserialize_with = "url", default)]
    redirect_url: Option<url::Url>,
}

fn url<'a, D>(
    deserializer: D,
) -> std::result::Result<Option<url::Url>, D::Error>
where
    D: serde::de::Deserializer<'a>,
{
    Ok(<Option<String>>::deserialize(deserializer)?
        .map(|s| url::Url::parse(&s))
        .transpose()
        .map_err(serde::de::Error::custom)?)
}
