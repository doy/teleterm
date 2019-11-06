use crate::prelude::*;
use std::io::Write as _;

#[derive(serde::Deserialize, Debug, Default)]
pub struct Config {
    #[serde(default)]
    client: crate::config::Client,
}

impl crate::config::Config for Config {
    fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        self.client.merge_args(matches)
    }

    fn run(
        &self,
    ) -> Box<dyn futures::future::Future<Item = (), Error = Error> + Send>
    {
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
            let make_connector: Box<
                dyn Fn() -> crate::client::Connector<_> + Send,
            > = Box::new(move || {
                let host = host.clone();
                let connector = connector.clone();
                Box::new(move || {
                    let host = host.clone();
                    let connector = connector.clone();
                    let connector = tokio_tls::TlsConnector::from(connector);
                    let stream =
                        tokio::net::tcp::TcpStream::connect(&address);
                    Box::new(
                        stream
                            .context(crate::error::Connect { address })
                            .and_then(move |stream| {
                                connector.connect(&host, stream).context(
                                    crate::error::ConnectTls { host },
                                )
                            }),
                    )
                })
            });
            Box::new(WatchSession::new(make_connector, &auth))
        } else {
            let make_connector: Box<
                dyn Fn() -> crate::client::Connector<_> + Send,
            > = Box::new(move || {
                Box::new(move || {
                    Box::new(
                        tokio::net::tcp::TcpStream::connect(&address)
                            .context(crate::error::Connect { address }),
                    )
                })
            });
            Box::new(WatchSession::new(make_connector, &auth))
        }
    }
}

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    crate::config::Client::cmd(app.about("Watch teleterm streams"))
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

// XXX https://github.com/rust-lang/rust/issues/64362
#[allow(dead_code)]
enum State<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static> {
    Temporary,
    LoggingIn {
        alternate_screen: crossterm::screen::AlternateScreen,
    },
    Choosing {
        sessions: crate::session_list::SessionList,
        alternate_screen: crossterm::screen::AlternateScreen,
    },
    Watching {
        client: Box<crate::client::Client<S>>,
    },
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    State<S>
{
    fn new() -> Self {
        Self::Temporary
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

    fn watching(&mut self, client: crate::client::Client<S>) {
        if let Self::Temporary = self {
            unreachable!()
        }
        *self = Self::Watching {
            client: Box::new(client),
        }
    }
}

struct WatchSession<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    make_connector: Box<dyn Fn() -> crate::client::Connector<S> + Send>,
    auth: crate::protocol::Auth,

    key_reader: crate::key_reader::KeyReader,
    list_client: crate::client::Client<S>,
    resizer: Box<
        dyn futures::stream::Stream<
                Item = (u16, u16),
                Error = crate::error::Error,
            > + Send,
    >,
    state: State<S>,
    raw_screen: Option<crossterm::screen::RawScreen>,
    needs_redraw: bool,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    WatchSession<S>
{
    fn new(
        make_connector: Box<dyn Fn() -> crate::client::Connector<S> + Send>,
        auth: &crate::protocol::Auth,
    ) -> Self {
        let list_client =
            crate::client::Client::list(make_connector(), auth, 4_194_304);

        Self {
            make_connector,
            auth: auth.clone(),

            key_reader: crate::key_reader::KeyReader::new(),
            list_client,
            resizer: Box::new(
                tokio_terminal_resize::resizes()
                    .flatten_stream()
                    .context(crate::error::Resize),
            ),
            state: State::new(),
            raw_screen: None,
            needs_redraw: true,
        }
    }

    fn reconnect(&mut self, hard: bool) -> Result<()> {
        self.state.logging_in()?;
        self.needs_redraw = true;
        if hard {
            self.list_client.reconnect();
        } else {
            self.list_client
                .send_message(crate::protocol::Message::list_sessions());
        }
        Ok(())
    }

    fn loading_keypress(
        &mut self,
        e: &crossterm::input::InputEvent,
    ) -> Result<bool> {
        match e {
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('q'),
            ) => {
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
                    crate::session_list::SessionList::new(
                        sessions,
                        crate::term::Size::get()?,
                    ),
                )?;
                self.needs_redraw = true;
            }
            crate::protocol::Message::Disconnected => {
                self.reconnect(true)?;
            }
            crate::protocol::Message::Error { msg } => {
                return Err(Error::Server { message: msg });
            }
            msg => {
                return Err(crate::error::Error::UnexpectedMessage {
                    message: msg,
                });
            }
        }
        Ok(())
    }

