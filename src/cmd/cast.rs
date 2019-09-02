use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::ResultExt as _;
use std::io::Write as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to connect: {}", source))]
    Connect { source: std::io::Error },

    #[snafu(display("failed to run process: {}", source))]
    Spawn { source: crate::process::Error },

    #[snafu(display("failed to read message: {}", source))]
    Read { source: crate::protocol::Error },

    #[snafu(display("failed to write message: {}", source))]
    Write { source: crate::protocol::Error },

    #[snafu(display("failed to write to stdout: {}", source))]
    Print { source: std::io::Error },

    #[snafu(display("failed to read message: unexpected message received"))]
    UnexpectedMessage,
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Stream your terminal")
}

pub fn run<'a>(_matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    run_impl().context(super::Cast)
}

fn run_impl() -> Result<()> {
    let sock =
        std::net::TcpStream::connect("127.0.0.1:8000").context(Connect)?;

    crate::protocol::Message::start_casting("doy")
        .write(&sock)
        .context(Write)?;

    let future = crate::process::Process::new("zsh", &[])
        .context(Spawn)?
        .map_err(|e| Error::Spawn { source: e })
        .for_each(|e| {
            match e {
                crate::process::CommandEvent::CommandStart(..) => {}
                crate::process::CommandEvent::CommandExit(..) => {}
                crate::process::CommandEvent::Output(output) => {
                    let stdout = std::io::stdout();
                    let mut stdout = stdout.lock();
                    stdout.write(&output).context(Print)?;
                    stdout.flush().context(Print)?;
                }
            }
            Ok(())
        })
        .map_err(|e| {
            eprintln!("cast: {}", e);
        });

    tokio::run(future);
    Ok(())
}
