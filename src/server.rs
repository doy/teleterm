use futures::stream::Stream as _;
use snafu::futures01::stream::StreamExt as _;
use snafu::futures01::FutureExt as _;
use tokio::io::AsyncRead as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display(
        "failed to receive new socket over channel: {}",
        source
    ))]
    SocketChannelReceive {
        source: tokio::sync::mpsc::error::RecvError,
    },

    #[snafu(display(
        "failed to receive new socket over channel: channel closed"
    ))]
    SocketChannelClosed,

    #[snafu(display("failed to read message: {}", source))]
    ReadMessage { source: crate::protocol::Error },

    #[snafu(display("failed to write message: {}", source))]
    WriteMessage { source: crate::protocol::Error },

    #[snafu(display("unexpected message: {:?}", message))]
    UnexpectedMessage { message: crate::protocol::Message },

    #[snafu(display("unauthenticated message: {:?}", message))]
    UnauthenticatedMessage { message: crate::protocol::Message },

    #[snafu(display("invalid watch id: {}", id))]
    InvalidWatchId { id: String },
}

pub type Result<T> = std::result::Result<T, Error>;

enum ReadSocket {
    Connected(crate::protocol::FramedReader),
    Reading(
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
    Connected(crate::protocol::FramedWriter),
    Writing(
        Box<
            dyn futures::future::Future<
                    Item = crate::protocol::FramedWriter,
                    Error = Error,
                > + Send,
        >,
    ),
}

#[derive(Debug, Clone)]
struct TerminalInfo {
    term: String,
    size: (u32, u32),
}

// XXX https://github.com/rust-lang/rust/issues/64362
#[allow(dead_code)]
enum ConnectionState {
    Accepted,
    LoggedIn {
        username: String,
        term_info: TerminalInfo,
    },
    Casting {
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

    fn login(
        &self,
        username: &str,
        term_type: &str,
        size: (u32, u32),
    ) -> Self {
        match self {
            Self::Accepted => Self::LoggedIn {
                username: username.to_string(),
                term_info: TerminalInfo {
                    term: term_type.to_string(),
                    size,
                },
            },
            _ => unreachable!(),
        }
    }

    fn cast(&self) -> Self {
        match self {
            Self::LoggedIn {
                username,
                term_info,
            } => Self::Casting {
                username: username.clone(),
                term_info: term_info.clone(),
                saved_data: crate::term::Buffer::new(),
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

struct Connection {
    id: String,
    rsock: Option<ReadSocket>,
    wsock: Option<WriteSocket>,
    to_send: std::collections::VecDeque<crate::protocol::Message>,
    closed: bool,
    state: ConnectionState,
}

impl Connection {
    fn new(s: tokio::net::tcp::TcpStream) -> Self {
        let (rs, ws) = s.split();
        Self {
            id: format!("{}", uuid::Uuid::new_v4()),
            rsock: Some(ReadSocket::Connected(
                crate::protocol::FramedReader::new(rs),
            )),
            wsock: Some(WriteSocket::Connected(
                crate::protocol::FramedWriter::new(ws),
            )),
            to_send: std::collections::VecDeque::new(),
            closed: false,
            state: ConnectionState::new(),
        }
    }

    fn session(&self) -> Option<crate::protocol::Session> {
        let (username, term_info) = match &self.state {
            ConnectionState::Accepted => return None,
            ConnectionState::LoggedIn {
                username,
                term_info,
            } => (username, term_info),
            ConnectionState::Casting {
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
        Some(crate::protocol::Session {
            id: self.id.clone(),
            username: username.clone(),
            term_type: term_info.term.clone(),
            size: term_info.size,
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

pub struct Server {
    sock_stream: Box<
        dyn futures::stream::Stream<Item = Connection, Error = Error> + Send,
    >,
    connections: std::collections::HashMap<String, Connection>,
}

impl Server {
    pub fn new(
        sock_r: tokio::sync::mpsc::Receiver<tokio::net::tcp::TcpStream>,
    ) -> Self {
        let sock_stream =
            sock_r.map(Connection::new).context(SocketChannelReceive);
        Self {
            sock_stream: Box::new(sock_stream),
            connections: std::collections::HashMap::new(),
        }
    }

    fn handle_message(
        &mut self,
        conn: &mut Connection,
        message: crate::protocol::Message,
    ) -> Result<()> {
        match conn.state {
            ConnectionState::Accepted { .. } => {
                self.handle_login_message(conn, message)
            }
            ConnectionState::LoggedIn { .. } => {
                self.handle_other_message(conn, message)
            }
            ConnectionState::Casting { .. } => {
                self.handle_cast_message(conn, message)
            }
            ConnectionState::Watching { .. } => {
                self.handle_watch_message(conn, message)
            }
        }
    }

    fn handle_login_message(
        &mut self,
        conn: &mut Connection,
        message: crate::protocol::Message,
    ) -> Result<()> {
        match message {
            crate::protocol::Message::Login {
                username,
                term_type,
                size,
                ..
            } => {
                println!("got a connection from {}", username);
                conn.state = conn.state.login(&username, &term_type, size);
                Ok(())
            }
            m => Err(Error::UnauthenticatedMessage { message: m }),
        }
    }

    fn handle_cast_message(
        &mut self,
        conn: &mut Connection,
        message: crate::protocol::Message,
    ) -> Result<()> {
        let (username, saved_data) = if let ConnectionState::Casting {
            username,
            saved_data,
            ..
        } = &mut conn.state
        {
            (username, saved_data)
        } else {
            unreachable!()
        };

        match message {
            crate::protocol::Message::Heartbeat => {
                println!("got a heartbeat from {}", username);
                conn.to_send
                    .push_back(crate::protocol::Message::heartbeat());
                Ok(())
            }
            crate::protocol::Message::TerminalOutput { data } => {
                println!("got {} bytes of cast data", data.len());
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
                Ok(())
            }
            m => Err(Error::UnexpectedMessage { message: m }),
        }
    }

    fn handle_watch_message(
        &mut self,
        conn: &mut Connection,
        message: crate::protocol::Message,
    ) -> Result<()> {
        let username =
            if let ConnectionState::Watching { username, .. } = &conn.state {
                username
            } else {
                unreachable!()
            };
        match message {
            crate::protocol::Message::Heartbeat => {
                println!("got a heartbeat from {}", username);
                conn.to_send
                    .push_back(crate::protocol::Message::heartbeat());
                Ok(())
            }
            m => Err(Error::UnexpectedMessage { message: m }),
        }
    }

    fn handle_other_message(
        &mut self,
        conn: &mut Connection,
        message: crate::protocol::Message,
    ) -> Result<()> {
        match message {
            crate::protocol::Message::ListSessions => {
                let sessions: Vec<_> =
                    self.casters().flat_map(Connection::session).collect();
                conn.to_send
                    .push_back(crate::protocol::Message::sessions(&sessions));
                Ok(())
            }
            crate::protocol::Message::StartCasting => {
                conn.state = conn.state.cast();
                Ok(())
            }
            crate::protocol::Message::StartWatching { id } => {
                if let Some(cast_conn) = self.connections.get(&id) {
                    conn.state = conn.state.watch(&id);
                    let data = if let ConnectionState::Casting {
                        saved_data,
                        ..
                    } = &cast_conn.state
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
            m => Err(Error::UnexpectedMessage { message: m }),
        }
    }

    fn handle_disconnect(&mut self, conn: &mut Connection) {
        println!("disconnect");

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

    fn poll_read_connection(
        &mut self,
        conn: &mut Connection,
    ) -> Result<crate::component_future::Poll<()>> {
        match &mut conn.rsock {
            Some(ReadSocket::Connected(..)) => {
                if let Some(ReadSocket::Connected(s)) = conn.rsock.take() {
                    let fut = Box::new(
                        crate::protocol::Message::read_async(s)
                            .context(ReadMessage),
                    );
                    conn.rsock = Some(ReadSocket::Reading(fut));
                } else {
                    unreachable!()
                }
                Ok(crate::component_future::Poll::DidWork)
            }
            Some(ReadSocket::Reading(fut)) => {
                match fut.poll() {
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
                        if let Error::ReadMessage { ref source } = e {
                            match source {
                                crate::protocol::Error::ReadAsync {
                                    source: ref tokio_err,
                                } => {
                                    if tokio_err.kind()
                                        == tokio::io::ErrorKind::UnexpectedEof
                                    {
                                        Ok(crate::component_future::Poll::Event(()))
                                    } else {
                                        Err(e)
                                    }
                                }
                                crate::protocol::Error::EOF => Ok(
                                    crate::component_future::Poll::Event(()),
                                ),
                                _ => Err(e),
                            }
                        } else {
                            Err(e)
                        }
                    }
                }
            }
            _ => Ok(crate::component_future::Poll::NothingToDo),
        }
    }

    fn poll_write_connection(
        &mut self,
        conn: &mut Connection,
    ) -> Result<crate::component_future::Poll<()>> {
        match &mut conn.wsock {
            Some(WriteSocket::Connected(..)) => {
                if let Some(msg) = conn.to_send.pop_front() {
                    if let Some(WriteSocket::Connected(s)) = conn.wsock.take()
                    {
                        let fut = msg.write_async(s).context(WriteMessage);
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
            Some(WriteSocket::Writing(fut)) => {
                match fut.poll() {
                    Ok(futures::Async::Ready(s)) => {
                        conn.wsock = Some(WriteSocket::Connected(s));
                        Ok(crate::component_future::Poll::DidWork)
                    }
                    Ok(futures::Async::NotReady) => {
                        Ok(crate::component_future::Poll::NotReady)
                    }
                    Err(e) => {
                        if let Error::WriteMessage { ref source } = e {
                            match source {
                                crate::protocol::Error::WriteAsync {
                                    source: ref tokio_err,
                                } => {
                                    if tokio_err.kind()
                                        == tokio::io::ErrorKind::UnexpectedEof
                                    {
                                        Ok(crate::component_future::Poll::Event(()))
                                    } else {
                                        Err(e)
                                    }
                                }
                                crate::protocol::Error::EOF => Ok(
                                    crate::component_future::Poll::Event(()),
                                ),
                                _ => Err(e),
                            }
                        } else {
                            Err(e)
                        }
                    }
                }
            }
            _ => Ok(crate::component_future::Poll::NothingToDo),
        }
    }

    fn casters(&self) -> impl Iterator<Item = &Connection> {
        self.connections.values().filter(|conn| match conn.state {
            ConnectionState::Casting { .. } => true,
            _ => false,
        })
    }

    fn watchers_mut(&mut self) -> impl Iterator<Item = &mut Connection> {
        self.connections
            .values_mut()
            .filter(|conn| match conn.state {
                ConnectionState::Watching { .. } => true,
                _ => false,
            })
    }
}

impl Server {
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
        match self.sock_stream.poll() {
            Ok(futures::Async::Ready(Some(conn))) => {
                self.connections.insert(conn.id.to_string(), conn);
                Ok(crate::component_future::Poll::DidWork)
            }
            Ok(futures::Async::Ready(None)) => {
                Err(Error::SocketChannelClosed)
            }
            Ok(futures::Async::NotReady) => {
                Ok(crate::component_future::Poll::NotReady)
            }
            Err(e) => Err(e),
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
                    println!("error reading from active connection: {}", e);
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
                    println!("error reading from active connection: {}", e);
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
impl futures::future::Future for Server {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        crate::component_future::poll_future(self, Self::POLL_FNS)
    }
}