    fn list_keypress(
        &mut self,
        e: &crossterm::input::InputEvent,
    ) -> Result<bool> {
        let sessions =
            if let State::Choosing { sessions, .. } = &mut self.state {
                sessions
            } else {
                unreachable!()
            };

        match e {
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char(' '),
            ) => {
                self.list_client
                    .send_message(crate::protocol::Message::list_sessions());
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('q'),
            ) => {
                return Ok(true);
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('<'),
            ) => {
                sessions.prev_page();
                self.needs_redraw = true;
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('>'),
            ) => {
                sessions.next_page();
                self.needs_redraw = true;
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char(c),
            ) => {
                if let Some(id) = sessions.id_for(*c) {
                    let client = crate::client::Client::watch(
                        (self.make_connector)(),
                        &self.auth,
                        4_194_304,
                        id,
                    );
                    self.state.watching(client);
                    crossterm::execute!(
                        std::io::stdout(),
                        crossterm::terminal::Clear(
                            crossterm::terminal::ClearType::All
                        )
                    )
                    .context(crate::error::WriteTerminalCrossterm)?;
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
                stdout.write(&data).context(crate::error::WriteTerminal)?;
                stdout.flush().context(crate::error::FlushTerminal)?;
            }
            crate::protocol::Message::Disconnected => {
                self.reconnect(false)?;
            }
            crate::protocol::Message::Error { msg } => {
                return Err(Error::Server { message: msg });
            }
            msg => {
                return Err(crate::error::Error::UnexpectedMessage {
                    message: msg,
                });
            }
        }
        Ok(())
    }

