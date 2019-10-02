use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::futures01::stream::StreamExt as _;
use snafu::ResultExt as _;
use tokio::io::AsyncWrite as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
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
            clap::Arg::with_name("address")
                .long("address")
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
    run_impl(
        matches.value_of("address").unwrap_or("127.0.0.1:4144"),
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

fn run_impl(address: &str, command: &str, args: &[String]) -> Result<()> {
    tokio::run(
        CastSession::new(
            command,
            args,
            address,
            "doy",
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
}

impl CastSession {
    fn new(
        cmd: &str,
        args: &[String],
        address: &str,
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
            buffer: crate::term::Buffer::new(),
            sent_local: 0,
            sent_remote: 0,
            needs_flush: false,
            done: false,
        })
    }

    fn record_bytes(&mut self, buf: &[u8]) {
        if self.buffer.append(buf) {
            self.sent_local = 0;
            self.sent_remote = 0;
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
                // TODO
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
        crate::component_future::poll_future(self, Self::POLL_FNS)
    }
}
