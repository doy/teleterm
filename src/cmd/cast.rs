use snafu::ResultExt as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to connect: {}", source))]
    Connect { source: std::io::Error },

    #[snafu(display("failed to read message: {}", source))]
    Read { source: crate::protocol::Error },

    #[snafu(display("failed to write message: {}", source))]
    Write { source: crate::protocol::Error },

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
    let msg = crate::protocol::Message::start_casting("doy");
    msg.write(&sock).context(Write)?;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
        crate::protocol::Message::heartbeat()
            .write(&sock)
            .context(Write)?;
        let res = crate::protocol::Message::read(&sock).context(Read)?;
        match res {
            crate::protocol::Message::Heartbeat => {
                println!("received heartbeat response");
            }
            _ => {
                return Err(Error::UnexpectedMessage);
            }
        }
    }
}