    fn watch_keypress(
        &mut self,
        e: &crossterm::input::InputEvent,
    ) -> Result<bool> {
        match e {
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('q'),
            ) => {
                self.reconnect(false)?;
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
                self.display_loading_screen()?;
            }
            State::Choosing { .. } => {
                self.display_choosing_screen()?;
            }
            State::Watching { .. } => {}
        }
        Ok(())
    }

    fn display_loading_screen(&self) -> Result<()> {
        crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
        )
        .context(crate::error::WriteTerminalCrossterm)?;

        println!("loading...\r");
        if let Some(err) = self.list_client.last_error() {
            println!("error: {}\r", err);
        }
        print!("q: quit --> ");

        std::io::stdout()
            .flush()
            .context(crate::error::FlushTerminal)?;

        Ok(())
    }

    fn display_choosing_screen(&self) -> Result<()> {
        let sessions = if let State::Choosing { sessions, .. } = &self.state {
            sessions
        } else {
            unreachable!()
        };

        let char_width = 2;

        let max_name_width = (sessions.size().cols / 3) as usize;
        let name_width = sessions
            .visible_sessions()
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

        let max_idle_time = sessions
            .visible_sessions()
            .iter()
            .map(|s| s.idle_time)
            .max()
            .unwrap_or(4);
        let idle_width = format_time(max_idle_time).len();
        let idle_width = if idle_width < 4 { 4 } else { idle_width };

        let watch_width = 5;

        let max_title_width = (sessions.size().cols as usize)
            - char_width
            - 3
            - name_width
            - 3
            - size_width
            - 3
            - idle_width
            - 3
            - watch_width
            - 3;

        crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
        )
        .context(crate::error::WriteTerminalCrossterm)?;
        println!("welcome to teleterm\r");
        println!("available sessions:\r");
        println!("\r");
        println!(
            "{:5$} | {:6$} | {:7$} | {:8$} | {:9$} | title\r",
            "",
            "name",
            "size",
            "idle",
            "watch",
            char_width,
            name_width,
            size_width,
            idle_width,
            watch_width,
        );
        println!(
            "{}+{}+{}+{}+{}+{}\r",
            "-".repeat(char_width + 1),
            "-".repeat(name_width + 2),
            "-".repeat(size_width + 2),
            "-".repeat(idle_width + 2),
            "-".repeat(watch_width + 2),
            "-".repeat(max_title_width + 1)
        );

        let mut prev_name: Option<&str> = None;
        for (c, session) in sessions.visible_sessions_with_chars() {
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
            let display_size_plain = format!("{}", &session.size);
            let display_size_full = if &session.size == sessions.size() {
                // XXX i should be able to use crossterm::style here, but
                // it has bugs
                format!("\x1b[32m{}\x1b[m", display_size_plain)
            } else if session.size.fits_in(sessions.size()) {
                display_size_plain.clone()
            } else {
                // XXX i should be able to use crossterm::style here, but
                // it has bugs
                format!("\x1b[31m{}\x1b[m", display_size_plain)
            };
            let display_idle = format_time(session.idle_time);
            let display_title = truncate(&session.title, max_title_width);
            let display_watch = session.watchers;

            println!(
                "{:6$} | {:7$} | {:8$} | {:9$} | {:10$} | {}\r",
                display_char,
                display_name,
                display_size_full,
                display_idle,
                display_watch,
                display_title,
                char_width,
                name_width,
                size_width
                    + (display_size_full.len() - display_size_plain.len()),
                idle_width,
                watch_width,
            );

            prev_name = Some(&session.username);
        }
        print!(
            "({}/{}) space: refresh, q: quit, <: prev page, >: next page --> ",
            sessions.current_page(),
            sessions.total_pages(),
        );
        std::io::stdout()
            .flush()
            .context(crate::error::FlushTerminal)?;

        Ok(())
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    WatchSession<S>
{
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> component_future::Poll<
            (),
            Error,
        >] = &[
        &Self::poll_resizer,
        &Self::poll_input,
        &Self::poll_list_client,
        &Self::poll_watch_client,
    ];

    fn poll_resizer(&mut self) -> component_future::Poll<(), Error> {
        let (rows, cols) =
            component_future::try_ready!(self.resizer.poll()).unwrap();
        self.resize(crate::term::Size { rows, cols })?;
        Ok(component_future::Async::DidWork)
    }

    fn poll_input(&mut self) -> component_future::Poll<(), Error> {
        if self.raw_screen.is_none() {
            self.raw_screen = Some(
                crossterm::screen::RawScreen::into_raw_mode()
                    .context(crate::error::ToRawMode)?,
            );
        }
        if let State::Temporary = self.state {
            self.state = State::LoggingIn {
                alternate_screen: new_alternate_screen()?,
            }
        }

        let e = component_future::try_ready!(self.key_reader.poll()).unwrap();
        let quit = match &mut self.state {
            State::Temporary => unreachable!(),
            State::LoggingIn { .. } => self.loading_keypress(&e)?,
            State::Choosing { .. } => self.list_keypress(&e)?,
            State::Watching { .. } => self.watch_keypress(&e)?,
        };
        if quit {
            Ok(component_future::Async::Ready(()))
        } else {
            Ok(component_future::Async::DidWork)
        }
    }

    fn poll_list_client(&mut self) -> component_future::Poll<(), Error> {
        match component_future::try_ready!(self.list_client.poll()).unwrap() {
            crate::client::Event::Disconnect => {
                self.reconnect(true)?;
            }
            crate::client::Event::Connect => {
                self.list_client
                    .send_message(crate::protocol::Message::list_sessions());
            }
            crate::client::Event::ServerMessage(msg) => {
                self.list_server_message(msg)?;
            }
        }
        Ok(component_future::Async::DidWork)
    }

    fn poll_watch_client(&mut self) -> component_future::Poll<(), Error> {
        let client = if let State::Watching { client } = &mut self.state {
            client
        } else {
            return Ok(component_future::Async::NothingToDo);
        };

        match component_future::try_ready!(client.poll()).unwrap() {
            crate::client::Event::Disconnect => {
                self.reconnect(true)?;
            }
            crate::client::Event::Connect => {}
            crate::client::Event::ServerMessage(msg) => {
                self.watch_server_message(msg)?;
            }
        }
        Ok(component_future::Async::DidWork)
    }
}

