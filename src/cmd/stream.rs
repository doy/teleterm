use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::{OptionExt as _, ResultExt as _};
use tokio::io::AsyncWrite as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("{}", source))]
    Common { source: crate::error::Error },

    #[snafu(display("failed to run process: {}", source))]
    Spawn { source: crate::process::Error },

    #[snafu(display("failed to write to stdout: {}", source))]
    WriteTerminal { source: tokio::io::Error },

    #[snafu(display("failed to write to stdout: {}", source))]
    FlushTerminal { source: tokio::io::Error },

    #[snafu(display("process failed: {}", source))]
    Process { source: crate::process::Error },

    #[snafu(display("communication with server failed: {}", source))]
    Client { source: crate::client::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Stream your terminal")
        .arg(
            clap::Arg::with_name("username")
                .long("username")
                .takes_value(true),
        )
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
        .arg(clap::Arg::with_name("command").index(1))
        .arg(clap::Arg::with_name("args").index(2).multiple(true))
}

pub fn run<'a>(matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    let username = matches
        .value_of("username")
        .map(std::string::ToString::to_string)
        .or_else(|| std::env::var("USER").ok())
        .context(crate::error::CouldntFindUsername)
        .context(Common)
        .context(super::Stream)?;
    let address = matches.value_of("address").map_or_else(
        || Ok("0.0.0.0:4144".parse().unwrap()),
        |s| {
            s.parse()
                .context(crate::error::ParseAddr)
                .context(Common)
                .context(super::Stream)
        },
    )?;
    let buffer_size =
        matches
            .value_of("buffer-size")
            .map_or(Ok(4 * 1024 * 1024), |s| {
                s.parse()
                    .context(crate::error::ParseBufferSize { input: s })
                    .context(Common)
                    .context(super::Stream)
            })?;
    let command = matches.value_of("command").map_or_else(
        || std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()),
        std::string::ToString::to_string,
    );
    let args = if let Some(args) = matches.values_of("args") {
        args.map(std::string::ToString::to_string).collect()
    } else {
        vec![]
    };
    run_impl(&username, address, buffer_size, &command, &args)
        .context(super::Stream)
}

fn run_impl(
    username: &str,
    address: std::net::SocketAddr,
    buffer_size: usize,
    command: &str,
    args: &[String],
) -> Result<()> {
    tokio::run(
        StreamSession::new(command, args, address, buffer_size, username)?
            .map_err(|e| {
                eprintln!("{}", e);
            }),
    );

    Ok(())
}

struct StreamSession {
    client: crate::client::Client,
    process: crate::process::Process<crate::async_stdin::Stdin>,
    stdout: tokio::io::Stdout,
    buffer: crate::term::Buffer,
    sent_local: usize,
    sent_remote: usize,
    needs_flush: bool,
    done: bool,
    raw_screen: Option<crossterm::RawScreen>,
}

impl StreamSession {
    fn new(
        cmd: &str,
        args: &[String],
        address: std::net::SocketAddr,
        buffer_size: usize,
        username: &str,
    ) -> Result<Self> {
        let client =
            crate::client::Client::stream(address, username, buffer_size);

        // TODO: tokio::io::stdin is broken (it's blocking)
        // see https://github.com/tokio-rs/tokio/issues/589
        // let input = tokio::io::stdin();
        let input = crate::async_stdin::Stdin::new();

        let process =
            crate::process::Process::new(cmd, args, input).context(Spawn)?;

        Ok(Self {
            client,
            process,
            stdout: tokio::io::stdout(),
            buffer: crate::term::Buffer::new(buffer_size),
            sent_local: 0,
            sent_remote: 0,
            needs_flush: false,
            done: false,
            raw_screen: None,
        })
    }

    fn record_bytes(&mut self, buf: &[u8]) {
        let truncated = self.buffer.append(buf);
        if truncated > self.sent_local {
            self.sent_local = 0;
        } else {
            self.sent_local -= truncated;
        }
        if truncated > self.sent_remote {
            self.sent_remote = 0;
        } else {
            self.sent_remote -= truncated;
        }
    }
}

