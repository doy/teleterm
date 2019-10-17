use crate::prelude::*;
use tokio::io::AsyncWrite as _;

#[derive(serde::Deserialize)]
pub struct Config {
    #[serde(
        deserialize_with = "crate::config::auth",
        default = "crate::config::default_auth"
    )]
    auth: crate::protocol::Auth,

    #[serde(
        deserialize_with = "crate::config::connect_address",
        default = "crate::config::default_connect_address"
    )]
    address: (String, std::net::SocketAddr),

    #[serde(default = "crate::config::default_tls")]
    tls: bool,

    #[serde(default = "crate::config::default_connection_buffer_size")]
    buffer_size: usize,

    #[serde(default = "crate::config::default_command")]
    command: String,

    #[serde(default = "crate::config::default_args")]
    args: Vec<String>,
}

impl Config {
    fn host(&self) -> &str {
        &self.address.0
    }

    fn addr(&self) -> &std::net::SocketAddr {
        &self.address.1
    }
}

impl crate::config::Config for Config {
    fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        if matches.is_present("login-recurse-center") {
            let id = crate::oauth::load_client_auth_id(
                crate::protocol::AuthType::RecurseCenter,
            );
            self.auth = crate::protocol::Auth::recurse_center(
                id.as_ref().map(std::string::String::as_str),
            );
        }
        if matches.is_present("login-plain") {
            let username =
                matches.value_of("login-plain").unwrap().to_string();
            self.auth = crate::protocol::Auth::plain(&username);
        }
        if matches.is_present("address") {
            let address = matches.value_of("address").unwrap();
            self.address = crate::config::to_connect_address(address)?;
        }
        if matches.is_present("tls") {
            self.tls = true;
        }
        if matches.is_present("buffer-size") {
            let buffer_size = matches.value_of("buffer-size").unwrap();
            self.buffer_size = buffer_size.parse().context(
                crate::error::ParseBufferSize { input: buffer_size },
            )?;
        }
        if matches.is_present("command") {
            self.command = matches.value_of("command").unwrap().to_string();
        }
        if matches.is_present("args") {
            self.args = matches
                .values_of("args")
                .unwrap()
                .map(std::string::ToString::to_string)
                .collect();
        }
        Ok(())
    }

    fn run(&self) -> Result<()> {
        let host = self.host().to_string();
        let address = *self.addr();
        let fut: Box<
            dyn futures::future::Future<Item = (), Error = Error> + Send,
        > = if self.tls {
            let connector = native_tls::TlsConnector::new()
                .context(crate::error::CreateConnector)?;
            let connect: crate::client::Connector<_> = Box::new(move || {
                let host = host.clone();
                let connector = connector.clone();
                let connector = tokio_tls::TlsConnector::from(connector);
                let stream = tokio::net::tcp::TcpStream::connect(&address);
                Box::new(stream.context(crate::error::Connect).and_then(
                    move |stream| {
                        connector
                            .connect(&host, stream)
                            .context(crate::error::ConnectTls)
                    },
                ))
            });
            Box::new(StreamSession::new(
                &self.command,
                &self.args,
                connect,
                self.buffer_size,
                &self.auth,
            ))
        } else {
            let connect: crate::client::Connector<_> = Box::new(move || {
                Box::new(
                    tokio::net::tcp::TcpStream::connect(&address)
                        .context(crate::error::Connect),
                )
            });
            Box::new(StreamSession::new(
                &self.command,
                &self.args,
                connect,
                self.buffer_size,
                &self.auth,
            ))
        };
        tokio::run(fut.map_err(|e| {
            eprintln!("{}", e);
        }));
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auth: crate::config::default_auth(),
            address: crate::config::default_connect_address(),
            tls: crate::config::default_tls(),
            buffer_size: crate::config::default_connection_buffer_size(),
            command: crate::config::default_command(),
            args: crate::config::default_args(),
        }
    }
}

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Stream your terminal")
        .arg(
            clap::Arg::with_name("login-plain")
                .long("login-plain")
                .takes_value(true),
        )
        .arg(
            clap::Arg::with_name("login-recurse-center")
                .long("login-recurse-center")
                .conflicts_with("login-plain"),
        )
        .arg(
            clap::Arg::with_name("address")
                .long("address")
                .takes_value(true),
        )
        .arg(clap::Arg::with_name("tls").long("tls"))
        .arg(
            clap::Arg::with_name("buffer-size")
                .long("buffer-size")
                .takes_value(true),
        )
        .arg(clap::Arg::with_name("command").index(1))
        .arg(clap::Arg::with_name("args").index(2).multiple(true))
}

pub fn config() -> Box<dyn crate::config::Config> {
    Box::new(Config::default())
}

