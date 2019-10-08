use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::{OptionExt as _, ResultExt as _};
use std::io::Write as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("{}", source))]
    Common { source: crate::error::Error },

    #[snafu(display("{}", source))]
    SessionList { source: crate::session_list::Error },

    #[snafu(display("failed to read key from terminal: {}", source))]
    ReadKey { source: crate::key_reader::Error },

    #[snafu(display("failed to write to terminal: {}", source))]
    WriteTerminal { source: std::io::Error },

    #[snafu(display("failed to write to terminal: {}", source))]
    WriteTerminalCrossterm { source: crossterm::ErrorKind },

    #[snafu(display("failed to flush writes to terminal: {}", source))]
    FlushTerminal { source: std::io::Error },

    #[snafu(display("communication with server failed: {}", source))]
    Client { source: crate::client::Error },

    #[snafu(display("received error from server: {}", message))]
    Server { message: String },

    #[snafu(display("failed to create key reader: {}", source))]
    KeyReader { source: crate::key_reader::Error },

    #[snafu(display("failed to switch to alternate screen: {}", source))]
    ToAlternateScreen { source: crossterm::ErrorKind },
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Watch shellshare streams")
        .arg(
            clap::Arg::with_name("username")
                .long("username")
                .takes_value(true),
        )
        .arg(
            clap::Arg::with_name("address")
                .long("address")
                .takes_value(true),
        )
}

pub fn run<'a>(matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    run_impl(
        &matches
            .value_of("username")
            .map(std::string::ToString::to_string)
            .or_else(|| std::env::var("USER").ok())
            .context(crate::error::CouldntFindUsername)
            .context(Common)
            .context(super::Watch)?,
        matches.value_of("address").unwrap_or("127.0.0.1:4144"),
    )
    .context(super::Watch)
}

fn run_impl(username: &str, address: &str) -> Result<()> {
    let username = username.to_string();
    let address = address.to_string();
    tokio::run(futures::lazy(move || {
        futures::future::result(WatchSession::new(
            futures::task::current(),
            &address,
            &username,
            std::time::Duration::from_secs(5),
        ))
        .flatten()
        .map_err(|e| {
            eprintln!("{}", e);
        })
    }));

    Ok(())
}

// XXX https://github.com/rust-lang/rust/issues/64362
#[allow(dead_code)]
enum State {
    Temporary,
    LoggingIn {
        alternate_screen: crossterm::AlternateScreen,
    },
    Choosing {
        sessions: crate::session_list::SessionList,
        alternate_screen: crossterm::AlternateScreen,
    },
    Watching {
        client: Box<crate::client::Client>,
    },
}

impl State {
    fn new() -> Result<Self> {
        Ok(Self::LoggingIn {
            alternate_screen: new_alternate_screen()?,
        })
    }

    fn logging_in(&mut self) -> Result<()> {
        let prev_state = std::mem::replace(self, Self::Temporary);
        *self = match prev_state {
            Self::Temporary => unreachable!(),
            Self::LoggingIn { alternate_screen } => {
                Self::LoggingIn { alternate_screen }
            }
            Self::Choosing {
                alternate_screen, ..
            } => Self::LoggingIn { alternate_screen },
            _ => Self::LoggingIn {
                alternate_screen: new_alternate_screen()?,
            },
        };
        Ok(())
    }

    fn choosing(
        &mut self,
        sessions: crate::session_list::SessionList,
    ) -> Result<()> {
        let prev_state = std::mem::replace(self, Self::Temporary);
        *self = match prev_state {
            Self::Temporary => unreachable!(),
            Self::LoggingIn { alternate_screen } => Self::Choosing {
                alternate_screen,
                sessions,
            },
            Self::Choosing {
                alternate_screen, ..
            } => Self::Choosing {
                alternate_screen,
                sessions,
            },
            _ => Self::Choosing {
                alternate_screen: new_alternate_screen()?,
                sessions,
            },
        };
        Ok(())
    }

    fn watching(&mut self, client: crate::client::Client) {
        if let Self::Temporary = self {
            unreachable!()
        }
        *self = Self::Watching {
            client: Box::new(client),
        }
    }
}

struct WatchSession {
    address: String,
    username: String,
    heartbeat_duration: std::time::Duration,

