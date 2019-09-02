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

enum SockType {
    Cast,
    Watch,
}

struct ConnectionHandler {
    cast_socks: Vec<tokio::net::tcp::TcpStream>,
    watch_socks: Vec<tokio::net::tcp::TcpStream>,

    sock_stream: Box<
        dyn futures::stream::Stream<
                Item = (SockType, tokio::net::tcp::TcpStream),
                Error = tokio::sync::mpsc::error::RecvError,
            > + Send,
    >,
    in_progress_cast_reads: Vec<
        Box<
            dyn futures::future::Future<
                    Item = tokio::net::tcp::TcpStream,
                    Error = Error,
                > + Send,
        >,
    >,
    in_progress_watch_reads: Vec<
        Box<
            dyn futures::future::Future<
                    Item = tokio::net::tcp::TcpStream,
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
            .map(|s| (SockType::Cast, s))
            .select(watch_sock_r.map(|s| (SockType::Watch, s)));
        Self {
            cast_socks: vec![],
            watch_socks: vec![],

            sock_stream: Box::new(sock_stream),
            in_progress_cast_reads: vec![],
            in_progress_watch_reads: vec![],
        }
    }

    fn poll_new_connections(&mut self) -> Result<bool> {
        match self.sock_stream.poll() {
            Ok(futures::Async::Ready(Some((sock_ty, sock)))) => {
                match sock_ty {
                    SockType::Cast => {
                        self.cast_socks.push(sock);
                    }
                    SockType::Watch => {
                        self.watch_socks.push(sock);
                    }
                }
                Ok(true)
            }
            Ok(futures::Async::Ready(None)) => {
                Err(Error::SocketChannelClosed)
            }
            Ok(futures::Async::NotReady) => Ok(false),
            Err(e) => Err(e).context(SocketChannelReceive),
        }
    }

    fn poll_cast_readable(&mut self) -> Result<bool> {
        let mut did_work = false;

        let mut i = 0;
        while i < self.cast_socks.len() {
            match self.cast_socks[i].poll_read_ready(mio::Ready::readable()) {
                Ok(futures::Async::Ready(_)) => {
                    let s = self.cast_socks.swap_remove(i);
                    let read_fut = crate::protocol::Message::read_async(s)
                        .map_err(|e| Error::ReadMessage { source: e })
                        .and_then(|(msg, s)| {
                            match msg {
                                crate::protocol::Message::StartCasting {
                                    username,
                                } => {
                                    println!(
                                        "got a cast connection from {}",
                                        username
                                    );
                                }
                                m => {
                                    return Err(Error::UnexpectedMessage {
                                        message: m,
                                    })
                                }
                            }
                            Ok(s)
                        });
                    self.in_progress_cast_reads.push(Box::new(read_fut));
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

    fn poll_watch_readable(&mut self) -> Result<bool> {
        let mut did_work = false;

        let mut i = 0;
        while i < self.watch_socks.len() {
            match self.watch_socks[i].poll_read_ready(mio::Ready::readable())
            {
                Ok(futures::Async::Ready(_)) => {
                    let s = self.watch_socks.swap_remove(i);
                    let read_fut = crate::protocol::Message::read_async(s)
                        .map_err(|e| Error::ReadMessage { source: e })
                        .and_then(|(msg, s)| {
                            match msg {
                                crate::protocol::Message::StartWatching {
                                    username,
                                } => {
                                    println!(
                                        "got a watch connection from {}",
                                        username
                                    );
                                }
                                m => {
                                    return Err(Error::UnexpectedMessage {
                                        message: m,
                                    })
                                }
                            }
                            Ok(s)
                        });
                    self.in_progress_watch_reads.push(Box::new(read_fut));
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

    fn poll_cast_read(&mut self) -> Result<bool> {
        let mut did_work = false;

        let mut i = 0;
        while i < self.in_progress_cast_reads.len() {
            match self.in_progress_cast_reads[i].poll() {
                Ok(futures::Async::Ready(s)) => {
                    self.in_progress_cast_reads.swap_remove(i);
                    self.cast_socks.push(s);
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
                            self.in_progress_cast_reads.swap_remove(i);
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

    fn poll_watch_read(&mut self) -> Result<bool> {
        let mut did_work = false;

        let mut i = 0;
        while i < self.in_progress_watch_reads.len() {
            match self.in_progress_watch_reads[i].poll() {
                Ok(futures::Async::Ready(s)) => {
                    self.in_progress_watch_reads.swap_remove(i);
                    self.watch_socks.push(s);
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
                            self.in_progress_watch_reads.swap_remove(i);
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
}

impl futures::stream::Stream for ConnectionHandler {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        loop {
            let mut did_work = false;

            did_work |= self.poll_new_connections()?;
            did_work |= self.poll_cast_readable()?;
            did_work |= self.poll_watch_readable()?;
            did_work |= self.poll_cast_read()?;
            did_work |= self.poll_watch_read()?;

            if !did_work {
                break;
            }
        }

        Ok(futures::Async::NotReady)
    }
}
