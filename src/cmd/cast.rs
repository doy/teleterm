use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::futures01::stream::StreamExt as _;
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

    #[snafu(display("SIGWINCH handler failed: {}", source))]
    SigWinchHandler { source: std::io::Error },
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
    let args: Vec<_> = if let Some(args) = matches.values_of("args") {
        args.map(std::string::ToString::to_string).collect()
    } else {
        vec![]
    };
    let buffer_size_str =
        matches.value_of("buffer-size").unwrap_or("10000000");
    let buffer_size: usize = buffer_size_str
        .parse()
        .context(crate::error::ParseBufferSize {
            input: buffer_size_str,
        })
        .context(Common)
        .context(super::Cast)?;
    run_impl(
        &matches
            .value_of("username")
            .map(std::string::ToString::to_string)
            .or_else(|| std::env::var("USER").ok())
            .context(crate::error::CouldntFindUsername)
            .context(Common)
            .context(super::Cast)?,
        matches.value_of("address").unwrap_or("127.0.0.1:4144"),
        buffer_size,
        &matches.value_of("command").map_or_else(
            || {
                std::env::var("SHELL")
                    .unwrap_or_else(|_| "/bin/bash".to_string())
            },
            std::string::ToString::to_string,
        ),
        &args,
    )
    .context(super::Cast)
}

fn run_impl(
    username: &str,
    address: &str,
    buffer_size: usize,
    command: &str,
    args: &[String],
) -> Result<()> {
    tokio::run(
        CastSession::new(
            command,
            args,
            address,
            buffer_size,
            username,
            std::time::Duration::from_secs(5),
        )?
        .map_err(|e| {
            eprintln!("{}", e);
        }),
    );

    Ok(())
}

struct CastSession {
    client: crate::client::Client,
    process: crate::process::Process,
    stdout: tokio::io::Stdout,
    winches:
        Box<dyn futures::stream::Stream<Item = (), Error = Error> + Send>,
    buffer: crate::term::Buffer,
    sent_local: usize,
    sent_remote: usize,
    needs_flush: bool,
    done: bool,
    raw_screen: Option<crossterm::RawScreen>,
}

impl CastSession {
    fn new(
        cmd: &str,
        args: &[String],
        address: &str,
        buffer_size: usize,
        username: &str,
        heartbeat_duration: std::time::Duration,
    ) -> Result<Self> {
        let client = crate::client::Client::cast(
            address,
            username,
            heartbeat_duration,
        );
        let process =
            crate::process::Process::new(cmd, args).context(Spawn)?;
        let winches = tokio_signal::unix::Signal::new(
            tokio_signal::unix::libc::SIGWINCH,
        )
        .flatten_stream()
        .map(|_| ())
        .context(SigWinchHandler);
        Ok(Self {
            client,
            process,
            stdout: tokio::io::stdout(),
            winches: Box::new(winches),
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

impl CastSession {
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
        &Self::poll_sigwinch,
    ];

    // this should never return Err, because we don't want server
    // communication issues to ever interrupt a running process
    fn poll_read_client(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        match self.client.poll().context(Client) {
            Ok(futures::Async::Ready(Some(e))) => match e {
                crate::client::Event::Reconnect => {
                    self.sent_remote = 0;
                    Ok(crate::component_future::Poll::DidWork)
                }
                crate::client::Event::ServerMessage(..) => {
                    // we don't expect to ever see a server message once we
                    // start casting, so if one comes through, assume
                    // something is messed up and try again
                    self.client.reconnect();
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

    fn poll_sigwinch(&mut self) -> Result<crate::component_future::Poll<()>> {
        match self.winches.poll()? {
            futures::Async::Ready(Some(_)) => {
                let (cols, rows) = crossterm::terminal()
                    .size()
                    .context(crate::error::GetTerminalSize)
                    .context(Common)?;
                self.process.resize(rows, cols);
                self.client.send_message(crate::protocol::Message::resize((
                    u32::from(cols),
                    u32::from(rows),
                )));
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::Ready(None) => unreachable!(),
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }
}

#[must_use = "futures do nothing unless polled"]
impl futures::future::Future for CastSession {
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
