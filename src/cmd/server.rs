use snafu::ResultExt as _;
use tokio::prelude::*;

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
    let addr = "127.0.0.1:8000".parse().context(ParseAddress)?;
    let listener = tokio::net::TcpListener::bind(&addr).context(Bind)?;
    let server = listener
        .incoming()
        .for_each(|_sock| {
            println!("got a connection");
            Ok(())
        })
        .map_err(|e| {
            eprintln!("accept failed: {}", e);
        });
    tokio::run(server);
    Ok(())
}
