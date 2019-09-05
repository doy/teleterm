use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::futures01::FutureExt as _;
use snafu::ResultExt as _;
use tokio::io::{AsyncRead as _, AsyncWrite as _};

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

    #[snafu(display("heartbeat timer failed: {}", source))]
    Timer { source: tokio::timer::Error },

    #[snafu(display("sending heartbeat failed: {}", source))]
    Heartbeat { source: crate::protocol::Error },

    #[snafu(display("failed to read message from server: {}", source))]
    ReadServer { source: crate::protocol::Error },

    #[snafu(display("unexpected message: {:?}", message))]
    UnexpectedMessage { message: crate::protocol::Message },
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Stream your terminal")
}

pub fn run<'a>(_matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    run_impl().context(super::Cast)
}

fn run_impl() -> Result<()> {
    tokio::run(
        CastSession::new("zsh", &[], std::time::Duration::from_secs(5))?
            .map_err(|e| {
                eprintln!("{}", e);
            }),
    );

    Ok(())
}

enum ReadSocket {
    NotConnected,
    Connected(tokio::io::ReadHalf<tokio::net::tcp::TcpStream>),
    ReadingMessage(
        Box<
            dyn futures::future::Future<
                    Item = (
                        crate::protocol::Message,
                        tokio::io::ReadHalf<tokio::net::tcp::TcpStream>,
                    ),
                    Error = Error,
                > + Send,
        >,
    ),
}

enum WriteSocket {
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
                    Item = tokio::io::WriteHalf<tokio::net::tcp::TcpStream>,
                    Error = Error,
                > + Send,
        >,
    ),
    Connected(tokio::io::WriteHalf<tokio::net::tcp::TcpStream>),
    SendingOutput(
        Box<
            dyn futures::future::Future<
                    Item = tokio::io::WriteHalf<tokio::net::tcp::TcpStream>,
                    Error = Error,
                > + Send,
        >,
        usize,
    ),
    SendingHeartbeat(
        Box<
            dyn futures::future::Future<
                    Item = tokio::io::WriteHalf<tokio::net::tcp::TcpStream>,
                    Error = Error,
                > + Send,
        >,
    ),
}

struct CastSession {
    process: crate::process::Process,
    heartbeat_timer: tokio::timer::Interval,
    reconnect_timer: Option<tokio::timer::Delay>,
    heartbeat_duration: std::time::Duration,
    stdout: tokio::io::Stdout,
    rsock: ReadSocket,
    wsock: WriteSocket,
    buffer: Vec<u8>,
    sent_local: usize,
    sent_remote: usize,
    needs_flush: bool,
    needs_send_heartbeat: bool,
    done: bool,
    last_server_time: std::time::Instant,
}

impl CastSession {
    fn new(
        cmd: &str,
        args: &[String],
        heartbeat_duration: std::time::Duration,
    ) -> Result<Self> {
        let process =
            crate::process::Process::new(cmd, args).context(Spawn)?;
        let heartbeat_timer =
            tokio::timer::Interval::new_interval(heartbeat_duration);
        Ok(Self {
            process,
            heartbeat_timer,
            reconnect_timer: None,
            heartbeat_duration,
            stdout: tokio::io::stdout(),
            rsock: ReadSocket::NotConnected,
            wsock: WriteSocket::NotConnected,
            buffer: vec![],
            sent_local: 0,
            sent_remote: 0,
            needs_flush: false,
            needs_send_heartbeat: false,
            done: false,
            last_server_time: std::time::Instant::now(),
        })
    }