    key_reader: crate::key_reader::KeyReader,
    list_client: crate::client::Client,
    state: State,
    raw_screen: Option<crossterm::RawScreen>,
    needs_redraw: bool,
}

impl WatchSession {
    fn new(
        task: futures::task::Task,
        address: &str,
        username: &str,
        heartbeat_duration: std::time::Duration,
    ) -> Result<Self> {
        let list_client = crate::client::Client::list(
            address,
            username,
            heartbeat_duration,
        );

        Ok(Self {
            address: address.to_string(),
            username: username.to_string(),
            heartbeat_duration,

            key_reader: crate::key_reader::KeyReader::new(task)
                .context(KeyReader)?,
            list_client,
            state: State::new()?,
            raw_screen: None,
            needs_redraw: true,
        })
    }

    fn reconnect(&mut self) -> Result<()> {
        self.state.logging_in()?;
        self.list_client.reconnect();
        self.needs_redraw = true;
        Ok(())
    }

    fn loading_keypress(
        &mut self,
        e: &crossterm::InputEvent,
    ) -> Result<bool> {
        #[allow(clippy::single_match)]
        match e {
            crossterm::InputEvent::Keyboard(crossterm::KeyEvent::Char(
                'q',
            )) => {
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }

    fn list_server_message(
        &mut self,
        msg: crate::protocol::Message,
    ) -> Result<()> {
        match msg {
            crate::protocol::Message::Sessions { sessions } => {
                self.state.choosing(
                    crate::session_list::SessionList::new(sessions)
                        .context(SessionList)?,
                )?;
                self.needs_redraw = true;
            }
            msg => {
                return Err(crate::error::Error::UnexpectedMessage {
                    message: msg,
                })
                .context(Common);
            }
        }
        Ok(())
    }

    fn list_keypress(&mut self, e: &crossterm::InputEvent) -> Result<bool> {
        let sessions =
            if let State::Choosing { sessions, .. } = &mut self.state {
                sessions
            } else {
                unreachable!()
            };

        match e {
            crossterm::InputEvent::Keyboard(crossterm::KeyEvent::Char(
                ' ',
            )) => {
                self.list_client
                    .send_message(crate::protocol::Message::list_sessions());
            }
            crossterm::InputEvent::Keyboard(crossterm::KeyEvent::Char(
                'q',
            )) => {
                return Ok(true);
            }
            crossterm::InputEvent::Keyboard(crossterm::KeyEvent::Char(
                '<',
            )) => {
                sessions.prev_page();
                self.needs_redraw = true;
            }
            crossterm::InputEvent::Keyboard(crossterm::KeyEvent::Char(
                '>',
            )) => {
                sessions.next_page();
                self.needs_redraw = true;
            }
            crossterm::InputEvent::Keyboard(crossterm::KeyEvent::Char(c)) => {
                if let Some(id) = sessions.id_for(*c) {
                    let client = crate::client::Client::watch(
                        &self.address,
                        &self.username,
                        self.heartbeat_duration,
                        id,
                    );
                    self.state.watching(client);
                    crossterm::terminal()
                        .clear(crossterm::ClearType::All)
                        .context(WriteTerminalCrossterm)?;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn watch_server_message(
        &mut self,
        msg: crate::protocol::Message,
    ) -> Result<()> {
        match msg {
            crate::protocol::Message::TerminalOutput { data } => {
                let data: Vec<_> = data
                    .iter()
                    // replace \n with \r\n since we're writing to a
                    // raw terminal
                    .fold(vec![], |mut acc, &c| {
                        if c == b'\n' {
                            acc.push(b'\r');
                            acc.push(b'\n');
                        } else {
                            acc.push(c);
                        }
                        acc
                    });
                // TODO async
                let stdout = std::io::stdout();
                let mut stdout = stdout.lock();
                stdout.write(&data).context(WriteTerminal)?;
                stdout.flush().context(FlushTerminal)?;
            }
            crate::protocol::Message::Disconnected => {
                self.reconnect()?;
            }
            crate::protocol::Message::Error { msg } => {
                return Err(Error::Server { message: msg });
            }
            msg => {
                return Err(crate::error::Error::UnexpectedMessage {
                    message: msg,
                })
                .context(Common);
            }
        }
        Ok(())
    }

    fn watch_keypress(&mut self, e: &crossterm::InputEvent) -> Result<bool> {
        #[allow(clippy::single_match)]
        match e {
            crossterm::InputEvent::Keyboard(crossterm::KeyEvent::Char(
                'q',
            )) => {
                self.reconnect()?;
            }
            _ => {}
        }
        Ok(false)
    }

    fn resize(&mut self, size: crate::term::Size) -> Result<()> {
        if let State::Choosing { sessions, .. } = &mut self.state {
            sessions.resize(size);
            self.needs_redraw = true;
        }
        Ok(())
    }

    fn redraw(&self) -> Result<()> {
        match &self.state {
            State::Temporary => unreachable!(),
            State::LoggingIn { .. } => {
                self.display_loading_message()?;
            }
            State::Choosing { sessions, .. } => {
                sessions.print().context(SessionList)?;
            }
            State::Watching { .. } => {}
        }
        Ok(())
    }

    fn display_loading_message(&self) -> Result<()> {
        crossterm::terminal()
            .clear(crossterm::ClearType::All)
            .context(WriteTerminalCrossterm)?;
        let data = b"loading...\r\nq: quit --> ";
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        stdout.write(data).context(WriteTerminal)?;
        stdout.flush().context(FlushTerminal)?;
        Ok(())
    }
}

impl WatchSession {
    const POLL_FNS: &'static [&'static dyn for<'a> Fn(
        &'a mut Self,
    ) -> Result<
        crate::component_future::Poll<()>,
    >] = &[
        &Self::poll_input,
        &Self::poll_list_client,
        &Self::poll_watch_client,
    ];

    fn poll_input(&mut self) -> Result<crate::component_future::Poll<()>> {
        match self.key_reader.poll().context(ReadKey)? {
            futures::Async::Ready(Some(e)) => {
                let quit = match &mut self.state {
                    State::Temporary => unreachable!(),
                    State::LoggingIn { .. } => self.loading_keypress(&e)?,
                    State::Choosing { .. } => self.list_keypress(&e)?,
                    State::Watching { .. } => self.watch_keypress(&e)?,
                };
                if quit {
                    Ok(crate::component_future::Poll::Event(()))
                } else {
                    Ok(crate::component_future::Poll::DidWork)
                }
            }
            futures::Async::Ready(None) => unreachable!(),
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }

    fn poll_list_client(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        match self.list_client.poll().context(Client)? {
            futures::Async::Ready(Some(e)) => {
                match e {
                    crate::client::Event::Disconnect => {
                        self.reconnect()?;
                    }
                    crate::client::Event::Connect(_) => {
                        self.list_client.send_message(
                            crate::protocol::Message::list_sessions(),
                        );
                    }
                    crate::client::Event::ServerMessage(msg) => {
                        self.list_server_message(msg)?;
                    }
                    crate::client::Event::Resize(size) => {
                        self.resize(size)?;
                    }
                }
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::Ready(None) => {
                // the client should never exit on its own
                unreachable!()
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }

    fn poll_watch_client(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        let client = if let State::Watching { client } = &mut self.state {
            client
        } else {
            return Ok(crate::component_future::Poll::NothingToDo);
        };

        match client.poll().context(Client)? {
            futures::Async::Ready(Some(e)) => {
                match e {
                    crate::client::Event::Disconnect => {
                        self.reconnect()?;
                    }
                    crate::client::Event::Connect(_) => {}
                    crate::client::Event::ServerMessage(msg) => {
                        self.watch_server_message(msg)?;
                    }
                    crate::client::Event::Resize(_) => {
                        // watch clients don't respond to resize events
                        unreachable!();
                    }
                }
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::Ready(None) => {
                // the client should never exit on its own
                unreachable!()
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }
}

fn new_alternate_screen() -> Result<crossterm::AlternateScreen> {
    crossterm::AlternateScreen::to_alternate(false).context(ToAlternateScreen)
}

#[must_use = "futures do nothing unless polled"]
impl futures::future::Future for WatchSession {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        if self.raw_screen.is_none() {
            self.raw_screen = Some(
                crossterm::RawScreen::into_raw_mode()
                    .context(crate::error::IntoRawMode)
                    .context(Common)?,
            );
        }
        let res = crate::component_future::poll_future(self, Self::POLL_FNS);
        if self.needs_redraw {
            self.redraw()?;
            self.needs_redraw = false;
        }
        res
    }
}
