use crate::prelude::*;
use tokio::io::AsyncWrite as _;

#[derive(serde::Deserialize, Debug, Default)]
pub struct Config {
    #[serde(default)]
    client: crate::config::Client,

    #[serde(default)]
    command: crate::config::Command,
}

impl crate::config::Config for Config {
    fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        self.client.merge_args(matches)?;
        self.command.merge_args(matches)?;
        Ok(())
    }

    fn run(
        &self,
    ) -> Box<dyn futures::Future<Item = (), Error = Error> + Send> {
        let auth = match self.client.auth {
            crate::protocol::AuthType::Plain => {
                let username = self
                    .client
                    .username
                    .clone()
                    .context(crate::error::CouldntFindUsername);
                match username {
                    Ok(username) => crate::protocol::Auth::plain(&username),
                    Err(e) => return Box::new(futures::future::err(e)),
                }
            }
            crate::protocol::AuthType::RecurseCenter => {
                let id = crate::oauth::load_client_auth_id(self.client.auth);
                crate::protocol::Auth::recurse_center(
                    id.as_ref().map(std::string::String::as_str),
                )
            }
        };

        let host = self.client.host().to_string();
        let address = *self.client.addr();
        if self.client.tls {
            let connector = match native_tls::TlsConnector::new()
                .context(crate::error::CreateConnector)
            {
                Ok(connector) => connector,
                Err(e) => return Box::new(futures::future::err(e)),
            };
            let connect: crate::client::Connector<_> = Box::new(move || {
                let host = host.clone();
                let connector = connector.clone();
                let connector = tokio_tls::TlsConnector::from(connector);
                let stream = tokio::net::tcp::TcpStream::connect(&address);
                Box::new(
                    stream
                        .context(crate::error::Connect { address })
                        .and_then(move |stream| {
                            connector
                                .connect(&host, stream)
                                .context(crate::error::ConnectTls { host })
                        }),
                )
            });
            Box::new(StreamSession::new(
                &self.command.command,
                &self.command.args,
                connect,
                &auth,
            ))
        } else {
            let connect: crate::client::Connector<_> = Box::new(move || {
                Box::new(
                    tokio::net::tcp::TcpStream::connect(&address)
                        .context(crate::error::Connect { address }),
                )
            });
            Box::new(StreamSession::new(
                &self.command.command,
                &self.command.args,
                connect,
                &auth,
            ))
        }
    }
}

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    crate::config::Client::cmd(crate::config::Command::cmd(
        app.about("Stream your terminal"),
    ))
}

pub fn config(
    mut config: Option<config::Config>,
) -> Result<Box<dyn crate::config::Config>> {
    if config.is_none() {
        config = crate::config::wizard::run()?;
    }
    let config: Config = if let Some(config) = config {
        config
            .try_into()
            .context(crate::error::CouldntParseConfig)?
    } else {
        Config::default()
    };
    Ok(Box::new(config))
}

