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
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SockType {
    Unknown,
    Cast,
    Watch,
}

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

struct Connection {
    rsock: Option<ReadSocket>,
    wsock: Option<WriteSocket>,

    ty: SockType,
    id: String,
    username: Option<String>,
    term_type: Option<String>,
    saved_data: crate::term::Buffer,

    to_send: std::collections::VecDeque<crate::protocol::Message>,
}

impl Connection {
    fn new(s: tokio::net::tcp::TcpStream) -> Self {
        let (rs, ws) = s.split();
        Self {
            rsock: Some(ReadSocket::Connected(
                crate::protocol::FramedReader::new(rs),
            )),
            wsock: Some(WriteSocket::Connected(
                crate::protocol::FramedWriter::new(ws),
            )),

            ty: SockType::Unknown,
            id: format!("{}", uuid::Uuid::new_v4()),
            username: None,
            term_type: None,
            saved_data: crate::term::Buffer::new(),

            to_send: std::collections::VecDeque::new(),
        }
    }
}

pub struct ConnectionHandler {
    sock_stream: Box<
        dyn futures::stream::Stream<Item = Connection, Error = Error> + Send,
    >,
    connections: Vec<Connection>,
}

impl ConnectionHandler {
    const POLL_FNS: &'static [&'static dyn for<'a> Fn(
        &'a mut Self,
    ) -> Result<
        crate::component_future::Poll<()>,
    >] = &[
        &Self::poll_new_connections,
        &Self::poll_read,
        &Self::poll_write,
    ];

    pub fn new(
        sock_r: tokio::sync::mpsc::Receiver<tokio::net::tcp::TcpStream>,
    ) -> Self {
        let sock_stream =
            sock_r.map(Connection::new).context(SocketChannelReceive);
        Self {
            sock_stream: Box::new(sock_stream),
            connections: vec![],
        }
    }

    fn poll_new_connections(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        match self.sock_stream.poll() {
            Ok(futures::Async::Ready(Some(conn))) => {
                self.connections.push(conn);
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

        let mut i = 0;
        while i < self.connections.len() {
            match &mut self.connections[i].rsock {
                Some(ReadSocket::Connected(..)) => {
                    if let Some(ReadSocket::Connected(s)) =
                        self.connections[i].rsock.take()
                    {
                        let fut = Box::new(
                            crate::protocol::Message::read_async(s)
                                .context(ReadMessage),
                        );
                        self.connections[i].rsock =
                            Some(ReadSocket::Reading(fut));
                    } else {
                        unreachable!()
                    }
                    did_work = true;
                }
                Some(ReadSocket::Reading(fut)) => match fut.poll() {
                    Ok(futures::Async::Ready((msg, s))) => {
                        self.handle_message(i, msg)?;
                        self.connections[i].rsock =
                            Some(ReadSocket::Connected(s));
                        did_work = true;
                    }
                    Ok(futures::Async::NotReady) => {
                        i += 1;
                        not_ready = true;
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
                                        println!("disconnect");
                                        self.connections.swap_remove(i);
                                    } else {
                                        return Err(e);
                                    }
                                }
                                crate::protocol::Error::EOF => {
                                    println!("disconnect");
                                    self.connections.swap_remove(i);
                                }
                                _ => return Err(e),
                            }
                        } else {
                            return Err(e);
                        }
                    }
                },
                _ => i += 1,
            }
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

        let mut i = 0;
        while i < self.connections.len() {
            match &mut self.connections[i].wsock {
                Some(WriteSocket::Connected(..)) => {
                    if let Some(msg) = self.connections[i].to_send.pop_front()
                    {
                        if let Some(WriteSocket::Connected(s)) =
                            self.connections[i].wsock.take()
                        {
                            let fut =
                                msg.write_async(s).context(WriteMessage);
                            self.connections[i].wsock =
                                Some(WriteSocket::Writing(Box::new(fut)));
                        } else {
                            unreachable!()
                        }
                        did_work = true;
                    } else {
                        i += 1;
                    }
                }
                Some(WriteSocket::Writing(fut)) => match fut.poll() {
                    Ok(futures::Async::Ready(s)) => {
                        self.connections[i].wsock =
                            Some(WriteSocket::Connected(s));
                        did_work = true;
                    }
                    Ok(futures::Async::NotReady) => {
                        i += 1;
                        not_ready = true;
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
                                        println!("disconnect");
                                        self.connections.swap_remove(i);
                                    } else {
                                        return Err(e);
                                    }
                                }
                                crate::protocol::Error::EOF => {
                                    println!("disconnect");
                                    self.connections.swap_remove(i);
                                }
                                _ => return Err(e),
                            }
                        } else {
                            return Err(e);
                        }
                    }
                },
                _ => i += 1,
            }
        }

        if did_work {
            Ok(crate::component_future::Poll::DidWork)
        } else if not_ready {
            Ok(crate::component_future::Poll::NotReady)
        } else {
            Ok(crate::component_future::Poll::NothingToDo)
        }
    }

    fn handle_message(
        &mut self,
        i: usize,
        message: crate::protocol::Message,
    ) -> Result<()> {
        match self.connections[i].ty {
            SockType::Unknown => self.handle_login_message(i, message),
            SockType::Cast => self.handle_cast_message(i, message),
            SockType::Watch => self.handle_watch_message(i, message),
        }
    }

    fn handle_login_message(
        &mut self,
        i: usize,
        message: crate::protocol::Message,
    ) -> Result<()> {
        let conn = &mut self.connections[i];
        match message {
            crate::protocol::Message::StartCasting {
                username,
                term_type,
                ..
            } => {
                println!("got a cast connection from {}", username);
                conn.ty = SockType::Cast;
                conn.username = Some(username);
                conn.term_type = Some(term_type);
                Ok(())
            }
            crate::protocol::Message::StartWatching {
                username,
                term_type,
                ..
            } => {
                println!("got a watch connection from {}", username);
                conn.ty = SockType::Watch;
                conn.username = Some(username);
                conn.term_type = Some(term_type);
                Ok(())
            }
            m => Err(Error::UnexpectedMessage { message: m }),
        }
    }

    fn handle_cast_message(
        &mut self,
        i: usize,
        message: crate::protocol::Message,
    ) -> Result<()> {
        let conn = &mut self.connections[i];
        match message {
            crate::protocol::Message::Heartbeat => {
                println!(
                    "got a heartbeat from {}",
                    conn.username.as_ref().unwrap()
                );
                conn.to_send
                    .push_back(crate::protocol::Message::heartbeat());
                Ok(())
            }
            crate::protocol::Message::TerminalOutput { data } => {
                println!("got {} bytes of cast data", data.len());
                conn.saved_data.append(data);
                for conn in self.connections.iter() {
                    if conn.ty == SockType::Watch {
                        // XXX test if it's watching the correct session
                        // XXX async-send a TerminalOutput message back
                        // (probably need another vec of in-progress async
                        // sends)
                    }
                }
                Ok(())
            }
            m => Err(Error::UnexpectedMessage { message: m }),
        }
    }

    fn handle_watch_message(
        &mut self,
        i: usize,
        message: crate::protocol::Message,
    ) -> Result<()> {
        match message {
            crate::protocol::Message::ListSessions => {
                let mut ids = vec![];
                for caster in
                    self.connections.iter().filter(|c| c.ty == SockType::Cast)
                {
                    ids.push(caster.id.clone());
                }
                let conn = &mut self.connections[i];
                conn.to_send
                    .push_back(crate::protocol::Message::sessions(&ids));
                Ok(())
            }
            crate::protocol::Message::WatchSession { id } => {
                let _id = id;
                // XXX start by sending a TerminalOutput message containing
                // the saved_data for the given session, then register for
                // further updates
                Ok(())
            }
            m => Err(Error::UnexpectedMessage { message: m }),
        }
    }
}

impl futures::future::Future for ConnectionHandler {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        crate::component_future::poll_component_future(self, Self::POLL_FNS)
    }
}