impl StreamSession {
    const POLL_FNS: &'static [&'static dyn for<'a> Fn(
        &'a mut Self,
    ) -> Result<
        crate::component_future::Poll<()>,
    >] = &[
        &Self::poll_read_client,
        &Self::poll_read_process,
        &Self::poll_write_terminal,
        &Self::poll_flush_terminal,
        &Self::poll_write_server,
    ];

    // this should never return Err, because we don't want server
    // communication issues to ever interrupt a running process
    fn poll_read_client(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        match self.client.poll().context(Client) {
            Ok(futures::Async::Ready(Some(e))) => match e {
                crate::client::Event::Disconnect => {
                    Ok(crate::component_future::Poll::DidWork)
                }
                crate::client::Event::Connect(size) => {
                    self.sent_remote = 0;
                    self.process.resize(size);
                    Ok(crate::component_future::Poll::DidWork)
                }
                crate::client::Event::ServerMessage(..) => {
                    // we don't expect to ever see a server message once we
                    // start streaming, so if one comes through, assume
                    // something is messed up and try again
                    self.client.reconnect();
                    Ok(crate::component_future::Poll::DidWork)
                }
                crate::client::Event::Resize(size) => {
                    self.process.resize(size);
                    Ok(crate::component_future::Poll::DidWork)
                }
            },
            Ok(futures::Async::Ready(None)) => {
                // the client should never exit on its own
                unreachable!()
            }
            Ok(futures::Async::NotReady) => {
                Ok(crate::component_future::Poll::NotReady)
            }
            Err(..) => {
                self.client.reconnect();
                Ok(crate::component_future::Poll::DidWork)
            }
        }
    }

    fn poll_read_process(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        match self.process.poll().context(Process)? {
            futures::Async::Ready(Some(e)) => {
                match e {
                    crate::process::Event::CommandStart(..) => {}
                    crate::process::Event::CommandExit(..) => {
                        self.done = true;
                    }
                    crate::process::Event::Output(output) => {
                        self.record_bytes(&output);
                    }
                }
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::Ready(None) => {
                if !self.done {
                    unreachable!()
                }
                // don't return final event here - wait until we are done
                // sending all data to the server (see poll_write_server)
                Ok(crate::component_future::Poll::NothingToDo)
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }

    fn poll_write_terminal(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        if self.sent_local == self.buffer.len() {
            return Ok(crate::component_future::Poll::NothingToDo);
        }

        match self
            .stdout
            .poll_write(&self.buffer.contents()[self.sent_local..])
            .context(WriteTerminal)?
        {
            futures::Async::Ready(n) => {
                self.sent_local += n;
                self.needs_flush = true;
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }

    fn poll_flush_terminal(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        if !self.needs_flush {
            return Ok(crate::component_future::Poll::NothingToDo);
        }

        match self.stdout.poll_flush().context(FlushTerminal)? {
            futures::Async::Ready(()) => {
                self.needs_flush = false;
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }

    fn poll_write_server(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        if self.sent_remote == self.buffer.len() {
            // ship all data to the server before actually ending
            if self.done {
                return Ok(crate::component_future::Poll::Event(()));
            } else {
                return Ok(crate::component_future::Poll::NothingToDo);
            }
        }

        let buf = &self.buffer.contents()[self.sent_remote..];
        self.client
            .send_message(crate::protocol::Message::terminal_output(buf));
        self.sent_remote = self.buffer.len();

        Ok(crate::component_future::Poll::DidWork)
    }
}

#[must_use = "futures do nothing unless polled"]
impl futures::future::Future for StreamSession {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        if self.raw_screen.is_none() {
            self.raw_screen = Some(
                crossterm::RawScreen::into_raw_mode()
                    .context(crate::error::IntoRawMode)
                    .context(Common)?,
            );
        }
        crate::component_future::poll_future(self, Self::POLL_FNS)
    }
}
