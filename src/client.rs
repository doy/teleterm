use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::futures01::FutureExt as _;
use snafu::ResultExt as _;
use tokio::io::AsyncRead as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to parse address: {}", source))]
    ParseAddr { source: std::net::AddrParseError },

    #[snafu(display("failed to connect: {}", source))]
    Connect { source: std::io::Error },

    #[snafu(display("failed to write message: {}", source))]
    Write { source: crate::protocol::Error },

    #[snafu(display("heartbeat timer failed: {}", source))]
    Timer { source: tokio::timer::Error },

    #[snafu(display("sending heartbeat failed: {}", source))]
    Heartbeat { source: crate::protocol::Error },

    #[snafu(display("failed to read message from server: {}", source))]
    ReadServer { source: crate::protocol::Error },

    #[snafu(display("failed to write message to server: {}", source))]
    WriteServer { source: crate::protocol::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

enum ReadSocket {
    NotConnected,
    Connected(crate::protocol::FramedReader),
    ReadingMessage(
        Box<
            dyn futures::future::Future<
                    Item = (
                        crate::protocol::Message,
                        crate::protocol::FramedReader,
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
                    Item = crate::protocol::FramedWriter,
                    Error = Error,
                > + Send,
        >,
    ),
    Connected(crate::protocol::FramedWriter),
    WritingMessage(
        Box<
            dyn futures::future::Future<
                    Item = crate::protocol::FramedWriter,
                    Error = Error,
                > + Send,
        >,
    ),
}

pub enum Event {
    ServerMessage(crate::protocol::Message),
    Reconnect,
}

pub enum Type {
    Casting,
    Watching,
}

pub struct Client {
    address: String,
    username: String,
    ty: Type,
    heartbeat_duration: std::time::Duration,

    heartbeat_timer: tokio::timer::Interval,
    reconnect_timer: Option<tokio::timer::Delay>,
    last_server_time: std::time::Instant,

    rsock: ReadSocket,
    wsock: WriteSocket,

    to_send: std::collections::VecDeque<crate::protocol::Message>,
}

impl Client {
    // XXX rustfmt does a terrible job here
    const POLL_FNS: &'static [&'static dyn for<'a> Fn(
        &'a mut Self,
    ) -> Result<
        crate::component_future::Poll<Event>,
    >] = &[
        &Self::poll_reconnect_server,
        &Self::poll_read_server,
        &Self::poll_write_server,
        &Self::poll_heartbeat,
    ];

    pub fn new(
        address: &str,
        username: &str,
        ty: Type,
        heartbeat_duration: std::time::Duration,
    ) -> Self {
        let heartbeat_timer =
            tokio::timer::Interval::new_interval(heartbeat_duration);
        Self {
            address: address.to_string(),
            username: username.to_string(),
            ty,
            heartbeat_duration,

            heartbeat_timer,
            reconnect_timer: None,
            last_server_time: std::time::Instant::now(),

            rsock: ReadSocket::NotConnected,
            wsock: WriteSocket::NotConnected,

            to_send: std::collections::VecDeque::new(),
        }
    }

    pub fn send_message(&mut self, msg: crate::protocol::Message) {
        self.to_send.push_back(msg);
    }

    fn reconnect(&mut self) {
        self.rsock = ReadSocket::NotConnected;
        self.wsock = WriteSocket::NotConnected;
    }

    fn poll_reconnect_server(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        match self.wsock {
            WriteSocket::NotConnected | WriteSocket::Connecting(..) => {}
            _ => {
                let since_last_server = std::time::Instant::now()
                    .duration_since(self.last_server_time);
                if since_last_server > self.heartbeat_duration * 2 {
                    self.reconnect();
                    return Ok(crate::component_future::Poll::DidWork);
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
                        futures::Async::NotReady => {
                            return Ok(
                                crate::component_future::Poll::NotReady,
                            );
                        }
                    }
                }

                self.wsock = WriteSocket::Connecting(Box::new(
                    tokio::net::tcp::TcpStream::connect(
                        &self
                            .address
                            .parse::<std::net::SocketAddr>()
                            .context(ParseAddr)?,
                    )
                    .context(Connect),
                ));
                self.to_send.clear();
                Ok(crate::component_future::Poll::Event(Event::Reconnect))
            }
            WriteSocket::Connecting(ref mut fut) => match fut.poll() {
                Ok(futures::Async::Ready(s)) => {
                    let (rs, ws) = s.split();
                    self.last_server_time = std::time::Instant::now();
                    let term = std::env::var("TERM")
                        .unwrap_or_else(|_| "".to_string());
                    let msg = match self.ty {
                        Type::Casting => {
                            crate::protocol::Message::start_casting(
                                &self.username,
                                &term,
                            )
                        }
                        Type::Watching => {
                            crate::protocol::Message::start_watching(
                                &self.username,
                                &term,
                            )
                        }
                    };
                    let fut = msg
                        .write_async(crate::protocol::FramedWriter::new(ws))
                        .context(Write);
                    self.rsock = ReadSocket::Connected(
                        crate::protocol::FramedReader::new(rs),
                    );
                    self.wsock = WriteSocket::LoggingIn(Box::new(fut));
                    Ok(crate::component_future::Poll::DidWork)
                }
                Ok(futures::Async::NotReady) => {
                    Ok(crate::component_future::Poll::NotReady)
                }
                Err(..) => {
                    self.wsock = WriteSocket::NotConnected;
                    self.reconnect_timer = Some(tokio::timer::Delay::new(
                        std::time::Instant::now()
                            + std::time::Duration::from_secs(1),
                    ));
                    Ok(crate::component_future::Poll::DidWork)
                }
            },
            WriteSocket::LoggingIn(ref mut fut) => match fut.poll()? {
                futures::Async::Ready(s) => {
                    self.last_server_time = std::time::Instant::now();
                    self.wsock = WriteSocket::Connected(s);
                    Ok(crate::component_future::Poll::DidWork)
                }
                futures::Async::NotReady => {
                    Ok(crate::component_future::Poll::NotReady)
                }
            },
            WriteSocket::Connected(..) | WriteSocket::WritingMessage(..) => {
                Ok(crate::component_future::Poll::NothingToDo)
            }
        }
    }

    fn poll_read_server(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        match &mut self.rsock {
            ReadSocket::NotConnected => {
                Ok(crate::component_future::Poll::NothingToDo)
            }
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
                Ok(crate::component_future::Poll::DidWork)
            }
            ReadSocket::ReadingMessage(ref mut fut) => match fut.poll() {
                Ok(futures::Async::Ready((msg, s))) => {
                    self.last_server_time = std::time::Instant::now();
                    self.rsock = ReadSocket::Connected(s);
                    match msg {
                        crate::protocol::Message::Heartbeat => {
                            Ok(crate::component_future::Poll::DidWork)
                        }
                        _ => Ok(crate::component_future::Poll::Event(
                            Event::ServerMessage(msg),
                        )),
                    }
                }
                Ok(futures::Async::NotReady) => {
                    Ok(crate::component_future::Poll::NotReady)
                }
                Err(..) => {
                    self.reconnect();
                    Ok(crate::component_future::Poll::DidWork)
                }
            },
        }
    }

    fn poll_write_server(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        match &mut self.wsock {
            WriteSocket::NotConnected
            | WriteSocket::Connecting(..)
            | WriteSocket::LoggingIn(..) => {
                Ok(crate::component_future::Poll::NothingToDo)
            }
            WriteSocket::Connected(..) => {
                if self.to_send.is_empty() {
                    return Ok(crate::component_future::Poll::NothingToDo);
                }

                let mut tmp = WriteSocket::NotConnected;
                std::mem::swap(&mut self.wsock, &mut tmp);
                if let WriteSocket::Connected(s) = tmp {
                    if let Some(msg) = self.to_send.pop_back() {
                        let fut = msg.write_async(s).context(WriteServer);
                        self.wsock =
                            WriteSocket::WritingMessage(Box::new(fut));
                    } else {
                        unreachable!()
                    }
                } else {
                    unreachable!()
                }
                Ok(crate::component_future::Poll::DidWork)
            }
            WriteSocket::WritingMessage(ref mut fut) => match fut.poll()? {
                futures::Async::Ready(s) => {
                    self.wsock = WriteSocket::Connected(s);
                    Ok(crate::component_future::Poll::DidWork)
                }
                futures::Async::NotReady => {
                    Ok(crate::component_future::Poll::NotReady)
                }
            },
        }
    }

    fn poll_heartbeat(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        match self.heartbeat_timer.poll().context(Timer)? {
            futures::Async::Ready(..) => {
                self.send_message(crate::protocol::Message::heartbeat());
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }
}

#[must_use = "streams do nothing unless polled"]
impl futures::stream::Stream for Client {
    type Item = Event;
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        crate::component_future::poll_stream(self, Self::POLL_FNS)
    }
}