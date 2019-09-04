use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::futures01::FutureExt as _;
use snafu::ResultExt as _;
use tokio::io::AsyncWrite as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to parse address: {}", source))]
    ParseAddr { source: std::net::AddrParseError },

    #[snafu(display("failed to connect: {}", source))]
    Connect { source: std::io::Error },

    #[snafu(display("failed to run process: {}", source))]
    Spawn { source: crate::process::Error },

    #[snafu(display("failed to write message: {}", source))]
    Write { source: crate::protocol::Error },

    #[snafu(display("failed to write to stdout: {}", source))]
    WriteTerminal { source: tokio::io::Error },

    #[snafu(display("failed to write to stdout: {}", source))]
    FlushTerminal { source: tokio::io::Error },

    #[snafu(display("process failed: {}", source))]
    Process { source: crate::process::Error },

    #[snafu(display("failed to send output to server: {}", source))]
    ServerOutput { source: crate::protocol::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Stream your terminal")
}

pub fn run<'a>(_matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    run_impl().context(super::Cast)
}

fn run_impl() -> Result<()> {
    tokio::run(CastSession::new("zsh", &[])?.map_err(|e| {
        eprintln!("{}", e);
    }));

    Ok(())
}

enum Socket {
    NotConnected,
    Connecting(
        Box<
            dyn futures::future::Future<
                    Item = tokio::net::tcp::TcpStream,
                    Error = Error,
                > + Send,
        >,
    ),
    LoggingIn(
        Box<
            dyn futures::future::Future<
                    Item = tokio::net::tcp::TcpStream,
                    Error = Error,
                > + Send,
        >,
    ),
    Connected(tokio::net::tcp::TcpStream),
    SendingMessage(
        Box<
            dyn futures::future::Future<
                    Item = tokio::net::tcp::TcpStream,
                    Error = Error,
                > + Send,
        >,
        usize,
    ),
}

struct CastSession {
    process: crate::process::Process,
    heartbeat_timer: tokio::timer::Interval,
    stdout: tokio::io::Stdout,
    sock: Socket,
    buffer: Vec<u8>,
    sent_local: usize,
    sent_remote: usize,
    needs_flush: bool,
    done: bool,
}

impl CastSession {
    fn new(cmd: &str, args: &[String]) -> Result<Self> {
        let process =
            crate::process::Process::new(cmd, args).context(Spawn)?;
        let heartbeat_timer = tokio::timer::Interval::new_interval(
            std::time::Duration::from_secs(15),
        );
        Ok(Self {
            process,
            heartbeat_timer,
            stdout: tokio::io::stdout(),
            sock: Socket::NotConnected,
            buffer: vec![],
            sent_local: 0,
            sent_remote: 0,
            needs_flush: false,
            done: false,
        })
    }

    fn poll_reconnect_server(&mut self) -> Result<bool> {
        match &mut self.sock {
            Socket::NotConnected => {
                self.sock = Socket::Connecting(Box::new(
                    tokio::net::tcp::TcpStream::connect(
                        &"127.0.0.1:8000"
                            .parse::<std::net::SocketAddr>()
                            .context(ParseAddr)?,
                    )
                    .context(Connect),
                ));
                Ok(false)
            }
            Socket::Connecting(ref mut fut) => match fut.poll()? {
                futures::Async::Ready(s) => {
                    let fut = crate::protocol::Message::start_casting("doy")
                        .write_async(s)
                        .context(Write);
                    self.sock = Socket::LoggingIn(Box::new(fut));
                    Ok(true)
                }
                futures::Async::NotReady => Ok(false),
            },
            Socket::LoggingIn(ref mut fut) => match fut.poll()? {
                futures::Async::Ready(s) => {
                    self.sock = Socket::Connected(s);
                    Ok(true)
                }
                futures::Async::NotReady => Ok(false),
            },
            Socket::Connected(..) => Ok(false),
            Socket::SendingMessage(..) => Ok(false),
        }
    }

    fn poll_read_process(&mut self) -> Result<bool> {
        match self.process.poll().context(Process)? {
            futures::Async::Ready(Some(e)) => match e {
                crate::process::CommandEvent::CommandStart(..) => {}
                crate::process::CommandEvent::CommandExit(..) => {
                    self.done = true
                }
                crate::process::CommandEvent::Output(mut output) => {
                    // XXX handle terminal resets
                    self.buffer.append(&mut output);
                }
            },
            futures::Async::Ready(None) => {
                if !self.done {
                    unreachable!()
                }
            }
            futures::Async::NotReady => return Ok(false),
        }
        Ok(true)
    }

    fn poll_read_server(&mut self) -> Result<bool> {
        match self.sock {
            Socket::Connected(..) => {}
            _ => return Ok(false),
        }

        Ok(false)
    }

    fn poll_write_terminal(&mut self) -> Result<bool> {
        if self.sent_local == self.buffer.len() {
            return Ok(false);
        }

        match self
            .stdout
            .poll_write(&self.buffer[self.sent_local..])
            .context(WriteTerminal)?
        {
            futures::Async::Ready(n) => {
                self.sent_local += n;
                self.needs_flush = true;
                Ok(true)
            }
            futures::Async::NotReady => Ok(false),
        }
    }

    fn poll_flush_terminal(&mut self) -> Result<bool> {
        if !self.needs_flush {
            return Ok(false);
        }

        match self.stdout.poll_flush().context(FlushTerminal)? {
            futures::Async::Ready(()) => {
                self.needs_flush = false;
                Ok(true)
            }
            futures::Async::NotReady => Ok(false),
        }
    }

    fn poll_write_server_output(&mut self) -> Result<bool> {
        match &mut self.sock {
            Socket::NotConnected => Ok(false),
            Socket::Connecting(..) => Ok(false),
            Socket::LoggingIn(..) => Ok(false),
            Socket::Connected(..) => {
                if self.sent_remote == self.buffer.len() {
                    return Ok(false);
                }
                let buf = &self.buffer[self.sent_remote..];
                let mut tmp = Socket::NotConnected;
                std::mem::swap(&mut self.sock, &mut tmp);
                if let Socket::Connected(s) = tmp {
                    let fut = crate::protocol::Message::terminal_output(buf)
                        .write_async(s)
                        .context(ServerOutput);
                    self.sock =
                        Socket::SendingMessage(Box::new(fut), buf.len());
                } else {
                    unreachable!()
                }
                Ok(false)
            }
            Socket::SendingMessage(ref mut fut, ref n) => match fut.poll()? {
                futures::Async::Ready(s) => {
                    self.sent_remote += n;
                    self.sock = Socket::Connected(s);
                    Ok(true)
                }
                futures::Async::NotReady => Ok(false),
            },
        }
    }

    fn poll_write_server_heartbeat(&mut self) -> Result<bool> {
        match self.sock {
            Socket::Connected(..) => {}
            _ => return Ok(false),
        }

        Ok(false)
    }
}

impl futures::future::Future for CastSession {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        loop {
            let mut did_work = false;

            did_work |= self.poll_reconnect_server()?;
            did_work |= self.poll_read_process()?;
            did_work |= self.poll_read_server()?;
            did_work |= self.poll_write_terminal()?;
            did_work |= self.poll_flush_terminal()?;
            did_work |= self.poll_write_server_output()?;
            did_work |= self.poll_write_server_heartbeat()?;

            if self.done {
                return Ok(futures::Async::Ready(()));
            }

            if !did_work {
                break;
            }
        }

        Ok(futures::Async::NotReady)
    }
}
