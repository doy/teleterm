use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::futures01::stream::StreamExt as _;
use snafu::futures01::FutureExt as _;
use snafu::ResultExt as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to parse address: {}", source))]
    ParseAddress { source: std::net::AddrParseError },

    #[snafu(display("failed to bind: {}", source))]
    Bind { source: tokio::io::Error },

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

    #[snafu(display("failed to poll for readability: {}", source))]
    PollReadReady { source: tokio::io::Error },

    #[snafu(display("failed to read message: {}", source))]
    ReadMessage { source: crate::protocol::Error },

    #[snafu(display("failed to write message: {}", source))]
    WriteMessage { source: crate::protocol::Error },

    #[snafu(display("unexpected message: {:?}", message))]
    UnexpectedMessage { message: crate::protocol::Message },
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Run a termcast server")
}

pub fn run<'a>(_matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    run_impl().context(super::Server)
}

fn run_impl() -> Result<()> {
    let (mut sock_w, sock_r) = tokio::sync::mpsc::channel(1);
    let addr = "127.0.0.1:8000".parse().context(ParseAddress)?;
    let listener = tokio::net::TcpListener::bind(&addr).context(Bind)?;
    let server = listener
        .incoming()
        .map_err(|e| {
            eprintln!("accept failed: {}", e);
        })
        .for_each(move |sock| {
            sock_w.try_send(sock).map_err(|e| {
                eprintln!("sending socket to manager thread failed: {}", e);
            })
        });

    tokio::run(futures::future::lazy(move || {
        let connection_handler =
            ConnectionHandler::new(sock_r).map_err(|e| eprintln!("{}", e));
        tokio::spawn(connection_handler);

        server.map(|_| ()).map_err(|_| ())
    }));
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SockType {
    Unknown,
    Cast,
    Watch,
}

#[derive(Debug)]
struct SocketMetadata {
    ty: SockType,
    id: String,
    username: Option<String>,
    term_type: Option<String>,
    saved_data: crate::term::Buffer,
}

impl SocketMetadata {
    fn new(ty: SockType) -> Self {
        Self {
            ty,
            id: format!("{}", uuid::Uuid::new_v4()),
            username: None,
            term_type: None,
            saved_data: crate::term::Buffer::new(),
        }
    }
}

type AcceptStream =
    Box<dyn futures::stream::Stream<Item = Socket, Error = Error> + Send>;
type ReadFuture = Box<
    dyn futures::future::Future<
            Item = (crate::protocol::Message, tokio::net::tcp::TcpStream),
            Error = Error,
        > + Send,
>;
type WriteFuture = Box<
    dyn futures::future::Future<
            Item = tokio::net::tcp::TcpStream,
            Error = Error,
        > + Send,
>;
type WriteFutureFactory =
    Box<dyn FnOnce(tokio::net::tcp::TcpStream) -> WriteFuture>;

enum Socket {
    Listening {
        s: tokio::net::tcp::TcpStream,
        meta: SocketMetadata,
    },
    Reading {
        future: ReadFuture,
        meta: SocketMetadata,
    },
    Writing {
        future: WriteFuture,
        meta: SocketMetadata,
    },
}

impl Socket {
    fn new(s: tokio::net::tcp::TcpStream) -> Self {
        Self::Listening {
            s,
            meta: SocketMetadata::new(SockType::Unknown),
        }
    }

    fn meta(&self) -> &SocketMetadata {
        match self {
            Socket::Listening { meta, .. } => meta,
            Socket::Reading { meta, .. } => meta,
            Socket::Writing { meta, .. } => meta,
        }
    }
}

struct ConnectionHandler {
    sock_stream: AcceptStream,

    socks: Vec<Socket>,
}

impl ConnectionHandler {
    fn new(
        sock_r: tokio::sync::mpsc::Receiver<tokio::net::tcp::TcpStream>,
    ) -> Self {
        let sock_stream =
            sock_r.map(Socket::new).context(SocketChannelReceive);
        Self {
            sock_stream: Box::new(sock_stream),

            socks: vec![],
        }
    }

    fn poll_new_connections(&mut self) -> Result<bool> {
        match self.sock_stream.poll() {
            Ok(futures::Async::Ready(Some(s))) => {
                self.socks.push(s);
                Ok(true)
            }
            Ok(futures::Async::Ready(None)) => {
                Err(Error::SocketChannelClosed)
            }
            Ok(futures::Async::NotReady) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn poll_readable(&mut self) -> Result<bool> {
        let mut did_work = false;

        let mut i = 0;
        while i < self.socks.len() {
            if let Socket::Listening { s, .. } = &self.socks[i] {
                match s.poll_read_ready(mio::Ready::readable()) {
                    Ok(futures::Async::Ready(_)) => {
                        if let Socket::Listening { s, meta } =
                            self.socks.swap_remove(i)
                        {
                            let future = Box::new(
                                crate::protocol::Message::read_async(s)
                                    .context(ReadMessage),
                            );
                            self.socks.push(Socket::Reading { future, meta });
                            did_work = true;
                        } else {
                            unreachable!()
                        }
                    }
                    Ok(futures::Async::NotReady) => {
                        i += 1;
                    }
                    Err(e) => return Err(e).context(PollReadReady),
                }
            } else {
                i += 1;
            }
        }

        Ok(did_work)
    }

    fn poll_read(&mut self) -> Result<bool> {
        let mut did_work = false;

        let mut i = 0;
        while i < self.socks.len() {
            if let Socket::Reading { future, .. } = &mut self.socks[i] {
                match future.poll() {
                    Ok(futures::Async::Ready((msg, s))) => {
                        if let Socket::Reading { mut meta, .. } =
                            self.socks.swap_remove(i)
                        {
                            if let Some(future) =
                                self.handle_message(&mut meta, msg)?
                            {
                                self.socks.push(Socket::Writing {
                                    future: future(s),
                                    meta,
                                });
                            } else {
                                self.socks
                                    .push(Socket::Listening { s, meta });
                            }
                            did_work = true;
                        } else {
                            unreachable!()
                        }
                    }
                    Ok(futures::Async::NotReady) => {
                        i += 1;
                    }
                    Err(e) => {
                        if let Error::ReadMessage {
                            source:
                                crate::protocol::Error::ReadAsync {
                                    source: ref tokio_err,
                                },
                        } = e
                        {
                            if tokio_err.kind()
                                == tokio::io::ErrorKind::UnexpectedEof
                            {
                                println!("disconnect");
                                self.socks.swap_remove(i);
                            } else {
                                return Err(e);
                            }
                        } else {
                            return Err(e);
                        }
                    }
                }
            } else {
                i += 1;
            }
        }

        Ok(did_work)
    }

    fn poll_write(&mut self) -> Result<bool> {
        let mut did_work = false;

        let mut i = 0;
        while i < self.socks.len() {
            if let Socket::Writing { future, .. } = &mut self.socks[i] {
                match future.poll() {
                    Ok(futures::Async::Ready(s)) => {
                        if let Socket::Writing { meta, .. } =
                            self.socks.swap_remove(i)
                        {
                            let sock = Socket::Listening { s, meta };
                            self.socks.push(sock);
                            did_work = true;
                        } else {
                            unreachable!()
                        }
                    }
                    Ok(futures::Async::NotReady) => {
                        i += 1;
                    }
                    Err(e) => {
                        if let Error::ReadMessage {
                            source:
                                crate::protocol::Error::WriteAsync {
                                    source: ref tokio_err,
                                },
                        } = e
                        {
                            if tokio_err.kind()
                                == tokio::io::ErrorKind::UnexpectedEof
                            {
                                println!("disconnect");
                                self.socks.swap_remove(i);
                            } else {
                                return Err(e);
                            }
                        } else {
                            return Err(e);
                        }
                    }
                }
            } else {
                i += 1;
            }
        }

        Ok(did_work)
    }

    fn handle_message(
        &self,
        meta: &mut SocketMetadata,
        message: crate::protocol::Message,
    ) -> Result<Option<WriteFutureFactory>> {
        match meta.ty {
            SockType::Unknown => self.handle_login_message(meta, message),
            SockType::Cast => self.handle_cast_message(meta, message),
            SockType::Watch => self.handle_watch_message(meta, message),
        }
    }

    fn handle_login_message(
        &self,
        meta: &mut SocketMetadata,
        message: crate::protocol::Message,
    ) -> Result<Option<WriteFutureFactory>> {
        match message {
            crate::protocol::Message::StartCasting {
                username,
                term_type,
                ..
            } => {
                println!("got a cast connection from {}", username);
                meta.ty = SockType::Cast;
                meta.username = Some(username);
                meta.term_type = Some(term_type);
                Ok(None)
            }
            crate::protocol::Message::StartWatching {
                username,
                term_type,
                ..
            } => {
                println!("got a watch connection from {}", username);
                meta.ty = SockType::Watch;
                meta.username = Some(username);
                meta.term_type = Some(term_type);
                Ok(None)
            }
            m => Err(Error::UnexpectedMessage { message: m }),
        }
    }

    fn handle_cast_message(
        &self,
        meta: &mut SocketMetadata,
        message: crate::protocol::Message,
    ) -> Result<Option<WriteFutureFactory>> {
        match message {
            crate::protocol::Message::StartCasting { username, .. } => {
                println!("got a cast connection from {}", username);
                meta.username = Some(username);
                Ok(None)
            }
            crate::protocol::Message::Heartbeat => {
                println!(
                    "got a heartbeat from {}",
                    meta.username.as_ref().unwrap()
                );
                let msg = crate::protocol::Message::heartbeat();
                Ok(Some(Box::new(move |s| {
                    Box::new(msg.write_async(s).context(WriteMessage))
                })))
            }
            crate::protocol::Message::TerminalOutput { data } => {
                println!("got {} bytes of cast data", data.len());
                meta.saved_data.append(data);
                for sock in self.socks.iter() {
                    if sock.meta().ty == SockType::Watch {
                        // XXX test if it's watching the correct session
                        // XXX async-send a TerminalOutput message back
                        // (probably need another vec of in-progress async
                        // sends)
                    }
                }
                Ok(None)
            }
            m => Err(Error::UnexpectedMessage { message: m }),
        }
    }

    fn handle_watch_message(
        &self,
        meta: &mut SocketMetadata,
        message: crate::protocol::Message,
    ) -> Result<Option<WriteFutureFactory>> {
        match message {
            crate::protocol::Message::ListSessions => {
                let mut ids = vec![];
                for sock in self.socks.iter() {
                    if sock.meta().ty == SockType::Cast {
                        ids.push(sock.meta().id.clone());
                    }
                }
                let msg = crate::protocol::Message::sessions(&ids);
                Ok(Some(Box::new(move |s| {
                    Box::new(msg.write_async(s).context(WriteMessage))
                })))
            }
            crate::protocol::Message::WatchSession { id } => {
                let _id = id;
                // XXX start by sending a TerminalOutput message containing
                // the saved_data for the given session, then register for
                // further updates
                Ok(None)
            }
            m => Err(Error::UnexpectedMessage { message: m }),
        }
    }
}

impl futures::future::Future for ConnectionHandler {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        loop {
            let mut did_work = false;

            did_work |= self.poll_new_connections()?;
            did_work |= self.poll_readable()?;
            did_work |= self.poll_read()?;
            did_work |= self.poll_write()?;

            if !did_work {
                break;
            }
        }

        Ok(futures::Async::NotReady)
    }
}
