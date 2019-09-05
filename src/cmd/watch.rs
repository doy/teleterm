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
    app.about("Watch termcast streams")
}

pub fn run<'a>(_matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    run_impl().context(super::Watch)
}

fn run_impl() -> Result<()> {
    let sock =
        std::net::TcpStream::connect("127.0.0.1:8000").context(Connect)?;
    let term = std::env::var("TERM").unwrap_or_else(|_| "".to_string());
    let msg = crate::protocol::Message::start_watching("doy", &term);
    msg.write(&sock).context(Write)?;

    let msg = crate::protocol::Message::list_sessions();
    msg.write(&sock).context(Write)?;

    let res = crate::protocol::Message::read(&sock).context(Read)?;
    match res {
        crate::protocol::Message::Sessions { ids } => {
            println!("available sessions:");
            for id in ids {
                println!("got id '{}'", id);
            }
        }
        _ => {
            return Err(Error::UnexpectedMessage);
        }
    }

    Ok(())
}
