use crate::prelude::*;
use std::convert::TryFrom as _;
use std::io::Read as _;

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

pub fn run<'a>(matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    let address = matches.value_of("address").map_or_else(
        || Ok("0.0.0.0:4144".parse().unwrap()),
        |s| s.parse().context(crate::error::ParseAddr),
    )?;
    let buffer_size =
        matches
            .value_of("buffer-size")
            .map_or(Ok(4 * 1024 * 1024), |s| {
                s.parse()
                    .context(crate::error::ParseBufferSize { input: s })
            })?;
    let read_timeout = matches.value_of("read-timeout").map_or(
        Ok(std::time::Duration::from_secs(120)),
        |s| {
            s.parse()
                .map(std::time::Duration::from_secs)
                .context(crate::error::ParseReadTimeout { input: s })
        },
    )?;
    let tls_identity_file = matches.value_of("tls-identity-file");
    let allowed_login_methods =
        matches.values_of("allowed-login-methods").map_or_else(
            || Ok(crate::protocol::AuthType::iter().collect()),
            |methods| {
                methods.map(crate::protocol::AuthType::try_from).collect()
            },
        )?;
    run_impl(
        address,
        buffer_size,
        read_timeout,
        tls_identity_file,
        allowed_login_methods,
    )
}

fn run_impl(
    address: std::net::SocketAddr,
    buffer_size: usize,
    read_timeout: std::time::Duration,
    tls_identity_file: Option<&str>,
    allowed_login_methods: std::collections::HashSet<
        crate::protocol::AuthType,
    >,
) -> Result<()> {
    let (acceptor, server) =
        if let Some(tls_identity_file) = tls_identity_file {
            create_server_tls(
                address,
                buffer_size,
                read_timeout,
                tls_identity_file,
                allowed_login_methods,
            )?
        } else {
            create_server(
                address,
                buffer_size,
                read_timeout,
                allowed_login_methods,
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