struct StreamSession<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    client: crate::client::Client<S>,
    connected: bool,

    process:
        tokio_pty_process_stream::ResizingProcess<crate::async_stdin::Stdin>,
    raw_screen: Option<crossterm::screen::RawScreen>,
    done: bool,

    term: vt100::Parser,
    last_screen: vt100::Screen,
    needs_screen_update: bool,

    stdout: tokio::io::Stdout,
    to_print: std::collections::VecDeque<u8>,
    needs_flush: bool,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    StreamSession<S>
{
    fn new(
        cmd: &str,
        args: &[String],
        connect: crate::client::Connector<S>,
        auth: &crate::protocol::Auth,
    ) -> Self {
        let term_type =
            std::env::var("TERM").unwrap_or_else(|_| "".to_string());
        let client = crate::client::Client::stream(&term_type, connect, auth);

        // TODO: tokio::io::stdin is broken (it's blocking)
        // see https://github.com/tokio-rs/tokio/issues/589
        // let input = tokio::io::stdin();
        let input = crate::async_stdin::Stdin::new();

        let process = tokio_pty_process_stream::ResizingProcess::new(
            tokio_pty_process_stream::Process::new(cmd, args, input),
        );

        let term = vt100::Parser::default();
        let screen = term.screen().clone();

        Self {
            client,
            connected: false,

            process,
            raw_screen: None,
            done: false,

            term,
            last_screen: screen,
            needs_screen_update: false,

            stdout: tokio::io::stdout(),
            to_print: std::collections::VecDeque::new(),
            needs_flush: false,
        }
    }

    fn record_bytes(&mut self, buf: &[u8]) {
        self.to_print.extend(buf);
        self.term.process(buf);
        self.needs_screen_update = true;
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    StreamSession<S>
{
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> component_future::Poll<
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
    fn poll_read_client(&mut self) -> component_future::Poll<(), Error> {
        match self.client.poll() {
            Ok(futures::Async::Ready(Some(e))) => match e {
                crate::client::Event::Disconnect => {
                    self.connected = false;
                    Ok(component_future::Async::DidWork)
                }
                crate::client::Event::Connect => {
                    self.connected = true;
                    self.client.send_message(
                        crate::protocol::Message::terminal_output(
                            &self.last_screen.contents_formatted(),
                        ),
                    );
                    Ok(component_future::Async::DidWork)
                }
                crate::client::Event::ServerMessage(..) => {
                    // we don't expect to ever see a server message once we
                    // start streaming, so if one comes through, assume
                    // something is messed up and try again
                    self.client.reconnect();
                    Ok(component_future::Async::DidWork)
                }
            },
            Ok(futures::Async::Ready(None)) => {
                // the client should never exit on its own
                unreachable!()
            }
            Ok(futures::Async::NotReady) => {
                Ok(component_future::Async::NotReady)
            }
            Err(..) => {
                self.client.reconnect();
                Ok(component_future::Async::DidWork)
            }
        }
    }

    fn poll_read_process(&mut self) -> component_future::Poll<(), Error> {
        match component_future::try_ready!(self
            .process
            .poll()
            .context(crate::error::Subprocess))
        {
            Some(tokio_pty_process_stream::Event::CommandStart {
                ..
            }) => {
                if self.raw_screen.is_none() {
                    self.raw_screen = Some(
                        crossterm::screen::RawScreen::into_raw_mode()
                            .context(crate::error::ToRawMode)?,
                    );
                }
            }
            Some(tokio_pty_process_stream::Event::CommandExit { .. }) => {
                self.done = true;
            }
            Some(tokio_pty_process_stream::Event::Output { data }) => {
                self.record_bytes(&data);
            }
            Some(tokio_pty_process_stream::Event::Resize {
                size: (rows, cols),
            }) => {
                self.client.send_message(crate::protocol::Message::resize(
                    crate::term::Size { rows, cols },
                ));
            }
            None => {
                if !self.done {
                    unreachable!()
                }
                // don't return final event here - wait until we are done
                // sending all data to the server (see poll_write_server)
            }
        }
        Ok(component_future::Async::DidWork)
    }

    fn poll_write_terminal(&mut self) -> component_future::Poll<(), Error> {
        if self.to_print.is_empty() {
            return Ok(component_future::Async::NothingToDo);
        }

        let (a, b) = self.to_print.as_slices();
        let buf = if a.is_empty() { b } else { a };
        let n = component_future::try_ready!(self
            .stdout
            .poll_write(buf)
            .context(crate::error::WriteTerminal));
        for _ in 0..n {
            self.to_print.pop_front();
        }
        self.needs_flush = true;
        Ok(component_future::Async::DidWork)
    }

    fn poll_flush_terminal(&mut self) -> component_future::Poll<(), Error> {
        if !self.needs_flush {
            return Ok(component_future::Async::NothingToDo);
        }

        component_future::try_ready!(self
            .stdout
            .poll_flush()
            .context(crate::error::FlushTerminal));
        self.needs_flush = false;
        Ok(component_future::Async::DidWork)
    }

    fn poll_write_server(&mut self) -> component_future::Poll<(), Error> {
        if !self.connected || !self.needs_screen_update {
            // ship all data to the server before actually ending
            if self.done {
                return Ok(component_future::Async::Ready(()));
            } else {
                return Ok(component_future::Async::NothingToDo);
            }
        }

        let screen = self.term.screen().clone();
        self.client
            .send_message(crate::protocol::Message::terminal_output(
                &screen.contents_diff(&self.last_screen),
            ));
        self.last_screen = screen;
        self.needs_screen_update = false;

        Ok(component_future::Async::DidWork)
    }
}

#[must_use = "futures do nothing unless polled"]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    futures::Future for StreamSession<S>
{
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        component_future::poll_future(self, Self::POLL_FNS)
    }
}
