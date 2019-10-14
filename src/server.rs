use crate::prelude::*;
use tokio::util::FutureExt as _;

pub mod tls;

enum ReadSocket<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    Connected(crate::protocol::FramedReadHalf<S>),
    Reading(
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
    Connected(crate::protocol::FramedWriteHalf<S>),
    Writing(
        Box<
            dyn futures::future::Future<
                    Item = crate::protocol::FramedWriteHalf<S>,
                    Error = Error,
                > + Send,
        >,
    ),
}

#[derive(Debug, Clone)]
struct TerminalInfo {
    term: String,
    size: crate::term::Size,
}

// XXX https://github.com/rust-lang/rust/issues/64362
#[allow(dead_code)]
enum ConnectionState {
    Accepted,
    LoggedIn {
        username: String,
        term_info: TerminalInfo,
    },
    Streaming {
        username: String,
        term_info: TerminalInfo,
        saved_data: crate::term::Buffer,
    },
    Watching {
        username: String,
        term_info: TerminalInfo,
        watch_id: String,
    },
}

impl ConnectionState {
    fn new() -> Self {
        Self::Accepted
    }

    fn username(&self) -> Option<&str> {
        match self {
            Self::Accepted => None,
            Self::LoggedIn { username, .. } => Some(username),
            Self::Streaming { username, .. } => Some(username),
            Self::Watching { username, .. } => Some(username),
        }
    }

    fn login(
        &self,
        username: &str,
        term_type: &str,
        size: &crate::term::Size,
    ) -> Self {
        match self {
            Self::Accepted => Self::LoggedIn {
                username: username.to_string(),
                term_info: TerminalInfo {
                    term: term_type.to_string(),
                    size: size.clone(),
                },
            },
            _ => unreachable!(),
        }
    }

    fn stream(&self, buffer_size: usize) -> Self {
        match self {
            Self::LoggedIn {
                username,
                term_info,
            } => Self::Streaming {
                username: username.clone(),
                term_info: term_info.clone(),
                saved_data: crate::term::Buffer::new(buffer_size),
            },
            _ => unreachable!(),
        }
    }

    fn watch(&self, id: &str) -> Self {
        match self {
            Self::LoggedIn {
                username,
                term_info,
            } => Self::Watching {
                username: username.clone(),
                term_info: term_info.clone(),
                watch_id: id.to_string(),
            },
            _ => unreachable!(),
        }
    }
}

struct Connection<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    id: String,
    rsock: Option<ReadSocket<S>>,
    wsock: Option<WriteSocket<S>>,
    to_send: std::collections::VecDeque<crate::protocol::Message>,
    closed: bool,
    state: ConnectionState,
    last_activity: std::time::Instant,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Connection<S>
{
    fn new(s: S, buffer_size: usize) -> Self {
        let (rs, ws) = s.split();
        let id = format!("{}", uuid::Uuid::new_v4());
        log::info!("{}: new connection", id);

        Self {
            id,
            rsock: Some(ReadSocket::Connected(
                crate::protocol::FramedReader::new(rs, buffer_size),
            )),
            wsock: Some(WriteSocket::Connected(
                crate::protocol::FramedWriter::new(ws, buffer_size),
            )),
            to_send: std::collections::VecDeque::new(),
            closed: false,
            state: ConnectionState::new(),
            last_activity: std::time::Instant::now(),
        }
    }

    fn session(&self) -> Option<crate::protocol::Session> {
        let (username, term_info) = match &self.state {
            ConnectionState::Accepted => return None,
            ConnectionState::LoggedIn {
                username,
                term_info,
            } => (username, term_info),
            ConnectionState::Streaming {
                username,
                term_info,
                ..
            } => (username, term_info),
            ConnectionState::Watching {
                username,
                term_info,
                ..
            } => (username, term_info),
        };
        let title = if let ConnectionState::Streaming { saved_data, .. } =
            &self.state
        {
            saved_data.title()
        } else {
            ""
        };

        // i don't really care if things break for a connection that has been
        // idle for 136 years
        #[allow(clippy::cast_possible_truncation)]
        Some(crate::protocol::Session {
            id: self.id.clone(),
            username: username.clone(),
            term_type: term_info.term.clone(),
            size: term_info.size.clone(),
            idle_time: std::time::Instant::now()
                .duration_since(self.last_activity)
                .as_secs() as u32,
            title: title.to_string(),
        })
    }

    fn close(&mut self, res: Result<()>) {
        let msg = match res {
            Ok(()) => crate::protocol::Message::disconnected(),
            Err(e) => crate::protocol::Message::error(&format!("{}", e)),
        };
        self.to_send.push_back(msg);
        self.closed = true;
    }
}

