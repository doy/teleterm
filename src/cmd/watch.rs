use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::{OptionExt as _, ResultExt as _};
use std::io::Write as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("{}", source))]
    Common { source: crate::error::Error },

    #[snafu(display("failed to read message: {}", source))]
    Read { source: crate::protocol::Error },

    #[snafu(display("failed to read key from terminal: {}", source))]
    ReadKey { source: crate::key_reader::Error },

    #[snafu(display("failed to write message: {}", source))]
    Write { source: crate::protocol::Error },

    #[snafu(display("failed to write to terminal: {}", source))]
    WriteTerminal { source: std::io::Error },

    #[snafu(display("failed to flush writes to terminal: {}", source))]
    FlushTerminal { source: std::io::Error },

    #[snafu(display("failed to write to terminal: {}", source))]
    WriteTerminalCrossterm { source: crossterm::ErrorKind },

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

struct SortedSessions {
    sessions: std::collections::BTreeMap<char, crate::protocol::Session>,
}

impl SortedSessions {
    fn new(sessions: Vec<crate::protocol::Session>) -> Self {
        let mut by_name = std::collections::HashMap::new();
        for session in sessions {
            if !by_name.contains_key(&session.username) {
                by_name.insert(session.username.clone(), vec![]);
            }
            by_name.get_mut(&session.username).unwrap().push(session);
        }
        let mut names: Vec<_> = by_name.keys().cloned().collect();
        names.sort_by(|a: &String, b: &String| {
            let a_idle =
                by_name[a].iter().min_by_key(|session| session.idle_time);
            let b_idle =
                by_name[b].iter().min_by_key(|session| session.idle_time);
            // these unwraps are safe because we know that none of the vecs in
            // the map can be empty
            a_idle.unwrap().idle_time.cmp(&b_idle.unwrap().idle_time)
        });
        for name in &names {
            if let Some(sessions) = by_name.get_mut(name) {
                sessions.sort_by_key(|s| s.idle_time);
            }
        }

        let mut keymap = std::collections::BTreeMap::new();
        let mut offset = 0;
        for name in names {
            let sessions = by_name.remove(&name).unwrap();
            for session in sessions {
                let c = std::char::from_u32(('a' as u32) + offset).unwrap();
                offset += 1;
                if offset == 16 {
                    // 'q'
                    offset += 1;
                }
                keymap.insert(c, session);
            }
        }

        Self { sessions: keymap }
    }

    fn print(&self) -> Result<()> {
        let name_width =
            self.sessions.iter().map(|(_, s)| s.username.len()).max();
        let name_width = if let Some(width) = name_width {
            if width < 4 {
                4
            } else {
                width
            }
        } else {
            4
        };
        let (cols, _) = crossterm::terminal()
            .size()
            .context(crate::error::GetTerminalSize)
            .context(Common)?;

        clear()?;
        println!("welcome to shellshare\r");
        println!("available sessions:\r");
        println!("\r");
        println!(
            "   | {:3$} | {:7} | {:13} | title\r",
            "name", "size", "idle", name_width
        );
        println!("{}\r", "-".repeat(cols as usize));

        let mut prev_name: Option<&str> = None;
        for (c, session) in &self.sessions {
            let first = if let Some(name) = prev_name {
                name != session.username
            } else {
                true
            };
            print!(
                "{})   {:2$} ",
                c,
                if first { &session.username } else { "" },
                name_width + 2,
            );
            print_session(session);

            println!("\r");
            prev_name = Some(&session.username);
        }
        print!(" --> ");
        std::io::stdout().flush().context(FlushTerminal)?;

        Ok(())
    }

    fn id_for(&self, c: char) -> Option<&str> {
        self.sessions.get(&c).map(|s| s.id.as_ref())
    }
}

enum State {
    LoggingIn,
    Choosing {
        sessions: SortedSessions,
        alternate_screen: crossterm::AlternateScreen,
    },
    Watching {
        client: Box<crate::client::Client>,
    },
}

struct WatchSession {
    address: String,
    username: String,
    heartbeat_duration: std::time::Duration,

