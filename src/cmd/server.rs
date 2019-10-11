use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::futures01::stream::StreamExt as _;
use snafu::futures01::FutureExt as _;
use snafu::ResultExt as _;
use std::io::Read as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("{}", source))]
    Common { source: crate::error::Error },

    #[snafu(display("failed to bind: {}", source))]
    Bind { source: tokio::io::Error },

    #[snafu(display(
        "failed to parse read timeout '{}': {}",
        input,
        source
    ))]
    ParseReadTimeout {
        input: String,
        source: std::num::ParseIntError,
    },

    #[snafu(display("failed to accept: {}", source))]
    Acceptor { source: tokio::io::Error },

    #[snafu(display(
        "failed to send accepted socket to server thread: {}",
        source
    ))]
    SocketChannel {
        source: tokio::sync::mpsc::error::TrySendError<tokio::net::TcpStream>,
    },

    #[snafu(display("failed to send accepted socket to server thread"))]
    TlsSocketChannel {
        // XXX tokio_tls::Accept doesn't implement Debug or Display
    // source: tokio::sync::mpsc::error::TrySendError<tokio_tls::Accept<tokio::net::TcpStream>>,
    },

    #[snafu(display("failed to run server: {}", source))]
    Server { source: crate::server::Error },

    #[snafu(display("failed to open identity file: {}", source))]
    OpenIdentityFile { source: std::io::Error },

    #[snafu(display("failed to read identity file: {}", source))]
    ReadIdentityFile { source: std::io::Error },

    #[snafu(display("failed to parse identity file: {}", source))]
    ParseIdentity { source: native_tls::Error },

    #[snafu(display("failed to create tls acceptor: {}", source))]
    CreateAcceptor { source: native_tls::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Run a shellshare server")
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
}

pub fn run<'a>(matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    let address = matches.value_of("address").map_or_else(
        || Ok("0.0.0.0:4144".parse().unwrap()),
        |s| {
            s.parse()
                .context(crate::error::ParseAddr)
                .context(Common)
                .context(super::Server)
        },
    )?;
    let buffer_size =
        matches
            .value_of("buffer-size")
            .map_or(Ok(4 * 1024 * 1024), |s| {
                s.parse()
                    .context(crate::error::ParseBufferSize { input: s })
                    .context(Common)
                    .context(super::Server)
            })?;
    let read_timeout = matches.value_of("read-timeout").map_or(
        Ok(std::time::Duration::from_secs(120)),
        |s| {
            s.parse()
                .map(std::time::Duration::from_secs)
                .context(ParseReadTimeout { input: s })
                .context(super::Server)
        },
    )?;
    let tls_identity_file = matches.value_of("tls-identity-file");
    run_impl(address, buffer_size, read_timeout, tls_identity_file)
        .context(super::Server)
}

fn run_impl(
    address: std::net::SocketAddr,
    buffer_size: usize,
    read_timeout: std::time::Duration,
    tls_identity_file: Option<&str>,
) -> Result<()> {
    let (acceptor, server) =
        if let Some(tls_identity_file) = tls_identity_file {
            create_server_tls(
                address,
                buffer_size,
                read_timeout,
                tls_identity_file,
            )?
        } else {
            create_server(address, buffer_size, read_timeout)?
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
) -> Result<(
    Box<dyn futures::future::Future<Item = (), Error = Error> + Send>,
    Box<dyn futures::future::Future<Item = (), Error = Error> + Send>,
)> {
    let (mut sock_w, sock_r) = tokio::sync::mpsc::channel(100);
    let listener = tokio::net::TcpListener::bind(&address).context(Bind)?;
    let acceptor = listener
        .incoming()
        .context(Acceptor)
        .for_each(move |sock| sock_w.try_send(sock).context(SocketChannel));
    let server =
        crate::server::Server::new(buffer_size, read_timeout, sock_r)
            .context(Server);
    Ok((Box::new(acceptor), Box::new(server)))
}

fn create_server_tls(
    address: std::net::SocketAddr,
    buffer_size: usize,
    read_timeout: std::time::Duration,
    tls_identity_file: &str,
) -> Result<(
    Box<dyn futures::future::Future<Item = (), Error = Error> + Send>,
    Box<dyn futures::future::Future<Item = (), Error = Error> + Send>,
)> {
    let (mut sock_w, sock_r) = tokio::sync::mpsc::channel(100);
    let listener = tokio::net::TcpListener::bind(&address).context(Bind)?;

    let mut file =
        std::fs::File::open(tls_identity_file).context(OpenIdentityFile)?;
    let mut identity = vec![];
    file.read_to_end(&mut identity).context(ReadIdentityFile)?;
    let identity = native_tls::Identity::from_pkcs12(&identity, "")
        .context(ParseIdentity)?;
    let acceptor =
        native_tls::TlsAcceptor::new(identity).context(CreateAcceptor)?;
    let acceptor = tokio_tls::TlsAcceptor::from(acceptor);

    let acceptor =
        listener.incoming().context(Acceptor).for_each(move |sock| {
            let sock = acceptor.accept(sock);
            sock_w
                .try_send(sock)
                .map_err(|_| Error::TlsSocketChannel {})
        });
    let server =
        crate::server::TlsServer::new(buffer_size, read_timeout, sock_r)
            .context(Server);
    Ok((Box::new(acceptor), Box::new(server)))
}