struct StreamSession<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    client: crate::client::Client<S>,
    process: crate::resize::ResizingProcess<crate::async_stdin::Stdin>,
    stdout: tokio::io::Stdout,
    buffer: crate::term::Buffer,
    sent_local: usize,
    sent_remote: usize,
    needs_flush: bool,
    connected: bool,
    done: bool,
    raw_screen: Option<crossterm::RawScreen>,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    StreamSession<S>
{
    fn new(
        cmd: &str,
        args: &[String],
        connect: crate::client::Connector<S>,
        buffer_size: usize,
        auth: &crate::protocol::Auth,
    ) -> Self {
        let client =
            crate::client::Client::stream(connect, auth, buffer_size);

        // TODO: tokio::io::stdin is broken (it's blocking)
        // see https://github.com/tokio-rs/tokio/issues/589
        // let input = tokio::io::stdin();
        let input = crate::async_stdin::Stdin::new();

        let process = crate::resize::ResizingProcess::new(
            crate::process::Process::new(cmd, args, input),
        );

        Self {
            client,
            process,
            stdout: tokio::io::stdout(),
            buffer: crate::term::Buffer::new(buffer_size),
            sent_local: 0,
            sent_remote: 0,
            needs_flush: false,
            connected: false,
            done: false,
            raw_screen: None,
        }
    }

    fn record_bytes(&mut self, buf: &[u8]) {
        let written = if self.connected {
            self.sent_local.min(self.sent_remote)
        } else {
            self.sent_local
        };
        let truncated = self.buffer.append_client(buf, written);
        self.sent_local -= truncated;
        if self.connected {
            self.sent_remote -= truncated;
        }
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    StreamSession<S>
{
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> crate::component_future::Poll<
            (),
            Error,
        >] = &[
        &Self::poll_read_client,
        &Self::poll_read_process,
        &Self::poll_write_terminal,
        &Self::poll_flush_terminal,
        &Self::poll_write_server,
    ];

    // this should never return Err, because we don't want server
    // communication issues to ever interrupt a running process
    fn poll_read_client(
        &mut self,
    ) -> crate::component_future::Poll<(), Error> {
        match self.client.poll() {
            Ok(futures::Async::Ready(Some(e))) => match e {
                crate::client::Event::Disconnect => {
                    self.connected = false;
                    Ok(crate::component_future::Async::DidWork)
                }
                crate::client::Event::Connect => {
                    self.connected = true;
                    self.sent_remote = 0;
                    Ok(crate::component_future::Async::DidWork)
                }
                crate::client::Event::ServerMessage(..) => {
                    // we don't expect to ever see a server message once we
                    // start streaming, so if one comes through, assume
                    // something is messed up and try again
                    self.client.reconnect();
                    Ok(crate::component_future::Async::DidWork)
                }
            },
            Ok(futures::Async::Ready(None)) => {
                // the client should never exit on its own
                unreachable!()
            }
            Ok(futures::Async::NotReady) => {
                Ok(crate::component_future::Async::NotReady)
            }
            Err(..) => {
                self.client.reconnect();
                Ok(crate::component_future::Async::DidWork)
            }
        }
    }

    fn poll_read_process(
        &mut self,
    ) -> crate::component_future::Poll<(), Error> {
        match self.process.poll()? {
            futures::Async::Ready(Some(crate::resize::Event::Process(e))) => {
                match e {
                    crate::process::Event::CommandStart(..) => {
                        if self.raw_screen.is_none() {
                            self.raw_screen = Some(
                                crossterm::RawScreen::into_raw_mode()
                                    .context(crate::error::ToRawMode)?,
                            );
                        }
                    }
                    crate::process::Event::CommandExit(..) => {
                        self.done = true;
                    }
                    crate::process::Event::Output(output) => {
                        self.record_bytes(&output);
                    }
                }
                Ok(crate::component_future::Async::DidWork)
            }
            futures::Async::Ready(Some(crate::resize::Event::Resize(
                size,
            ))) => {
                self.client
                    .send_message(crate::protocol::Message::resize(&size));
                Ok(crate::component_future::Async::DidWork)
            }
            futures::Async::Ready(None) => {
                if !self.done {
                    unreachable!()
                }
                // don't return final event here - wait until we are done
                // sending all data to the server (see poll_write_server)
                Ok(crate::component_future::Async::DidWork)
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Async::NotReady)
            }
        }
    }

    fn poll_write_terminal(
        &mut self,
    ) -> crate::component_future::Poll<(), Error> {
        if self.sent_local == self.buffer.len() {
            return Ok(crate::component_future::Async::NothingToDo);
        }

        match self
            .stdout
            .poll_write(&self.buffer.contents()[self.sent_local..])
            .context(crate::error::WriteTerminal)?
        {
            futures::Async::Ready(n) => {
                self.sent_local += n;
                self.needs_flush = true;
                Ok(crate::component_future::Async::DidWork)
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Async::NotReady)
            }
        }
    }

    fn poll_flush_terminal(
        &mut self,
    ) -> crate::component_future::Poll<(), Error> {
        if !self.needs_flush {
            return Ok(crate::component_future::Async::NothingToDo);
        }

        match self
            .stdout
            .poll_flush()
            .context(crate::error::FlushTerminal)?
        {
            futures::Async::Ready(()) => {
                self.needs_flush = false;
                Ok(crate::component_future::Async::DidWork)
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Async::NotReady)
            }
        }
    }

    fn poll_write_server(
        &mut self,
    ) -> crate::component_future::Poll<(), Error> {
        if self.sent_remote == self.buffer.len() || !self.connected {
            // ship all data to the server before actually ending
            if self.done {
                return Ok(crate::component_future::Async::Ready(()));
            } else {
                return Ok(crate::component_future::Async::NothingToDo);
            }
        }

        let buf = &self.buffer.contents()[self.sent_remote..];
        self.client
            .send_message(crate::protocol::Message::terminal_output(buf));
        self.sent_remote = self.buffer.len();

        Ok(crate::component_future::Async::DidWork)
    }
}

#[must_use = "futures do nothing unless polled"]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    futures::future::Future for StreamSession<S>
{
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        crate::component_future::poll_future(self, Self::POLL_FNS)
    }
}