#[must_use = "futures do nothing unless polled"]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    futures::future::Future for WatchSession<S>
{
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        let res = component_future::poll_future(self, Self::POLL_FNS);
        if res.is_err() {
            self.state = State::Temporary; // drop alternate screen
            self.raw_screen = None;
        } else if self.needs_redraw {
            self.redraw()?;
            self.needs_redraw = false;
        }
        res
    }
}

fn new_alternate_screen() -> Result<crossterm::screen::AlternateScreen> {
    crossterm::screen::AlternateScreen::to_alternate(false)
        .context(crate::error::ToAlternateScreen)
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("abcdefghij", 12), "abcdefghij");
        assert_eq!(truncate("abcdefghij", 11), "abcdefghij");
        assert_eq!(truncate("abcdefghij", 10), "abcdefghij");
        assert_eq!(truncate("abcdefghij", 9), "abcdef...");
        assert_eq!(truncate("abcdefghij", 8), "abcde...");
        assert_eq!(truncate("abcdefghij", 7), "abcd...");

        assert_eq!(truncate("", 7), "");
        assert_eq!(truncate("a", 7), "a");
        assert_eq!(truncate("ab", 7), "ab");
        assert_eq!(truncate("abc", 7), "abc");
        assert_eq!(truncate("abcd", 7), "abcd");
        assert_eq!(truncate("abcde", 7), "abcde");
        assert_eq!(truncate("abcdef", 7), "abcdef");
        assert_eq!(truncate("abcdefg", 7), "abcdefg");
        assert_eq!(truncate("abcdefgh", 7), "abcd...");
        assert_eq!(truncate("abcdefghi", 7), "abcd...");
        assert_eq!(truncate("abcdefghij", 7), "abcd...");
    }

    #[test]
    fn test_format_time() {
        assert_eq!(format_time(0), "0s");
        assert_eq!(format_time(5), "5s");
        assert_eq!(format_time(10), "10s");
        assert_eq!(format_time(60), "1m00s");
        assert_eq!(format_time(61), "1m01s");
        assert_eq!(format_time(601), "10m01s");
        assert_eq!(format_time(610), "10m10s");
        assert_eq!(format_time(3599), "59m59s");
        assert_eq!(format_time(3600), "1h00m00s");
        assert_eq!(format_time(3601), "1h00m01s");
        assert_eq!(format_time(3610), "1h00m10s");
        assert_eq!(format_time(3660), "1h01m00s");
        assert_eq!(format_time(3661), "1h01m01s");
        assert_eq!(format_time(3670), "1h01m10s");
        assert_eq!(format_time(4200), "1h10m00s");
        assert_eq!(format_time(4201), "1h10m01s");
        assert_eq!(format_time(4210), "1h10m10s");
        assert_eq!(format_time(36000), "10h00m00s");
        assert_eq!(format_time(86399), "23h59m59s");
        assert_eq!(format_time(86400), "1d00h00m00s");
        assert_eq!(format_time(86401), "1d00h00m01s");
        assert_eq!(format_time(864_000), "10d00h00m00s");
        assert_eq!(format_time(8_640_000), "100d00h00m00s");
        assert_eq!(format_time(86_400_000), "1000d00h00m00s");
    }
}
