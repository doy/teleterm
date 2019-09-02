use snafu::futures01::stream::StreamExt as _;
use snafu::ResultExt as _;
use tokio::prelude::*;

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
    let (mut cast_sock_w, cast_sock_r) = tokio::sync::mpsc::channel(1);
    let cast_addr = "127.0.0.1:8000".parse().context(ParseAddress)?;
    let cast_listener =
        tokio::net::TcpListener::bind(&cast_addr).context(Bind)?;
    let cast_server = cast_listener
        .incoming()
        .map_err(|e| {
            eprintln!("accept failed: {}", e);
        })
        .for_each(move |sock| {
            cast_sock_w.try_send(sock).map_err(|e| {
                eprintln!("sending socket to manager thread failed: {}", e);
            })
        });

    let (mut watch_sock_w, watch_sock_r) = tokio::sync::mpsc::channel(1);
    let watch_addr = "127.0.0.1:8001".parse().context(ParseAddress)?;
    let watch_listener =
        tokio::net::TcpListener::bind(&watch_addr).context(Bind)?;
    let watch_server = watch_listener
        .incoming()
        .map_err(|e| {
            eprintln!("accept failed: {}", e);
        })
        .for_each(move |sock| {
            watch_sock_w.try_send(sock).map_err(|e| {
                eprintln!("sending socket to manager thread failed: {}", e);
            })
        });

    let servers: Vec<
        Box<dyn futures::future::Future<Item = (), Error = ()> + Send>,
    > = vec![Box::new(cast_server), Box::new(watch_server)];

    tokio::run(futures::future::lazy(move || {
        let connection_handler =
            ConnectionHandler::new(cast_sock_r, watch_sock_r)
                .for_each(|_| futures::future::ok(()))
                .map_err(|e| eprintln!("{}", e));
        tokio::spawn(connection_handler);

        futures::future::join_all(servers)
            .map(|_| ())
            .map_err(|_| ())
    }));
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SockType {
    Cast,
    Watch,
}

#[derive(Debug)]
struct SocketMetadata {
    ty: SockType,
    username: Option<String>,
    saved_data: Vec<u8>,
}

#[derive(Debug)]
struct Socket {
    s: tokio::net::tcp::TcpStream,
    meta: SocketMetadata,
}

impl Socket {
    fn cast(s: tokio::net::tcp::TcpStream) -> Self {
        Self {
            s,
            meta: SocketMetadata {
                ty: SockType::Cast,
                username: None,
                saved_data: vec![],
            },
        }
    }

    fn watch(s: tokio::net::tcp::TcpStream) -> Self {
        Self {
            s,
            meta: SocketMetadata {
                ty: SockType::Watch,
                username: None,
                saved_data: vec![],
            },
        }
    }
}

struct ConnectionHandler {
    socks: Vec<Socket>,

    sock_stream:
        Box<dyn futures::stream::Stream<Item = Socket, Error = Error> + Send>,
    in_progress_reads: Vec<
        Box<
            dyn futures::future::Future<
                    Item = (crate::protocol::Message, Socket),
                    Error = Error,
                > + Send,
        >,
    >,
}

impl ConnectionHandler {
    fn new(
        cast_sock_r: tokio::sync::mpsc::Receiver<tokio::net::tcp::TcpStream>,
        watch_sock_r: tokio::sync::mpsc::Receiver<tokio::net::tcp::TcpStream>,
    ) -> Self {
        let sock_stream = cast_sock_r
            .map(Socket::cast)
            .select(watch_sock_r.map(Socket::watch))
            .context(SocketChannelReceive);
        Self {
            socks: vec![],

            sock_stream: Box::new(sock_stream),
            in_progress_reads: vec![],
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
            match self.socks[i].s.poll_read_ready(mio::Ready::readable()) {
                Ok(futures::Async::Ready(_)) => {
                    let Socket { s, meta } = self.socks.swap_remove(i);
                    let read_fut = crate::protocol::Message::read_async(s)
                        .map_err(|e| Error::ReadMessage { source: e })
                        .map(move |(msg, s)| (msg, Socket { s, meta }));
                    self.in_progress_reads.push(Box::new(read_fut));
                    did_work = true;
                }
                Ok(futures::Async::NotReady) => {
                    i += 1;
                }
                Err(e) => return Err(e).context(PollReadReady),
            }
        }

        Ok(did_work)
    }

    fn poll_read(&mut self) -> Result<bool> {
        let mut did_work = false;

        let mut i = 0;
        while i < self.in_progress_reads.len() {
            match self.in_progress_reads[i].poll() {
                Ok(futures::Async::Ready((msg, mut sock))) => {
                    self.handle_message(&mut sock, msg)?;
                    self.in_progress_reads.swap_remove(i);
                    self.socks.push(sock);
                    did_work = true;
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
                            self.in_progress_reads.swap_remove(i);
                        } else {
                            return Err(e);
                        }
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        Ok(did_work)
    }

    fn handle_message(
        &self,
        sock: &mut Socket,
        message: crate::protocol::Message,
    ) -> Result<()> {
        match sock.meta.ty {
            SockType::Cast => self.handle_cast_message(sock, message),
            SockType::Watch => self.handle_watch_message(sock, message),
        }
    }

    fn handle_cast_message(
        &self,
        sock: &mut Socket,
        message: crate::protocol::Message,
    ) -> Result<()> {
        match message {
            crate::protocol::Message::StartCasting { username } => {
                println!("got a cast connection from {}", username);
                sock.meta.username = Some(username);
                Ok(())
            }
            crate::protocol::Message::Heartbeat => {
                println!(
                    "got a heartbeat from {}",
                    sock.meta.username.as_ref().unwrap()
                );
                Ok(())
            }
            crate::protocol::Message::TerminalOutput { data } => {
                sock.meta.saved_data.extend_from_slice(&data);
                // XXX truncate data before most recent screen clear
                for sock in self.socks.iter() {
                    if sock.meta.ty == SockType::Watch {
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
        &self,
        sock: &mut Socket,
        message: crate::protocol::Message,
    ) -> Result<()> {
        match message {
            crate::protocol::Message::StartWatching { username } => {
                println!("got a watch connection from {}", username);
                sock.meta.username = Some(username);
                Ok(())
            }
            crate::protocol::Message::ListSessions => {
                // XXX send a Sessions reply back
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

impl futures::stream::Stream for ConnectionHandler {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        loop {
            let mut did_work = false;

            did_work |= self.poll_new_connections()?;
            did_work |= self.poll_readable()?;
            did_work |= self.poll_read()?;

            if !did_work {
                break;
            }
        }

        Ok(futures::Async::NotReady)
    }
}