    fn poll_reconnect_server(&mut self) -> Result<bool> {
        match self.wsock {
            WriteSocket::NotConnected => {}
            WriteSocket::Connecting(..) => {}
            _ => {
                let since_last_server = std::time::Instant::now()
                    .duration_since(self.last_server_time);
                if since_last_server > self.heartbeat_duration * 2 {
                    self.rsock = ReadSocket::NotConnected;
                    self.wsock = WriteSocket::NotConnected;
                    self.sent_remote = 0;
                    return Ok(true);
                }
            }
        }

        match &mut self.wsock {
            WriteSocket::NotConnected => {
                if let Some(timer) = &mut self.reconnect_timer {
                    match timer.poll().context(Timer)? {
                        futures::Async::Ready(..) => {
                            self.reconnect_timer = None;
                        }
                        futures::Async::NotReady => return Ok(false),
                    }
                }

                self.wsock = WriteSocket::Connecting(Box::new(
                    tokio::net::tcp::TcpStream::connect(
                        &"127.0.0.1:8000"
                            .parse::<std::net::SocketAddr>()
                            .context(ParseAddr)?,
                    )
                    .context(Connect),
                ));
                Ok(false)
            }
            WriteSocket::Connecting(ref mut fut) => match fut.poll() {
                Ok(futures::Async::Ready(s)) => {
                    let (rs, ws) = s.split();
                    self.last_server_time = std::time::Instant::now();
                    let fut = crate::protocol::Message::start_casting("doy")
                        .write_async(ws)
                        .context(Write);
                    self.rsock = ReadSocket::Connected(rs);
                    self.wsock = WriteSocket::LoggingIn(Box::new(fut));
                    Ok(true)
                }
                Ok(futures::Async::NotReady) => Ok(false),
                Err(..) => {
                    self.wsock = WriteSocket::NotConnected;
                    self.reconnect_timer = Some(tokio::timer::Delay::new(
                        std::time::Instant::now()
                            + std::time::Duration::from_secs(1),
                    ));
                    Ok(true)
                }
            },
            WriteSocket::LoggingIn(ref mut fut) => match fut.poll()? {
                futures::Async::Ready(s) => {
                    self.last_server_time = std::time::Instant::now();
                    self.wsock = WriteSocket::Connected(s);
                    Ok(true)
                }
                futures::Async::NotReady => Ok(false),
            },
            WriteSocket::Connected(..) => Ok(false),
            WriteSocket::SendingOutput(..) => Ok(false),
            WriteSocket::SendingHeartbeat(..) => Ok(false),
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
        match &mut self.rsock {
            ReadSocket::NotConnected => Ok(false),
            ReadSocket::Connected(..) => {
                let mut tmp = ReadSocket::NotConnected;
                std::mem::swap(&mut self.rsock, &mut tmp);
                if let ReadSocket::Connected(s) = tmp {
                    let fut = crate::protocol::Message::read_async(s)
                        .context(ReadServer);
                    self.rsock = ReadSocket::ReadingMessage(Box::new(fut));
                } else {
                    unreachable!()
                }
                Ok(false)
            }
            ReadSocket::ReadingMessage(ref mut fut) => match fut.poll() {
                Ok(futures::Async::Ready((msg, s))) => {
                    self.last_server_time = std::time::Instant::now();
                    self.rsock = ReadSocket::Connected(s);
                    match msg {
                        crate::protocol::Message::Heartbeat => {}
                        _ => {
                            return Err(Error::UnexpectedMessage {
                                message: msg,
                            });
                        }
                    }
                    Ok(true)
                }
                Ok(futures::Async::NotReady) => Ok(false),
                Err(..) => {
                    self.rsock = ReadSocket::NotConnected;
                    self.wsock = WriteSocket::NotConnected;
                    Ok(true)
                }
            },
        }
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
        match &mut self.wsock {
            WriteSocket::NotConnected => Ok(false),
            WriteSocket::Connecting(..) => Ok(false),
            WriteSocket::LoggingIn(..) => Ok(false),
            WriteSocket::Connected(..) => {
                if self.sent_remote == self.buffer.len() {
                    return Ok(false);
                }
                let buf = &self.buffer[self.sent_remote..];
                let mut tmp = WriteSocket::NotConnected;
                std::mem::swap(&mut self.wsock, &mut tmp);
                if let WriteSocket::Connected(s) = tmp {
                    let fut = crate::protocol::Message::terminal_output(buf)
                        .write_async(s)
                        .context(ServerOutput);
                    self.wsock =
                        WriteSocket::SendingOutput(Box::new(fut), buf.len());
                } else {
                    unreachable!()
                }
                Ok(false)
            }
            WriteSocket::SendingOutput(ref mut fut, ref n) => {
                match fut.poll()? {
                    futures::Async::Ready(s) => {
                        self.sent_remote += n;
                        self.wsock = WriteSocket::Connected(s);
                        Ok(true)
                    }
                    futures::Async::NotReady => Ok(false),
                }
            }
            WriteSocket::SendingHeartbeat(..) => Ok(false),
        }
    }

    fn poll_needs_heartbeat(&mut self) -> Result<bool> {
        match self.heartbeat_timer.poll().context(Timer)? {
            futures::Async::Ready(..) => {
                self.needs_send_heartbeat = true;
                Ok(true)
            }
            futures::Async::NotReady => Ok(false),
        }
    }

    fn poll_write_server_heartbeat(&mut self) -> Result<bool> {
        match self.wsock {
            WriteSocket::NotConnected => Ok(false),
            WriteSocket::Connecting(..) => Ok(false),
            WriteSocket::LoggingIn(..) => Ok(false),
            WriteSocket::Connected(..) => {
                if !self.needs_send_heartbeat {
                    return Ok(false);
                }

                self.needs_send_heartbeat = false;
                let mut tmp = WriteSocket::NotConnected;
                std::mem::swap(&mut self.wsock, &mut tmp);
                if let WriteSocket::Connected(s) = tmp {
                    let fut = crate::protocol::Message::heartbeat()
                        .write_async(s)
                        .context(Heartbeat);
                    self.wsock = WriteSocket::SendingHeartbeat(Box::new(fut));
                    Ok(true)
                } else {
                    unreachable!()
                }
            }
            WriteSocket::SendingOutput(..) => Ok(false),
            WriteSocket::SendingHeartbeat(ref mut fut) => match fut.poll() {
                Ok(futures::Async::Ready(s)) => {
                    self.wsock = WriteSocket::Connected(s);
                    Ok(true)
                }
                Ok(futures::Async::NotReady) => Ok(false),
                Err(e) => {
                    self.needs_send_heartbeat = true;
                    Err(e)
                }
            },
        }
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
            did_work |= self.poll_needs_heartbeat()?;
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