pub struct Server<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    buffer_size: usize,
    read_timeout: std::time::Duration,
    sock_stream: Box<
        dyn futures::stream::Stream<Item = Connection<S>, Error = Error>
            + Send,
    >,
    connections: std::collections::HashMap<String, Connection<S>>,
    rate_limiter: ratelimit_meter::KeyedRateLimiter<Option<String>>,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Server<S>
{
    pub fn new(
        buffer_size: usize,
        read_timeout: std::time::Duration,
        sock_r: tokio::sync::mpsc::Receiver<S>,
    ) -> Self {
        let sock_stream = sock_r
            .map(move |s| Connection::new(s, buffer_size))
            .context(crate::error::SocketChannelReceive);
        Self {
            buffer_size,
            read_timeout,
            sock_stream: Box::new(sock_stream),
            connections: std::collections::HashMap::new(),
            rate_limiter: ratelimit_meter::KeyedRateLimiter::new(
                std::num::NonZeroU32::new(300).unwrap(),
                std::time::Duration::from_secs(60),
            ),
        }
    }

    fn handle_accepted_message(
        &mut self,
        conn: &mut Connection<S>,
        message: crate::protocol::Message,
    ) -> Result<()> {
        match message {
            crate::protocol::Message::Login {
                auth,
                term_type,
                size,
                ..
            } => {
                if size.rows >= 1000 || size.cols >= 1000 {
                    return Err(Error::TermTooBig { size });
                }
                match auth {
                    crate::protocol::Auth::Plain { username } => {
                        log::info!("{}: login({})", conn.id, username);
                        conn.state =
                            conn.state.login(&username, &term_type, &size);
                    }
                }
                Ok(())
            }
            m => Err(Error::UnauthenticatedMessage { message: m }),
        }
    }

    fn handle_logged_in_message(
        &mut self,
        conn: &mut Connection<S>,
        message: crate::protocol::Message,
    ) -> Result<()> {
        let (username, term_info) = if let ConnectionState::LoggedIn {
            username,
            term_info,
            ..
        } = &mut conn.state
        {
            (username, term_info)
        } else {
            unreachable!()
        };

        match message {
            crate::protocol::Message::Heartbeat => {
                conn.to_send
                    .push_back(crate::protocol::Message::heartbeat());
                Ok(())
            }
            crate::protocol::Message::Resize { size } => {
                term_info.size = size;
                Ok(())
            }
            crate::protocol::Message::ListSessions => {
                let sessions: Vec<_> =
                    self.streamers().flat_map(Connection::session).collect();
                conn.to_send
                    .push_back(crate::protocol::Message::sessions(&sessions));
                Ok(())
            }
            crate::protocol::Message::StartStreaming => {
                log::info!("{}: stream({})", conn.id, username);
                conn.state = conn.state.stream(self.buffer_size);
                Ok(())
            }
            crate::protocol::Message::StartWatching { id } => {
                if let Some(stream_conn) = self.connections.get(&id) {
                    log::info!("{}: watch({}, {})", conn.id, username, id);
                    conn.state = conn.state.watch(&id);
                    let data = if let ConnectionState::Streaming {
                        saved_data,
                        ..
                    } = &stream_conn.state
                    {
                        saved_data.contents().to_vec()
                    } else {
                        unreachable!()
                    };
                    conn.to_send.push_back(
                        crate::protocol::Message::terminal_output(&data),
                    );
                    Ok(())
                } else {
                    Err(Error::InvalidWatchId { id })
                }
            }
            m => Err(crate::error::Error::UnexpectedMessage { message: m }),
        }
    }

    fn handle_streaming_message(
        &mut self,
        conn: &mut Connection<S>,
        message: crate::protocol::Message,
    ) -> Result<()> {
        let (term_info, saved_data) = if let ConnectionState::Streaming {
            term_info,
            saved_data,
            ..
        } = &mut conn.state
        {
            (term_info, saved_data)
        } else {
            unreachable!()
        };

        match message {
            crate::protocol::Message::Heartbeat => {
                conn.to_send
                    .push_back(crate::protocol::Message::heartbeat());
                Ok(())
            }
            crate::protocol::Message::Resize { size } => {
                term_info.size = size;
                Ok(())
            }
            crate::protocol::Message::TerminalOutput { data } => {
                saved_data.append(&data);
                for watch_conn in self.watchers_mut() {
                    match &watch_conn.state {
                        ConnectionState::Watching { watch_id, .. } => {
                            if &conn.id == watch_id {
                                watch_conn.to_send.push_back(
                                    crate::protocol::Message::terminal_output(
                                        &data,
                                    ),
                                );
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                conn.last_activity = std::time::Instant::now();
                Ok(())
            }
            m => Err(crate::error::Error::UnexpectedMessage { message: m }),
        }
    }

    fn handle_watching_message(
        &mut self,
        conn: &mut Connection<S>,
        message: crate::protocol::Message,
    ) -> Result<()> {
        let term_info =
            if let ConnectionState::Watching { term_info, .. } =
                &mut conn.state
            {
                term_info
            } else {
                unreachable!()
            };

        match message {
            crate::protocol::Message::Heartbeat => {
                conn.to_send
                    .push_back(crate::protocol::Message::heartbeat());
                Ok(())
            }
            crate::protocol::Message::Resize { size } => {
                term_info.size = size;
                Ok(())
            }
            m => Err(crate::error::Error::UnexpectedMessage { message: m }),
        }
    }

    fn handle_disconnect(&mut self, conn: &mut Connection<S>) {
        if let Some(username) = conn.state.username() {
            log::info!("{}: disconnect({})", conn.id, username);
        } else {
            log::info!("{}: disconnect", conn.id);
        }

        for watch_conn in self.watchers_mut() {
            if let ConnectionState::Watching { watch_id, .. } =
                &watch_conn.state
            {
                if watch_id == &conn.id {
                    watch_conn.close(Ok(()));
                }
            } else {
                unreachable!()
            }
        }
    }

    fn handle_message(
        &mut self,
        conn: &mut Connection<S>,
        message: crate::protocol::Message,
    ) -> Result<()> {
        if let crate::protocol::Message::TerminalOutput { .. } = message {
            // do nothing, we expect TerminalOutput spam
        } else {
            let username =
                conn.state.username().map(std::string::ToString::to_string);
            if self.rate_limiter.check(username).is_err() {
                let display_name =
                    conn.state.username().unwrap_or("(non-logged-in users)");
                log::info!("{}: ratelimit({})", conn.id, display_name);
                return Err(Error::RateLimited);
            }
        }

        message.log(&conn.id);

        match conn.state {
            ConnectionState::Accepted { .. } => {
                self.handle_accepted_message(conn, message)
            }
            ConnectionState::LoggedIn { .. } => {
                self.handle_logged_in_message(conn, message)
            }
            ConnectionState::Streaming { .. } => {
                self.handle_streaming_message(conn, message)
            }
            ConnectionState::Watching { .. } => {
                self.handle_watching_message(conn, message)
            }
        }
    }

    fn poll_read_connection(
        &mut self,
        conn: &mut Connection<S>,
    ) -> Result<crate::component_future::Poll<()>> {
        match &mut conn.rsock {
            Some(ReadSocket::Connected(..)) => {
                if let Some(ReadSocket::Connected(s)) = conn.rsock.take() {
                    let fut = Box::new(
                        crate::protocol::Message::read_async(s)
                            .timeout(self.read_timeout)
                            .context(crate::error::ReadMessageWithTimeout),
                    );
                    conn.rsock = Some(ReadSocket::Reading(fut));
                } else {
                    unreachable!()
                }
                Ok(crate::component_future::Poll::DidWork)
            }
            Some(ReadSocket::Reading(fut)) => match fut.poll() {
                Ok(futures::Async::Ready((msg, s))) => {
                    let res = self.handle_message(conn, msg);
                    if res.is_err() {
                        conn.close(res);
                    }
                    conn.rsock = Some(ReadSocket::Connected(s));
                    Ok(crate::component_future::Poll::DidWork)
                }
                Ok(futures::Async::NotReady) => {
                    Ok(crate::component_future::Poll::NotReady)
                }
                Err(e) => {
                    if let Error::ReadMessageWithTimeout { source } = e {
                        if source.is_inner() {
                            let source = source.into_inner().unwrap();
                            match source {
                                Error::ReadPacketAsync {
                                    source: ref tokio_err,
                                } => {
                                    if tokio_err.kind()
                                        == tokio::io::ErrorKind::UnexpectedEof
                                    {
                                        Ok(crate::component_future::Poll::Event(()))
                                    } else {
                                        Err(source)
                                    }
                                }
                                Error::EOF => Ok(
                                    crate::component_future::Poll::Event(()),
                                ),
                                _ => Err(source),
                            }
                        } else if source.is_elapsed() {
                            Err(Error::Timeout)
                        } else {
                            let source = source.into_timer().unwrap();
                            Err(Error::TimerReadTimeout { source })
                        }
                    } else {
                        Err(e)
                    }
                }
            },
            _ => Ok(crate::component_future::Poll::NothingToDo),
        }
    }

    fn poll_write_connection(
        &mut self,
        conn: &mut Connection<S>,
    ) -> Result<crate::component_future::Poll<()>> {
        match &mut conn.wsock {
            Some(WriteSocket::Connected(..)) => {
                if let Some(msg) = conn.to_send.pop_front() {
                    if let Some(WriteSocket::Connected(s)) = conn.wsock.take()
                    {
                        let fut = msg
                            .write_async(s)
                            .timeout(self.read_timeout)
                            .context(crate::error::WriteMessageWithTimeout);
                        conn.wsock =
                            Some(WriteSocket::Writing(Box::new(fut)));
                    } else {
                        unreachable!()
                    }
                    Ok(crate::component_future::Poll::DidWork)
                } else if conn.closed {
                    Ok(crate::component_future::Poll::Event(()))
                } else {
                    Ok(crate::component_future::Poll::NothingToDo)
                }
            }
            Some(WriteSocket::Writing(fut)) => match fut.poll() {
                Ok(futures::Async::Ready(s)) => {
                    conn.wsock = Some(WriteSocket::Connected(s));
                    Ok(crate::component_future::Poll::DidWork)
                }
                Ok(futures::Async::NotReady) => {
                    Ok(crate::component_future::Poll::NotReady)
                }
                Err(e) => {
                    if let Error::WriteMessageWithTimeout { source } = e {
                        if source.is_inner() {
                            let source = source.into_inner().unwrap();
                            match source {
                                Error::WritePacketAsync {
                                    source: ref tokio_err,
                                } => {
                                    if tokio_err.kind()
                                        == tokio::io::ErrorKind::UnexpectedEof
                                    {
                                        Ok(crate::component_future::Poll::Event(
                                        (),
                                    ))
                                    } else {
                                        Err(source)
                                    }
                                }
                                Error::EOF => Ok(
                                    crate::component_future::Poll::Event(()),
                                ),
                                _ => Err(source),
                            }
                        } else if source.is_elapsed() {
                            Err(Error::Timeout)
                        } else {
                            let source = source.into_timer().unwrap();
                            Err(Error::TimerReadTimeout { source })
                        }
                    } else {
                        Err(e)
                    }
                }
            },
            _ => Ok(crate::component_future::Poll::NothingToDo),
        }
    }

    fn streamers(&self) -> impl Iterator<Item = &Connection<S>> {
        self.connections.values().filter(|conn| match conn.state {
            ConnectionState::Streaming { .. } => true,
            _ => false,
        })
    }

    fn watchers_mut(&mut self) -> impl Iterator<Item = &mut Connection<S>> {
        self.connections
            .values_mut()
            .filter(|conn| match conn.state {
                ConnectionState::Watching { .. } => true,
                _ => false,
            })
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Server<S>
{
    const POLL_FNS: &'static [&'static dyn for<'a> Fn(
        &'a mut Self,
    ) -> Result<
        crate::component_future::Poll<()>,
    >] = &[
        &Self::poll_new_connections,
        &Self::poll_read,
        &Self::poll_write,
    ];

    fn poll_new_connections(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        match self.sock_stream.poll()? {
            futures::Async::Ready(Some(conn)) => {
                self.connections.insert(conn.id.to_string(), conn);
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::Ready(None) => Err(Error::SocketChannelClosed),
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }

    fn poll_read(&mut self) -> Result<crate::component_future::Poll<()>> {
        let mut did_work = false;
        let mut not_ready = false;

        let keys: Vec<_> = self.connections.keys().cloned().collect();
        for key in keys {
            let mut conn = self.connections.remove(&key).unwrap();
            match self.poll_read_connection(&mut conn) {
                Ok(crate::component_future::Poll::Event(())) => {
                    self.handle_disconnect(&mut conn);
                    continue;
                }
                Ok(crate::component_future::Poll::DidWork) => {
                    did_work = true;
                }
                Ok(crate::component_future::Poll::NotReady) => {
                    not_ready = true;
                }
                Err(e) => {
                    log::error!(
                        "error reading from active connection: {}",
                        e
                    );
                    continue;
                }
                _ => {}
            }
            self.connections.insert(key.to_string(), conn);
        }

        if did_work {
            Ok(crate::component_future::Poll::DidWork)
        } else if not_ready {
            Ok(crate::component_future::Poll::NotReady)
        } else {
            Ok(crate::component_future::Poll::NothingToDo)
        }
    }

    fn poll_write(&mut self) -> Result<crate::component_future::Poll<()>> {
        let mut did_work = false;
        let mut not_ready = false;

        let keys: Vec<_> = self.connections.keys().cloned().collect();
        for key in keys {
            let mut conn = self.connections.remove(&key).unwrap();
            match self.poll_write_connection(&mut conn) {
                Ok(crate::component_future::Poll::Event(())) => {
                    self.handle_disconnect(&mut conn);
                    continue;
                }
                Ok(crate::component_future::Poll::DidWork) => {
                    did_work = true;
                }
                Ok(crate::component_future::Poll::NotReady) => {
                    not_ready = true;
                }
                Err(e) => {
                    log::error!(
                        "error reading from active connection: {}",
                        e
                    );
                    continue;
                }
                _ => {}
            }
            self.connections.insert(key.to_string(), conn);
        }

        if did_work {
            Ok(crate::component_future::Poll::DidWork)
        } else if not_ready {
            Ok(crate::component_future::Poll::NotReady)
        } else {
            Ok(crate::component_future::Poll::NothingToDo)
        }
    }
}

#[must_use = "futures do nothing unless polled"]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    futures::future::Future for Server<S>
{
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        crate::component_future::poll_future(self, Self::POLL_FNS)
    }
}
