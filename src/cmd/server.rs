use crate::prelude::*;
use std::io::Read as _;

#[derive(serde::Deserialize, Debug, Default)]
pub struct Config {
    #[serde(default)]
    server: crate::config::Server,

    #[serde(
        rename = "oauth",
        deserialize_with = "crate::config::oauth_configs",
        default
    )]
    oauth_configs: std::collections::HashMap<
        crate::protocol::AuthType,
        crate::oauth::Config,
    >,
}

impl crate::config::Config for Config {
    fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        self.server.merge_args(matches)
    }

    fn run(
        &self,
    ) -> Box<dyn futures::future::Future<Item = (), Error = Error> + Send> {
        let (acceptor, server) =
            if let Some(tls_identity_file) = &self.server.tls_identity_file {
                match create_server_tls(
                    self.server.listen_address,
                    self.server.buffer_size,
                    self.server.read_timeout,
                    tls_identity_file,
                    self.server.allowed_login_methods.clone(),
                    self.oauth_configs.clone(),
                    self.server.uid,
                    self.server.gid,
                ) {
                    Ok(futs) => futs,
                    Err(e) => return Box::new(futures::future::err(e)),
                }
            } else {
                match create_server(
                    self.server.listen_address,
                    self.server.buffer_size,
                    self.server.read_timeout,
                    self.server.allowed_login_methods.clone(),
                    self.oauth_configs.clone(),
                    self.server.uid,
                    self.server.gid,
                ) {
                    Ok(futs) => futs,
                    Err(e) => return Box::new(futures::future::err(e)),
                }
            };
        Box::new(futures::future::lazy(move || {
            tokio::spawn(server.map_err(|e| {
                log::error!("{}", e);
            }));

            acceptor
        }))
    }
}

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    crate::config::Server::cmd(app.about("Run a teleterm server"))
}

pub fn config(
    config: Option<config::Config>,
) -> Result<Box<dyn crate::config::Config>> {
    let config: Config = if let Some(config) = config {
        config
            .try_into()
            .context(crate::error::CouldntParseConfig)?
    } else {
        Config::default()
    };
    Ok(Box::new(config))
}

fn create_server(
    address: std::net::SocketAddr,
    buffer_size: usize,
    read_timeout: std::time::Duration,
    allowed_login_methods: std::collections::HashSet<
        crate::protocol::AuthType,
    >,
    oauth_configs: std::collections::HashMap<
        crate::protocol::AuthType,
        crate::oauth::Config,
    >,
    uid: Option<users::uid_t>,
    gid: Option<users::gid_t>,
) -> Result<(
    Box<dyn futures::future::Future<Item = (), Error = Error> + Send>,
    Box<dyn futures::future::Future<Item = (), Error = Error> + Send>,
)> {
    let (mut sock_w, sock_r) = tokio::sync::mpsc::channel(100);
    let listener = tokio::net::TcpListener::bind(&address)
        .context(crate::error::Bind { address })?;
    drop_privs(uid, gid)?;
    log::info!("Listening on {}", address);
    let acceptor = listener
        .incoming()
        .context(crate::error::Acceptor)
        .for_each(move |sock| {
            sock_w
                .try_send(sock)
                .context(crate::error::SendSocketChannel)
        });
    let server = crate::server::Server::new(
        buffer_size,
        read_timeout,
        sock_r,
        allowed_login_methods,
        oauth_configs,
    );
    Ok((Box::new(acceptor), Box::new(server)))
}

fn create_server_tls(
    address: std::net::SocketAddr,
    buffer_size: usize,
    read_timeout: std::time::Duration,
    tls_identity_file: &str,
    allowed_login_methods: std::collections::HashSet<
        crate::protocol::AuthType,
    >,
    oauth_configs: std::collections::HashMap<
        crate::protocol::AuthType,
        crate::oauth::Config,
    >,
    uid: Option<users::uid_t>,
    gid: Option<users::gid_t>,
) -> Result<(
    Box<dyn futures::future::Future<Item = (), Error = Error> + Send>,
    Box<dyn futures::future::Future<Item = (), Error = Error> + Send>,
)> {
    let (mut sock_w, sock_r) = tokio::sync::mpsc::channel(100);
    let listener = tokio::net::TcpListener::bind(&address)
        .context(crate::error::Bind { address })?;
    drop_privs(uid, gid)?;
    log::info!("Listening on {}", address);

    let mut file = std::fs::File::open(tls_identity_file).context(
        crate::error::OpenFileSync {
            filename: tls_identity_file,
        },
    )?;
    let mut identity = vec![];
    file.read_to_end(&mut identity)
        .context(crate::error::ReadFileSync)?;
    let identity = native_tls::Identity::from_pkcs12(&identity, "")
        .context(crate::error::ParseIdentity)?;
    let acceptor = native_tls::TlsAcceptor::new(identity)
        .context(crate::error::CreateAcceptor)?;
    let acceptor = tokio_tls::TlsAcceptor::from(acceptor);

    let acceptor = listener
        .incoming()
        .context(crate::error::Acceptor)
        .for_each(move |sock| {
            let sock = acceptor.accept(sock);
            sock_w
                .try_send(sock)
                .map_err(|_| Error::SendSocketChannelTls {})
        });
    let server = crate::server::tls::Server::new(
        buffer_size,
        read_timeout,
        sock_r,
        allowed_login_methods,
        oauth_configs,
    );
    Ok((Box::new(acceptor), Box::new(server)))
}

fn drop_privs(
    uid: Option<users::uid_t>,
    gid: Option<users::gid_t>,
) -> Result<()> {
    if let Some(gid) = gid {
        users::switch::set_effective_gid(gid)
            .context(crate::error::SwitchGid)?;
    }
    if let Some(uid) = uid {
        users::switch::set_effective_uid(uid)
            .context(crate::error::SwitchUid)?;
    }
    Ok(())
}
