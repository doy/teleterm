use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::ResultExt as _;
use std::io::Write as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to connect: {}", source))]
    Connect { source: std::io::Error },

    #[snafu(display("failed to read message: {}", source))]
    Read { source: crate::protocol::Error },

    #[snafu(display("failed to write message: {}", source))]
    Write { source: crate::protocol::Error },

    #[snafu(display("failed to write to terminal: {}", source))]
    WriteTerminal { source: std::io::Error },

    #[snafu(display("communication with server failed: {}", source))]
    Client { source: crate::client::Error },

    #[snafu(display(
        "failed to read message: unexpected message received: {:?}",
        message
    ))]
    UnexpectedMessage { message: crate::protocol::Message },
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Watch termcast streams")
        .arg(clap::Arg::with_name("id"))
}

pub fn run<'a>(matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    run_impl(matches.value_of("id")).context(super::Watch)
}

fn run_impl(id: Option<&str>) -> Result<()> {
    if let Some(id) = id {
        watch(id)
    } else {
        list()
    }
}

fn list() -> Result<()> {
    let sock =
        std::net::TcpStream::connect("127.0.0.1:8000").context(Connect)?;
    let term = std::env::var("TERM").unwrap_or_else(|_| "".to_string());
    let msg = crate::protocol::Message::login("doy", &term);
    msg.write(&sock).context(Write)?;

    let msg = crate::protocol::Message::list_sessions();
    msg.write(&sock).context(Write)?;

    let res = crate::protocol::Message::read(&sock).context(Read)?;
    match res {
        crate::protocol::Message::Sessions { ids } => {
            println!("available sessions:");
            for id in ids {
                println!("{}", id);
            }
        }
        _ => {
            return Err(Error::UnexpectedMessage { message: res });
        }
    }

    Ok(())
}

fn watch(id: &str) -> Result<()> {
    tokio::run(
        WatchSession::new(
            id,
            "127.0.0.1:8000",
            "doy",
            std::time::Duration::from_secs(5),
        )?
        .map_err(|e| {
            eprintln!("{}", e);
        }),
    );

    Ok(())
}

struct WatchSession {
    client: crate::client::Client,
}

impl WatchSession {
    fn new(
        id: &str,
        address: &str,
        username: &str,
        heartbeat_duration: std::time::Duration,
    ) -> Result<Self> {
        let client = crate::client::Client::new(
            address,
            username,
            crate::common::ConnectionType::Watching(id.to_string()),
            heartbeat_duration,
        );
        Ok(Self { client })
    }
}

impl WatchSession {
    const POLL_FNS: &'static [&'static dyn for<'a> Fn(
        &'a mut Self,
    ) -> Result<
        crate::component_future::Poll<()>,
    >] = &[&Self::poll_read_client];

    fn poll_read_client(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        match self.client.poll().context(Client)? {
            futures::Async::Ready(Some(e)) => match e {
                crate::client::Event::Reconnect => {
                    Ok(crate::component_future::Poll::DidWork)
                }
                crate::client::Event::ServerMessage(msg) => match msg {
                    crate::protocol::Message::TerminalOutput { data } => {
                        // TODO async
                        let stderr = std::io::stderr();
                        let mut stderr = stderr.lock();
                        stderr.write(&data).context(WriteTerminal)?;
                        Ok(crate::component_future::Poll::DidWork)
                    }
                    crate::protocol::Message::Disconnected => {
                        Ok(crate::component_future::Poll::Event(()))
                    }
                    msg => Err(Error::UnexpectedMessage { message: msg }),
                },
            },
            futures::Async::Ready(None) => {
                // the client should never exit on its own
                unreachable!()
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }
}

#[must_use = "futures do nothing unless polled"]
impl futures::future::Future for WatchSession {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        crate::component_future::poll_future(self, Self::POLL_FNS)
    }
}