    key_reader: crate::key_reader::KeyReader,
    list_client: crate::client::Client,
    state: State,
    raw_screen: Option<crossterm::RawScreen>,
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
            state: State::LoggingIn,
            raw_screen: None,
        })
    }

    fn reconnect(&mut self) {
        self.state = State::LoggingIn;
        self.list_client
            .send_message(crate::protocol::Message::list_sessions());
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
        match &self.state {
            State::LoggingIn => {
                Ok(crate::component_future::Poll::NothingToDo)
            }
            State::Choosing { sessions, .. } => {
                match self.key_reader.poll().context(ReadKey)? {
                    futures::Async::Ready(Some(e)) => {
                        match e {
                            crossterm::InputEvent::Keyboard(
                                crossterm::KeyEvent::Char(' '),
                            ) => {
                                self.list_client.send_message(
                                    crate::protocol::Message::list_sessions(),
                                );
                            }
                            crossterm::InputEvent::Keyboard(
                                crossterm::KeyEvent::Char('q'),
                            ) => {
                                return Ok(
                                    crate::component_future::Poll::Event(()),
                                );
                            }
                            crossterm::InputEvent::Keyboard(
                                crossterm::KeyEvent::Char(c),
                            ) => {
                                if let Some(id) = sessions.id_for(c) {
                                    clear()?;
                                    let client = crate::client::Client::watch(
                                        &self.address,
                                        &self.username,
                                        self.heartbeat_duration,
                                        id,
                                    );
                                    self.state = State::Watching {
                                        client: Box::new(client),
                                    };
                                }
                            }
                            _ => {}
                        }
                        Ok(crate::component_future::Poll::DidWork)
                    }
                    futures::Async::Ready(None) => unreachable!(),
                    futures::Async::NotReady => {
                        Ok(crate::component_future::Poll::NotReady)
                    }
                }
            }
            State::Watching { .. } => {
                match self.key_reader.poll().context(ReadKey)? {
                    futures::Async::Ready(Some(e)) => {
                        #[allow(clippy::single_match)]
                        match e {
                            crossterm::InputEvent::Keyboard(
                                crossterm::KeyEvent::Char('q'),
                            ) => {
                                self.reconnect();
                            }
                            _ => {}
                        }
                        Ok(crate::component_future::Poll::DidWork)
                    }
                    futures::Async::Ready(None) => unreachable!(),
                    futures::Async::NotReady => {
                        Ok(crate::component_future::Poll::NotReady)
                    }
                }
            }
        }
    }

    fn poll_list_client(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        match self.list_client.poll().context(Client)? {
            futures::Async::Ready(Some(e)) => match e {
                crate::client::Event::Reconnect => {
                    self.reconnect();
                    Ok(crate::component_future::Poll::DidWork)
                }
                crate::client::Event::ServerMessage(msg) => match msg {
                    crate::protocol::Message::Sessions { sessions } => {
                        // avoid dropping the alternate screen object if we
                        // don't have to, because it causes flickering
                        let old_state = std::mem::replace(
                            &mut self.state,
                            State::LoggingIn,
                        );
                        let alternate_screen = if let State::Choosing {
                            alternate_screen,
                            ..
                        } = old_state
                        {
                            alternate_screen
                        } else {
                            crossterm::AlternateScreen::to_alternate(false)
                                .context(ToAlternateScreen)?
                        };

                        let sessions = SortedSessions::new(sessions);

                        self.state = State::Choosing {
                            sessions,
                            alternate_screen,
                        };

                        if let State::Choosing { sessions, .. } = &self.state
                        {
                            // TODO: async
                            sessions.print()?;
                        } else {
                            unreachable!();
                        }

                        Ok(crate::component_future::Poll::DidWork)
                    }
                    msg => Err(crate::error::Error::UnexpectedMessage {
                        message: msg,
                    })
                    .context(Common),
                },
            },
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
            futures::Async::Ready(Some(e)) => match e {
                crate::client::Event::Reconnect => {
                    Ok(crate::component_future::Poll::DidWork)
                }
                crate::client::Event::ServerMessage(msg) => match msg {
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
                        Ok(crate::component_future::Poll::DidWork)
                    }
                    crate::protocol::Message::Disconnected => {
                        self.reconnect();
                        Ok(crate::component_future::Poll::DidWork)
                    }
                    crate::protocol::Message::Error { msg } => {
                        eprintln!("server error: {}", msg);
                        Ok(crate::component_future::Poll::Event(()))
                    }
                    msg => Err(crate::error::Error::UnexpectedMessage {
                        message: msg,
                    })
                    .context(Common),
                },
            },
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

fn print_session(session: &crate::protocol::Session) {
    let size = format!("{}x{}", session.size.0, session.size.1);
    print!(
        "{:7}   {:13}   {}",
        size,
        format_time(session.idle_time),
        session.title
    );
}

fn format_time(dur: u32) -> String {
    let secs = dur % 60;
    let dur = dur / 60;
    if dur == 0 {
        return format!("{}s", secs);
    }

    let mins = dur % 60;
    let dur = dur / 60;
    if dur == 0 {
        return format!("{}m{}s", mins, secs);
    }

    let hours = dur % 24;
    let dur = dur / 24;
    if dur == 0 {
        return format!("{}h{}m{}s", hours, mins, secs);
    }

    let days = dur;
    format!("{}d{}h{}m{}s", days, hours, mins, secs)
}

fn clear() -> Result<()> {
    let term = crossterm::terminal();
    term.clear(crossterm::ClearType::All)
        .context(WriteTerminalCrossterm)
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
        crate::component_future::poll_future(self, Self::POLL_FNS)
    }
}
