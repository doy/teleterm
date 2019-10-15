use crate::prelude::*;
use rand::Rng as _;

const HEARTBEAT_DURATION: std::time::Duration =
    std::time::Duration::from_secs(30);
const RECONNECT_BACKOFF_BASE: std::time::Duration =
    std::time::Duration::from_secs(1);
const RECONNECT_BACKOFF_FACTOR: f32 = 2.0;
const RECONNECT_BACKOFF_MAX: std::time::Duration =
    std::time::Duration::from_secs(60);

enum ReadSocket<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    NotConnected,
    Connected(crate::protocol::FramedReadHalf<S>),
    ReadingMessage(
        Box<
            dyn futures::future::Future<
                    Item = (
                        crate::protocol::Message,
                        crate::protocol::FramedReadHalf<S>,
                    ),
                    Error = Error,
                > + Send,
        >,
    ),
}

enum WriteSocket<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    NotConnected,
    Connecting(
        Box<
            dyn futures::future::Future<
                    Item = S,
                    Error = crate::error::Error,
                > + Send,
        >,
    ),
    Connected(crate::protocol::FramedWriteHalf<S>),
    WritingMessage(
        Box<
            dyn futures::future::Future<
                    Item = crate::protocol::FramedWriteHalf<S>,
                    Error = Error,
                > + Send,
        >,
    ),
}

pub enum Event {
    Start(crate::term::Size),
    ServerMessage(crate::protocol::Message),
    Disconnect,
    Connect(),
    Resize(crate::term::Size),
}

pub type Connector<S> = Box<
    dyn Fn() -> Box<
            dyn futures::future::Future<
                    Item = S,
                    Error = crate::error::Error,
                > + Send,
        > + Send,
>;

