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
    sessions: Vec<crate::protocol::Session>,
    offset: usize,
    size: (u16, u16),
}

impl SortedSessions {
    fn new(sessions: Vec<crate::protocol::Session>) -> Result<Self> {
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

        let mut keymap = vec![];
        for name in names {
            let sessions = by_name.remove(&name).unwrap();
            for session in sessions {
                keymap.push(session);
            }
        }

        let (cols, rows) = crossterm::terminal()
            .size()
            .context(crate::error::GetTerminalSize)
            .context(Common)?;

        Ok(Self {
            sessions: keymap,
            offset: 0,
            size: (rows, cols),
        })
    }

    fn print(&self) -> Result<()> {
        let char_width = 2;

        let max_name_width = (self.size.1 / 3) as usize;
        let name_width = self
            .sessions
            .iter()
            .map(|s| s.username.len())
            .max()
            .unwrap_or(4);
        // XXX unstable
        // let name_width = name_width.clamp(4, max_name_width);
        let name_width = if name_width < 4 {
            4
        } else if name_width > max_name_width {
            max_name_width
        } else {
            name_width
        };

        let size_width = 7;

        let max_idle_time =
            self.sessions.iter().map(|s| s.idle_time).max().unwrap_or(4);
        let idle_width = format_time(max_idle_time).len();
        let idle_width = if idle_width < 4 { 4 } else { idle_width };

        let max_title_width = (self.size.1 as usize)
            - char_width
            - 3
            - name_width
            - 3
            - size_width
            - 3
            - idle_width
            - 3;

        clear()?;
        println!("welcome to shellshare\r");
        println!("available sessions:\r");
        println!("\r");
        println!(
            "{:4$} | {:5$} | {:6$} | {:7$} | title\r",
            "",
            "name",
            "size",
            "idle",
            char_width,
            name_width,
            size_width,
            idle_width
        );
        println!("{}\r", "-".repeat(self.size.1 as usize));

        let mut prev_name: Option<&str> = None;
        for (i, session) in self.sessions.iter().skip(self.offset).enumerate()
        {
            if let Some(c) = self.idx_to_char(i) {
                let first = if let Some(name) = prev_name {
                    name != session.username
                } else {
                    true
                };

                let display_char = format!("{})", c);
                let display_name = if first {
                    truncate(&session.username, max_name_width)
                } else {
                    "".to_string()
                };
                let display_size =
                    format!("{}x{}", session.size.0, session.size.1);
                let display_idle = format_time(session.idle_time);
                let display_title = truncate(&session.title, max_title_width);

                println!(
                    "{:5$} | {:6$} | {:7$} | {:8$} | {}\r",
                    display_char,
                    display_name,
                    display_size,
                    display_idle,
                    display_title,
                    char_width,
                    name_width,
                    size_width,
                    idle_width
                );

                prev_name = Some(&session.username);
            } else {
                break;
            }
        }
        print!(
            "({}/{}) space: refresh, q: quit, <: prev page, >: next page --> ",
            self.current_page(),
            self.total_pages(),
        );
        std::io::stdout().flush().context(FlushTerminal)?;

        Ok(())
    }

    fn idx_to_char(&self, mut i: usize) -> Option<char> {
        if i >= self.limit() {
            return None;
        }

        if i >= 16 {
            i += 1;
        }

        #[allow(clippy::cast_possible_truncation)]
        Some(std::char::from_u32(('a' as u32) + (i as u32)).unwrap())
    }

    fn char_to_idx(&self, c: char) -> Option<usize> {
        if c == 'q' {
            return None;
        }

        let i = ((c as i32) - ('a' as i32)) as isize;
        if i < 0 {
            return None;
        }
        #[allow(clippy::cast_sign_loss)]
        let mut i = i as usize;

        if i > 16 {
            i -= 1;
        }

        if i > self.limit() {
            return None;
        }

        Some(i)
    }

    fn id_for(&self, c: char) -> Option<&str> {
        self.char_to_idx(c).and_then(|i| {
            self.sessions.get(i + self.offset).map(|s| s.id.as_ref())
        })
    }

    fn next_page(&mut self) {
        let inc = self.limit();
        if self.offset + inc < self.sessions.len() {
            self.offset += inc;
        }
    }

    fn prev_page(&mut self) {
        let dec = self.limit();
        if self.offset >= dec {
            self.offset -= dec;
        }
    }

    fn current_page(&self) -> usize {
        self.offset / self.limit() + 1
    }

    fn total_pages(&self) -> usize {
        if self.sessions.is_empty() {
            1
        } else {
            (self.sessions.len() - 1) / self.limit() + 1
        }
    }

    fn limit(&self) -> usize {
        let rows = self.size.0;
        let limit = rows as usize - 6;
        if limit > 25 {
            25
        } else {
            limit
        }
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
        match &mut self.state {
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
                                crossterm::KeyEvent::Char('<'),
                            ) => {
                                sessions.prev_page();
                                sessions.print()?;
                            }
                            crossterm::InputEvent::Keyboard(
                                crossterm::KeyEvent::Char('>'),
                            ) => {
                                sessions.next_page();
                                sessions.print()?;
                            }
                            crossterm::InputEvent::Keyboard(
                                crossterm::KeyEvent::Char(c),
                            ) => {
                                if let Some(id) = sessions.id_for(c) {
                                    let client = crate::client::Client::watch(
                                        &self.address,
                                        &self.username,
                                        self.heartbeat_duration,
                                        id,
                                    );
                                    self.state = State::Watching {
                                        client: Box::new(client),
                                    };
                                    clear()?;
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
                crate::client::Event::Reconnect(_) => {
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

                        let sessions = SortedSessions::new(sessions)?;

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
                crate::client::Event::Resize(_) => {
                    match &self.state {
                        State::LoggingIn | State::Choosing { .. } => {
                            self.list_client.send_message(
                                crate::protocol::Message::list_sessions(),
                            );
                        }
                        State::Watching { .. } => {}
                    }
                    Ok(crate::component_future::Poll::DidWork)
                }
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
                crate::client::Event::Reconnect(_) => {
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
                crate::client::Event::Resize(_) => {
                    // watch clients don't respond to resize events
                    unreachable!()
                }
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

fn format_time(dur: u32) -> String {
    let secs = dur % 60;
    let dur = dur / 60;
    if dur == 0 {
        return format!("{}s", secs);
    }

    let mins = dur % 60;
    let dur = dur / 60;
    if dur == 0 {
        return format!("{}m{:02}s", mins, secs);
    }

    let hours = dur % 24;
    let dur = dur / 24;
    if dur == 0 {
        return format!("{}h{:02}m{:02}s", hours, mins, secs);
    }

    let days = dur;
    format!("{}d{:02}h{:02}m{:02}s", days, hours, mins, secs)
}

fn truncate(s: &str, len: usize) -> String {
    if s.len() <= len {
        s.to_string()
    } else {
        format!("{}...", &s[..(len - 3)])
    }
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
