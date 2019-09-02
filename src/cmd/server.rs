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
    let cast_addr = "127.0.0.1:8000".parse().context(ParseAddress)?;
    let cast_listener =
        tokio::net::TcpListener::bind(&cast_addr).context(Bind)?;
    let cast_server = cast_listener
        .incoming()
        .map_err(|e| {
            eprintln!("accept failed: {}", e);
        })
        .for_each(|sock| {
            crate::protocol::Message::read_async(sock)
                .map(|msg| match msg {
                    crate::protocol::Message::StartCasting { username } => {
                        println!("got a cast connection from {}", username);
                    }
                    m => println!("unexpected message received: {:?}", m),
                })
                .map_err(|e| {
                    eprintln!("failed to read message: {}", e);
                })
        });

    let watch_addr = "127.0.0.1:8001".parse().context(ParseAddress)?;
    let watch_listener =
        tokio::net::TcpListener::bind(&watch_addr).context(Bind)?;
    let watch_server = watch_listener
        .incoming()
        .map_err(|e| {
            eprintln!("accept failed: {}", e);
        })
        .for_each(|sock| {
            crate::protocol::Message::read_async(sock)
                .map(|msg| match msg {
                    crate::protocol::Message::StartWatching { username } => {
                        println!("got a watch connection from {}", username);
                    }
                    m => println!("unexpected message received: {:?}", m),
                })
                .map_err(|e| {
                    eprintln!("failed to read message: {}", e);
                })
        });

    let servers: Vec<
        Box<dyn futures::future::Future<Item = (), Error = ()> + Send>,
    > = vec![Box::new(cast_server), Box::new(watch_server)];

    tokio::run(
        futures::future::join_all(servers)
            .map(|_| ())
            .map_err(|_| ()),
    );
    Ok(())
}