pub struct Client<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    connect: Connector<S>,
    username: String,
    buffer_size: usize,

    term_type: String,

    heartbeat_timer: tokio::timer::Interval,
    reconnect_timer: Option<tokio::timer::Delay>,
    reconnect_backoff_amount: std::time::Duration,
    last_server_time: std::time::Instant,
    winches: Option<
        Box<dyn futures::stream::Stream<Item = (), Error = Error> + Send>,
    >,

    rsock: ReadSocket<S>,
    wsock: WriteSocket<S>,

    on_login: Vec<crate::protocol::Message>,
    to_send: std::collections::VecDeque<crate::protocol::Message>,
    started: bool,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Client<S>
{
    pub fn stream(
        connect: Connector<S>,
        username: &str,
        buffer_size: usize,
    ) -> Self {
        Self::new(
            connect,
            username,
            buffer_size,
            &[crate::protocol::Message::start_streaming()],
            true,
        )
    }

    pub fn watch(
        connect: Connector<S>,
        username: &str,
        buffer_size: usize,
        id: &str,
    ) -> Self {
        Self::new(
            connect,
            username,
            buffer_size,
            &[crate::protocol::Message::start_watching(id)],
            false,
        )
    }

    pub fn list(
        connect: Connector<S>,
        username: &str,
        buffer_size: usize,
    ) -> Self {
        Self::new(connect, username, buffer_size, &[], true)
    }

    fn new(
        connect: Connector<S>,
        username: &str,
        buffer_size: usize,
        on_login: &[crate::protocol::Message],
        handle_sigwinch: bool,
    ) -> Self {
        let term_type =
            std::env::var("TERM").unwrap_or_else(|_| "".to_string());
        let heartbeat_timer =
            tokio::timer::Interval::new_interval(HEARTBEAT_DURATION);
        let winches: Option<
            Box<dyn futures::stream::Stream<Item = (), Error = Error> + Send>,
        > = if handle_sigwinch {
            let winches = tokio_signal::unix::Signal::new(
                tokio_signal::unix::libc::SIGWINCH,
            )
            .flatten_stream()
            .map(|_| ())
            .context(crate::error::SigWinchHandler);
            Some(Box::new(winches))
        } else {
            None
        };

        Self {
            connect,
            username: username.to_string(),
            buffer_size,

            term_type,

            heartbeat_timer,
            reconnect_timer: None,
            reconnect_backoff_amount: RECONNECT_BACKOFF_BASE,
            last_server_time: std::time::Instant::now(),
            winches,

            rsock: ReadSocket::NotConnected,
            wsock: WriteSocket::NotConnected,

            on_login: on_login.to_vec(),
            to_send: std::collections::VecDeque::new(),
            started: false,
        }
    }

    pub fn send_message(&mut self, msg: crate::protocol::Message) {
        self.to_send.push_back(msg);
    }

    pub fn reconnect(&mut self) {
        self.rsock = ReadSocket::NotConnected;
        self.wsock = WriteSocket::NotConnected;
    }

    fn set_reconnect_timer(&mut self) {
        let delay = rand::thread_rng().gen_range(
            self.reconnect_backoff_amount / 2,
            self.reconnect_backoff_amount,
        );
        let delay = delay.max(RECONNECT_BACKOFF_BASE);
        self.reconnect_timer =
            Some(tokio::timer::Delay::new(std::time::Instant::now() + delay));
        self.reconnect_backoff_amount = self
            .reconnect_backoff_amount
            .mul_f32(RECONNECT_BACKOFF_FACTOR);
        self.reconnect_backoff_amount =
            self.reconnect_backoff_amount.min(RECONNECT_BACKOFF_MAX);
    }

    fn reset_reconnect_timer(&mut self) {
        self.reconnect_timer = None;
        self.reconnect_backoff_amount = RECONNECT_BACKOFF_BASE;
    }

    fn has_seen_server_recently(&self) -> bool {
        let since_last_server =
            std::time::Instant::now().duration_since(self.last_server_time);
        if since_last_server > HEARTBEAT_DURATION * 2 {
            return false;
        }

        true
    }

    fn should_wait_to_reconnect(&mut self) -> Result<bool> {
        if let Some(timer) = &mut self.reconnect_timer {
            match timer.poll().context(crate::error::TimerReconnect)? {
                futures::Async::NotReady => {
                    return Ok(true);
                }
                _ => {}
            }
        }

        Ok(false)
    }

    fn handle_successful_connection(&mut self, s: S) -> Result<()> {
        self.last_server_time = std::time::Instant::now();

        let (rs, ws) = s.split();
        self.rsock = ReadSocket::Connected(
            crate::protocol::FramedReader::new(rs, self.buffer_size),
        );
        self.wsock = WriteSocket::Connected(
            crate::protocol::FramedWriter::new(ws, self.buffer_size),
        );

        self.to_send.clear();
        self.send_message(crate::protocol::Message::login_plain(
            &self.username,
            &self.term_type,
            &crate::term::Size::get()?,
        ));

        self.reset_reconnect_timer();

        Ok(())
    }

    fn handle_message(
        &mut self,
        msg: crate::protocol::Message,
    ) -> Result<crate::component_future::Poll<Event>> {
        msg.log("recv");

        match msg {
            crate::protocol::Message::LoggedIn { .. } => {
                for msg in &self.on_login {
                    self.to_send.push_back(msg.clone());
                }
                Ok(crate::component_future::Poll::Event(Event::Connect()))
            }
            crate::protocol::Message::Heartbeat => {
                Ok(crate::component_future::Poll::DidWork)
            }
            _ => Ok(crate::component_future::Poll::Event(
                Event::ServerMessage(msg),
            )),
        }
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Client<S>
{
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
        &Self::poll_sigwinch,
    ];

    fn poll_reconnect_server(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        match &mut self.wsock {
            WriteSocket::NotConnected => {
                if self.should_wait_to_reconnect()? {
                    return Ok(crate::component_future::Poll::NotReady);
                }

                self.set_reconnect_timer();
                self.wsock = WriteSocket::Connecting((self.connect)());
            }
            WriteSocket::Connecting(ref mut fut) => match fut.poll() {
                Ok(futures::Async::Ready(s)) => {
                    self.handle_successful_connection(s)?;
                }
                Ok(futures::Async::NotReady) => {
                    return Ok(crate::component_future::Poll::NotReady);
                }
                Err(..) => {
                    self.reconnect();
                    // not sending a disconnect event here because we never
                    // actually connected, so it'd just be spammy
                }
            },
            WriteSocket::Connected(..) | WriteSocket::WritingMessage(..) => {
                if self.has_seen_server_recently() {
                    return Ok(crate::component_future::Poll::NothingToDo);
                } else {
                    self.reconnect();
                    return Ok(crate::component_future::Poll::Event(
                        Event::Disconnect,
                    ));
                }
            }
        }

        Ok(crate::component_future::Poll::DidWork)
    }

    fn poll_read_server(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        match &mut self.rsock {
            ReadSocket::NotConnected => {
                Ok(crate::component_future::Poll::NothingToDo)
            }
            ReadSocket::Connected(..) => {
                if let ReadSocket::Connected(s) = std::mem::replace(
                    &mut self.rsock,
                    ReadSocket::NotConnected,
                ) {
                    let fut = crate::protocol::Message::read_async(s);
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
                    self.handle_message(msg)
                }
                Ok(futures::Async::NotReady) => {
                    Ok(crate::component_future::Poll::NotReady)
                }
                Err(..) => {
                    self.reconnect();
                    Ok(crate::component_future::Poll::Event(
                        Event::Disconnect,
                    ))
                }
            },
        }
    }

    fn poll_write_server(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        match &mut self.wsock {
            WriteSocket::NotConnected | WriteSocket::Connecting(..) => {
                Ok(crate::component_future::Poll::NothingToDo)
            }
            WriteSocket::Connected(..) => {
                if self.to_send.is_empty() {
                    return Ok(crate::component_future::Poll::NothingToDo);
                }

                if let WriteSocket::Connected(s) = std::mem::replace(
                    &mut self.wsock,
                    WriteSocket::NotConnected,
                ) {
                    let msg = self.to_send.pop_front().unwrap();
                    msg.log("send");
                    let fut = msg.write_async(s);
                    self.wsock = WriteSocket::WritingMessage(Box::new(fut));
                } else {
                    unreachable!()
                }

                Ok(crate::component_future::Poll::DidWork)
            }
            WriteSocket::WritingMessage(ref mut fut) => match fut.poll() {
                Ok(futures::Async::Ready(s)) => {
                    self.wsock = WriteSocket::Connected(s);
                    Ok(crate::component_future::Poll::DidWork)
                }
                Ok(futures::Async::NotReady) => {
                    Ok(crate::component_future::Poll::NotReady)
                }
                Err(..) => {
                    self.reconnect();
                    Ok(crate::component_future::Poll::Event(
                        Event::Disconnect,
                    ))
                }
            },
        }
    }

    fn poll_heartbeat(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        match self
            .heartbeat_timer
            .poll()
            .context(crate::error::TimerHeartbeat)?
        {
            futures::Async::Ready(..) => {
                self.send_message(crate::protocol::Message::heartbeat());
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }

    fn poll_sigwinch(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        if let Some(winches) = &mut self.winches {
            if !self.started {
                self.started = true;
                return Ok(crate::component_future::Poll::Event(
                    Event::Start(crate::term::Size::get()?),
                ));
            }

            match winches.poll()? {
                futures::Async::Ready(Some(_)) => {
                    let size = crate::term::Size::get()?;
                    self.send_message(crate::protocol::Message::resize(
                        &size,
                    ));
                    Ok(crate::component_future::Poll::Event(Event::Resize(
                        size,
                    )))
                }
                futures::Async::Ready(None) => unreachable!(),
                futures::Async::NotReady => {
                    Ok(crate::component_future::Poll::NotReady)
                }
            }
        } else {
            Ok(crate::component_future::Poll::NothingToDo)
        }
    }
}

#[must_use = "streams do nothing unless polled"]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    futures::stream::Stream for Client<S>
{
    type Item = Event;
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        crate::component_future::poll_stream(self, Self::POLL_FNS)
    }
}
