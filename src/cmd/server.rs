use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::ResultExt as _;

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
    run_impl(address, buffer_size, read_timeout).context(super::Server)
}

fn run_impl(
    address: std::net::SocketAddr,
    buffer_size: usize,
    read_timeout: std::time::Duration,
) -> Result<()> {
    let (mut sock_w, sock_r) = tokio::sync::mpsc::channel(100);
    let listener = tokio::net::TcpListener::bind(&address).context(Bind)?;
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
        let server =
            crate::server::Server::new(buffer_size, read_timeout, sock_r)
                .map_err(|e| eprintln!("{}", e));
        tokio::spawn(server);

        acceptor
    }));
    Ok(())
}
