use snafu::ResultExt as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to connect: {}", source))]
    Connect { source: std::io::Error },

    #[snafu(display("failed to write message: {}", source))]
    Write { source: crate::protocol::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Watch termcast streams")
}

pub fn run<'a>(_matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    run_impl().context(super::Watch)
}

fn run_impl() -> Result<()> {
    let sock =
        std::net::TcpStream::connect("127.0.0.1:8001").context(Connect)?;
    let msg = crate::protocol::Message::start_watching("doy");
    msg.write(sock).context(Write)?;
    println!("wrote message successfully");
    Ok(())
}
