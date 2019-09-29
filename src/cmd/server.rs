use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::ResultExt as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to parse address: {}", source))]
    ParseAddress { source: std::net::AddrParseError },

    #[snafu(display("failed to bind: {}", source))]
    Bind { source: tokio::io::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Run a termcast server")
}

pub fn run<'a>(_matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    run_impl().context(super::Server)
}

fn run_impl() -> Result<()> {
    let (mut sock_w, sock_r) = tokio::sync::mpsc::channel(1);
    let addr = "127.0.0.1:8000".parse().context(ParseAddress)?;
    let listener = tokio::net::TcpListener::bind(&addr).context(Bind)?;
    let server = listener
        .incoming()
        .map_err(|e| {
            eprintln!("accept failed: {}", e);
        })
        .for_each(move |sock| {
            sock_w.try_send(sock).map_err(|e| {
                eprintln!("sending socket to manager thread failed: {}", e);
            })
        });

    tokio::run(futures::future::lazy(move || {
        let connection_handler =
            crate::server::ConnectionHandler::new(sock_r)
                .map_err(|e| eprintln!("{}", e));
        tokio::spawn(connection_handler);

        server.map(|_| ()).map_err(|_| ())
    }));
    Ok(())
}
