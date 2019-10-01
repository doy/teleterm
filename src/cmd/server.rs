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
    app.about("Run a shellshare server").arg(
        clap::Arg::with_name("address")
            .long("address")
            .takes_value(true),
    )
}

pub fn run<'a>(matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    run_impl(matches.value_of("address").unwrap_or("0.0.0.0:4144"))
        .context(super::Server)
}

fn run_impl(address: &str) -> Result<()> {
    let (mut sock_w, sock_r) = tokio::sync::mpsc::channel(100);
    let addr = address.parse().context(ParseAddress)?;
    let listener = tokio::net::TcpListener::bind(&addr).context(Bind)?;
    let acceptor = listener
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
        let server = crate::server::Server::new(sock_r)
            .map_err(|e| eprintln!("{}", e));
        tokio::spawn(server);

        acceptor.map(|_| ()).map_err(|_| ())
    }));
    Ok(())
}
