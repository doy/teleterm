use crate::prelude::*;
use std::convert::TryFrom as _;
use std::io::Read as _;

#[derive(serde::Deserialize)]
pub struct Config {
    #[serde(
        deserialize_with = "crate::config::listen_address",
        default = "crate::config::default_listen_address"
    )]
    address: std::net::SocketAddr,

    #[serde(default = "crate::config::default_connection_buffer_size")]
    buffer_size: usize,

    #[serde(default = "crate::config::default_read_timeout")]
    read_timeout: std::time::Duration,

    tls_identity_file: Option<String>,

    #[serde(
        deserialize_with = "crate::config::allowed_login_methods",
        default = "crate::config::default_allowed_login_methods"
    )]
    allowed_login_methods:
        std::collections::HashSet<crate::protocol::AuthType>,
}

impl crate::config::Config for Config {
    fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        if matches.is_present("address") {
            self.address = matches
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

    fn run(&self) -> Result<()> {
        let (acceptor, server) =
            if let Some(tls_identity_file) = &self.tls_identity_file {
                create_server_tls(
                    self.address,
                    self.buffer_size,
                    self.read_timeout,
                    tls_identity_file,
                    self.allowed_login_methods.clone(),
                )?
            } else {
                create_server(
                    self.address,
                    self.buffer_size,
                    self.read_timeout,
                    self.allowed_login_methods.clone(),
                )?
            };
        tokio::run(futures::future::lazy(move || {
            tokio::spawn(server.map_err(|e| {
                eprintln!("{}", e);
            }));

            acceptor.map_err(|e| {
                eprintln!("{}", e);
            })
        }));
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            address: crate::config::default_listen_address(),
            buffer_size: crate::config::default_connection_buffer_size(),
            read_timeout: crate::config::default_read_timeout(),
            tls_identity_file: None,
            allowed_login_methods:
                crate::config::default_allowed_login_methods(),
        }
    }
}

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Run a teleterm server")
        .arg(
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

pub fn config() -> Box<dyn crate::config::Config> {
    Box::new(Config::default())
}

fn create_server(
    address: std::net::SocketAddr,
    buffer_size: usize,
    read_timeout: std::time::Duration,
    allowed_login_methods: std::collections::HashSet<
        crate::protocol::AuthType,
    >,
) -> Result<(
    Box<dyn futures::future::Future<Item = (), Error = Error> + Send>,
    Box<dyn futures::future::Future<Item = (), Error = Error> + Send>,
)> {
    let (mut sock_w, sock_r) = tokio::sync::mpsc::channel(100);
    let listener = tokio::net::TcpListener::bind(&address)
        .context(crate::error::Bind)?;
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
) -> Result<(
    Box<dyn futures::future::Future<Item = (), Error = Error> + Send>,
    Box<dyn futures::future::Future<Item = (), Error = Error> + Send>,
)> {
    let (mut sock_w, sock_r) = tokio::sync::mpsc::channel(100);
    let listener = tokio::net::TcpListener::bind(&address)
        .context(crate::error::Bind)?;

    let mut file = std::fs::File::open(tls_identity_file)
        .context(crate::error::OpenIdentityFile)?;
    let mut identity = vec![];
    file.read_to_end(&mut identity)
        .context(crate::error::ReadIdentityFile)?;
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
    );
    Ok((Box::new(acceptor), Box::new(server)))
}
