use crate::prelude::*;
use tokio::util::FutureExt as _;

pub mod tls;

enum ReadSocket<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    Connected(crate::protocol::FramedReadHalf<S>),
    Reading(
        Box<
            dyn futures::Future<
                    Item = (
                        crate::protocol::Message,
                        crate::protocol::FramedReadHalf<S>,
                    ),
                    Error = Error,
                > + Send,
        >,
    ),
    Processing(
        crate::protocol::FramedReadHalf<S>,
        Box<
            dyn futures::Future<
                    Item = (ConnectionState, crate::protocol::Message),
                    Error = Error,
                > + Send,
        >,
    ),
}

enum WriteSocket<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    Connected(crate::protocol::FramedWriteHalf<S>),
    Writing(
        Box<
            dyn futures::Future<
                    Item = crate::protocol::FramedWriteHalf<S>,
                    Error = Error,
                > + Send,
        >,
    ),
}

#[derive(Debug, Clone)]
struct TerminalInfo {
    term: String,
    size: crate::term::Size,
}

#[allow(clippy::large_enum_variant)]
// XXX https://github.com/rust-lang/rust/issues/64362
#[allow(dead_code)]
enum ConnectionState {
    Accepted,
    LoggingIn {
        auth_type: crate::protocol::AuthType,
        term_info: TerminalInfo,
    },
    LoggedIn {
        username: String,
        term_info: TerminalInfo,
    },
    Streaming {
        username: String,
        term_info: TerminalInfo,
        term: vt100::Parser,
    },
    Watching {
        username: String,
        term_info: TerminalInfo,
        watch_id: String,
    },
}

impl ConnectionState {
    fn new() -> Self {
        Self::Accepted
    }

    fn username(&self) -> Option<&str> {
        match self {
            Self::Accepted => None,
            Self::LoggingIn { .. } => None,
            Self::LoggedIn { username, .. } => Some(username),
            Self::Streaming { username, .. } => Some(username),
            Self::Watching { username, .. } => Some(username),
        }
    }

    fn auth_type(&self) -> Option<crate::protocol::AuthType> {
        match self {
            Self::Accepted => None,
            Self::LoggingIn { auth_type, .. } => Some(*auth_type),
            Self::LoggedIn { .. } => None,
            Self::Streaming { .. } => None,
            Self::Watching { .. } => None,
        }
    }

    fn term_info(&self) -> Option<&TerminalInfo> {
        match self {
            Self::Accepted => None,
            Self::LoggingIn { term_info, .. } => Some(term_info),
            Self::LoggedIn { term_info, .. } => Some(term_info),
            Self::Streaming { term_info, .. } => Some(term_info),
            Self::Watching { term_info, .. } => Some(term_info),
        }
    }

    fn term_info_mut(&mut self) -> Option<&mut TerminalInfo> {
        match self {
            Self::Accepted => None,
            Self::LoggingIn { term_info, .. } => Some(term_info),
            Self::LoggedIn { term_info, .. } => Some(term_info),
            Self::Streaming { term_info, .. } => Some(term_info),
            Self::Watching { term_info, .. } => Some(term_info),
        }
    }

    fn term(&self) -> Option<&vt100::Parser> {
        match self {
            Self::Accepted => None,
            Self::LoggingIn { .. } => None,
            Self::LoggedIn { .. } => None,
            Self::Streaming { term, .. } => Some(term),
            Self::Watching { .. } => None,
        }
    }

    fn term_mut(&mut self) -> Option<&mut vt100::Parser> {
        match self {
            Self::Accepted => None,
            Self::LoggingIn { .. } => None,
            Self::LoggedIn { .. } => None,
            Self::Streaming { term, .. } => Some(term),
            Self::Watching { .. } => None,
        }
    }

    fn watch_id(&self) -> Option<&str> {
        match self {
            Self::Accepted => None,
            Self::LoggingIn { .. } => None,
            Self::LoggedIn { .. } => None,
            Self::Streaming { .. } => None,
            Self::Watching { watch_id, .. } => Some(watch_id),
        }
    }

    fn login_plain(
        &mut self,
        username: &str,
        term_type: &str,
        size: crate::term::Size,
    ) {
        if let Self::Accepted = self {
            *self = Self::LoggedIn {
                username: username.to_string(),
                term_info: TerminalInfo {
                    term: term_type.to_string(),
                    size,
                },
            };
        } else {
            unreachable!()
        }
    }

    fn login_oauth_start(
        &mut self,
        auth_type: crate::protocol::AuthType,
        term_type: &str,
        size: crate::term::Size,
    ) {
        if let Self::Accepted = self {
            *self = Self::LoggingIn {
                auth_type,
                term_info: TerminalInfo {
                    term: term_type.to_string(),
                    size,
                },
            };
        } else {
            unreachable!()
        }
    }

    fn stream(&mut self) {
        if let Self::LoggedIn {
            username,
            term_info,
        } = std::mem::replace(self, Self::Accepted)
        {
            let size = term_info.size;
            *self = Self::Streaming {
                username,
                term_info,
                term: vt100::Parser::new(size.rows, size.cols, 0),
            };
        } else {
            unreachable!()
        }
    }

    fn watch(&mut self, id: &str) {
        if let Self::LoggedIn {
            username,
            term_info,
        } = std::mem::replace(self, Self::Accepted)
        {
            *self = Self::Watching {
                username,
                term_info,
                watch_id: id.to_string(),
            };
        } else {
            unreachable!()
        }
    }
}

struct Connection<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    id: String,
    rsock: Option<ReadSocket<S>>,
    wsock: Option<WriteSocket<S>>,
    to_send: std::collections::VecDeque<crate::protocol::Message>,
    closed: bool,
    state: ConnectionState,
    last_activity: std::time::Instant,
    oauth_client: Option<crate::oauth::Oauth>,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Connection<S>
{
    fn new(s: S) -> Self {
        let (rs, ws) = s.split();
        let id = format!("{}", uuid::Uuid::new_v4());
        log::info!("{}: new connection", id);

        Self {
            id,
            rsock: Some(ReadSocket::Connected(
                crate::protocol::FramedReader::new(rs),
            )),
            wsock: Some(WriteSocket::Connected(
                crate::protocol::FramedWriter::new(ws),
            )),
            to_send: std::collections::VecDeque::new(),
            closed: false,
            state: ConnectionState::new(),
            last_activity: std::time::Instant::now(),
            oauth_client: None,
        }
    }

    fn session(&self, watchers: u32) -> Option<crate::protocol::Session> {
        let (username, term_info) = match &self.state {
            ConnectionState::Accepted => return None,
            ConnectionState::LoggingIn { .. } => return None,
            ConnectionState::LoggedIn {
                username,
                term_info,
            } => (username, term_info),
            ConnectionState::Streaming {
                username,
                term_info,
                ..
            } => (username, term_info),
            ConnectionState::Watching {
                username,
                term_info,
                ..
            } => (username, term_info),
        };
        let title = self
            .state
            .term()
            .map_or("", |parser| parser.screen().title());

        // i don't really care if things break for a connection that has been
        // idle for 136 years
        #[allow(clippy::cast_possible_truncation)]
        Some(crate::protocol::Session {
            id: self.id.clone(),
            username: username.clone(),
            term_type: term_info.term.clone(),
            size: term_info.size,
            idle_time: std::time::Instant::now()
                .duration_since(self.last_activity)
                .as_secs() as u32,
            title: title.to_string(),
            watchers,
        })
    }

    fn send_message(&mut self, message: crate::protocol::Message) {
        self.to_send.push_back(message);
    }

    fn close(&mut self, res: Result<()>) {
        let msg = match res {
            Ok(()) => crate::protocol::Message::disconnected(),
            Err(e) => crate::protocol::Message::error(&format!("{}", e)),
        };
        self.send_message(msg);
        self.closed = true;
    }
}

pub struct Server<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    read_timeout: std::time::Duration,
    acceptor: Box<dyn futures::Stream<Item = S, Error = Error> + Send>,
    connections: std::collections::HashMap<String, Connection<S>>,
    rate_limiter: ratelimit_meter::KeyedRateLimiter<Option<String>>,
    allowed_auth_types: std::collections::HashSet<crate::protocol::AuthType>,
    oauth_configs: std::collections::HashMap<
        crate::protocol::AuthType,
        crate::oauth::Config,
    >,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Server<S>
{
    pub fn new(
        acceptor: Box<dyn futures::Stream<Item = S, Error = Error> + Send>,
        read_timeout: std::time::Duration,
        allowed_auth_types: std::collections::HashSet<
            crate::protocol::AuthType,
        >,
        oauth_configs: std::collections::HashMap<
            crate::protocol::AuthType,
            crate::oauth::Config,
        >,
    ) -> Self {
        Self {
            read_timeout,
            acceptor,
            connections: std::collections::HashMap::new(),
            rate_limiter: ratelimit_meter::KeyedRateLimiter::new(
                std::num::NonZeroU32::new(300).unwrap(),
                std::time::Duration::from_secs(60),
            ),
            allowed_auth_types,
            oauth_configs,
        }
    }

    fn handle_message_login(
        &mut self,
        conn: &mut Connection<S>,
        auth: &crate::protocol::Auth,
        auth_client: crate::protocol::AuthClient,
        term_type: &str,
        size: crate::term::Size,
    ) -> Result<
        Option<
            Box<
                dyn futures::Future<
                        Item = (ConnectionState, crate::protocol::Message),
                        Error = Error,
                    > + Send,
            >,
        >,
    > {
        if size.rows >= 1000 || size.cols >= 1000 {
            return Err(Error::TermTooBig { size });
        }

        let ty = auth.auth_type();
        if !self.allowed_auth_types.contains(&ty) {
            return Err(Error::AuthTypeNotAllowed { ty });
        }

        match &auth {
            crate::protocol::Auth::Plain { username } => {
                log::info!(
                    "{}: login({}, {})",
                    auth.name(),
                    conn.id,
                    username
                );
                conn.state.login_plain(username, term_type, size);
                conn.send_message(crate::protocol::Message::logged_in(
                    username,
                ));
            }
            oauth if oauth.is_oauth() => {
                log::info!(
                    "{}: login(oauth({}.{}), {:?})",
                    conn.id,
                    auth.name(),
                    auth_client.name(),
                    auth.oauth_id(),
                );
                match auth_client {
                    crate::protocol::AuthClient::Cli => {
                        return self.handle_oauth_login_cli(
                            conn, auth, term_type, size,
                        );
                    }
                    crate::protocol::AuthClient::Web => {
                        return self.handle_oauth_login_web();
                    }
                }
            }
            _ => unreachable!(),
        }

        Ok(None)
    }

    fn handle_oauth_login_cli(
        &mut self,
        conn: &mut Connection<S>,
        auth: &crate::protocol::Auth,
        term_type: &str,
        size: crate::term::Size,
    ) -> Result<
        Option<
            Box<
                dyn futures::Future<
                        Item = (ConnectionState, crate::protocol::Message),
                        Error = Error,
                    > + Send,
            >,
        >,
    > {
        let ty = auth.auth_type();
        let config = self
            .oauth_configs
            .get(&ty)
            .context(crate::error::AuthTypeMissingOauthConfig { ty })?;
        let client = auth.oauth_client(config).unwrap();

        if client.server_token_file(true).is_some()
            && auth.oauth_id().is_some()
        {
            let term_type = term_type.to_string();
            let fut = client
                .get_access_token_from_refresh_token()
                .and_then(move |access_token| match ty {
                    crate::protocol::AuthType::RecurseCenter => {
                        crate::auth::recurse_center::get_username(
                            &access_token,
                        )
                    }
                    _ => unreachable!(),
                })
                .map(move |username| {
                    (
                        ConnectionState::LoggedIn {
                            username: username.clone(),
                            term_info: TerminalInfo {
                                term: term_type,
                                size,
                            },
                        },
                        crate::protocol::Message::logged_in(&username),
                    )
                });
            Ok(Some(Box::new(fut)))
        } else {
            conn.oauth_client = Some(client);
            let client = conn.oauth_client.as_ref().unwrap();
            conn.state.login_oauth_start(ty, term_type, size);
            let authorize_url = client.generate_authorize_url();
            let user_id = client.user_id().to_string();
            conn.send_message(crate::protocol::Message::oauth_cli_request(
                &authorize_url,
                &user_id,
            ));
            Ok(None)
        }
    }

    fn handle_oauth_login_web(
        &mut self,
    ) -> Result<
        Option<
            Box<
                dyn futures::Future<
                        Item = (ConnectionState, crate::protocol::Message),
                        Error = Error,
                    > + Send,
            >,
        >,
    > {
        unimplemented!()
    }

    fn handle_message_start_streaming(
        &mut self,
        conn: &mut Connection<S>,
    ) -> Result<()> {
        let username = conn.state.username().unwrap();

        log::info!("{}: stream({})", conn.id, username);
        conn.state.stream();

        Ok(())
    }

    fn handle_message_start_watching(
        &mut self,
        conn: &mut Connection<S>,
        id: String,
    ) -> Result<()> {
        let username = conn.state.username().unwrap();

        if let Some(stream_conn) = self.connections.get(&id) {
            let term = stream_conn.state.term().ok_or_else(|| {
                Error::InvalidWatchId { id: id.to_string() }
            })?;
            let (rows, cols) = term.screen().size();
            let data = term.screen().contents_formatted();

            log::info!("{}: watch({}, {})", conn.id, username, id);
            conn.state.watch(&id);
            conn.send_message(crate::protocol::Message::resize(
                crate::term::Size { rows, cols },
            ));
            conn.send_message(crate::protocol::Message::terminal_output(
                &data,
            ));

            Ok(())
        } else {
            Err(Error::InvalidWatchId { id })
        }
    }

    fn handle_message_heartbeat(
        &mut self,
        conn: &mut Connection<S>,
    ) -> Result<()> {
        conn.send_message(crate::protocol::Message::heartbeat());

        Ok(())
    }

    fn handle_message_terminal_output(
        &mut self,
        conn: &mut Connection<S>,
        data: &[u8],
    ) -> Result<()> {
        let parser = conn.state.term_mut().unwrap();

        let screen = parser.screen().clone();
        parser.process(data);
        let diff = parser.screen().contents_diff(&screen);
        for watch_conn in self.watchers_mut() {
            let watch_id = watch_conn.state.watch_id().unwrap();
            if conn.id == watch_id {
                watch_conn.send_message(
                    crate::protocol::Message::terminal_output(&diff),
                );
            }
        }

        conn.last_activity = std::time::Instant::now();

        Ok(())
    }

    fn handle_message_list_sessions(
        &mut self,
        conn: &mut Connection<S>,
    ) -> Result<()> {
        let mut watcher_counts = std::collections::HashMap::new();
        for watcher in self.watchers() {
            let watch_id =
                if let ConnectionState::Watching { watch_id, .. } =
                    &watcher.state
                {
                    watch_id
                } else {
                    unreachable!()
                };
            watcher_counts.insert(
                watch_id,
                *watcher_counts.get(&watch_id).unwrap_or(&0) + 1,
            );
        }
        let sessions: Vec<_> = self
            .streamers()
            .flat_map(|streamer| {
                streamer
                    .session(*watcher_counts.get(&streamer.id).unwrap_or(&0))
            })
            .collect();
        conn.send_message(crate::protocol::Message::sessions(&sessions));

        Ok(())
    }

    fn handle_message_resize(
        &mut self,
        conn: &mut Connection<S>,
        size: crate::term::Size,
    ) -> Result<()> {
        let term_info = conn.state.term_info_mut().unwrap();
        term_info.size = size;

        if let Some(parser) = conn.state.term_mut() {
            parser.set_size(size.rows, size.cols);
        }

        for watch_conn in self.watchers_mut() {
            let watch_id = watch_conn.state.watch_id().unwrap();
            if conn.id == watch_id {
                watch_conn
                    .send_message(crate::protocol::Message::resize(size));
            }
        }

        Ok(())
    }

    fn handle_message_oauth_cli_response(
        &mut self,
        conn: &mut Connection<S>,
        code: &str,
    ) -> Result<
        Option<
            Box<
                dyn futures::Future<
                        Item = (ConnectionState, crate::protocol::Message),
                        Error = Error,
                    > + Send,
            >,
        >,
    > {
        let client = conn.oauth_client.take().ok_or_else(|| {
            Error::UnexpectedMessage {
                message: crate::protocol::Message::oauth_cli_response(code),
            }
        })?;

        let ty = conn.state.auth_type().unwrap();
        let term_info = conn.state.term_info().unwrap().clone();
        let fut = client
            .get_access_token_from_auth_code(code)
            .and_then(move |access_token| match ty {
                crate::protocol::AuthType::RecurseCenter => {
                    crate::auth::recurse_center::get_username(&access_token)
                }
                _ => unreachable!(),
            })
            .map(|username| {
                (
                    ConnectionState::LoggedIn {
                        term_info,
                        username: username.clone(),
                    },
                    crate::protocol::Message::logged_in(&username),
                )
            });

        Ok(Some(Box::new(fut)))
    }

    fn handle_accepted_message(
        &mut self,
        conn: &mut Connection<S>,
        message: crate::protocol::Message,
    ) -> Result<
        Option<
            Box<
                dyn futures::Future<
                        Item = (ConnectionState, crate::protocol::Message),
                        Error = Error,
                    > + Send,
            >,
        >,
    > {
        match message {
            crate::protocol::Message::Login {
                auth,
                auth_client,
                term_type,
                size,
                ..
            } => self.handle_message_login(
                conn,
                &auth,
                auth_client,
                &term_type,
                size,
            ),
            m => Err(Error::UnauthenticatedMessage { message: m }),
        }
    }

    fn handle_logging_in_message(
        &mut self,
        conn: &mut Connection<S>,
        message: crate::protocol::Message,
    ) -> Result<
        Option<
            Box<
                dyn futures::Future<
                        Item = (ConnectionState, crate::protocol::Message),
                        Error = Error,
                    > + Send,
            >,
        >,
    > {
        match message {
            crate::protocol::Message::OauthCliResponse { code } => {
                self.handle_message_oauth_cli_response(conn, &code)
            }
            m => Err(Error::UnauthenticatedMessage { message: m }),
        }
    }

    fn handle_logged_in_message(
        &mut self,
        conn: &mut Connection<S>,
        message: crate::protocol::Message,
    ) -> Result<()> {
        match message {
            crate::protocol::Message::Heartbeat => {
                self.handle_message_heartbeat(conn)
            }
            crate::protocol::Message::Resize { size } => {
                self.handle_message_resize(conn, size)
            }
            crate::protocol::Message::ListSessions => {
                self.handle_message_list_sessions(conn)
            }
            crate::protocol::Message::StartStreaming => {
                self.handle_message_start_streaming(conn)
            }
            crate::protocol::Message::StartWatching { id } => {
                self.handle_message_start_watching(conn, id)
            }
            m => Err(crate::error::Error::UnexpectedMessage { message: m }),
        }
    }

    fn handle_streaming_message(
        &mut self,
        conn: &mut Connection<S>,
        message: crate::protocol::Message,
    ) -> Result<()> {
        match message {
            crate::protocol::Message::Heartbeat => {
                self.handle_message_heartbeat(conn)
            }
            crate::protocol::Message::Resize { size } => {
                self.handle_message_resize(conn, size)
            }
            crate::protocol::Message::TerminalOutput { data } => {
                self.handle_message_terminal_output(conn, &data)
            }
            m => Err(crate::error::Error::UnexpectedMessage { message: m }),
        }
    }

    fn handle_watching_message(
        &mut self,
        conn: &mut Connection<S>,
        message: crate::protocol::Message,
    ) -> Result<()> {
        match message {
            crate::protocol::Message::Heartbeat => {
                self.handle_message_heartbeat(conn)
            }
            crate::protocol::Message::Resize { size } => {
                self.handle_message_resize(conn, size)
            }
            m => Err(crate::error::Error::UnexpectedMessage { message: m }),
        }
    }

    fn handle_disconnect(&mut self, conn: &mut Connection<S>) {
        if let Some(username) = conn.state.username() {
            log::info!("{}: disconnect({})", conn.id, username);
        } else {
            log::info!("{}: disconnect", conn.id);
        }

        for watch_conn in self.watchers_mut() {
            let watch_id = watch_conn.state.watch_id().unwrap();
            if conn.id == watch_id {
                watch_conn.close(Ok(()));
            }
        }
    }

    fn handle_message(
        &mut self,
        conn: &mut Connection<S>,
        message: crate::protocol::Message,
    ) -> Result<
        Option<
            Box<
                dyn futures::Future<
                        Item = (ConnectionState, crate::protocol::Message),
                        Error = Error,
                    > + Send,
            >,
        >,
    > {
        if let crate::protocol::Message::TerminalOutput { .. } = message {
            // do nothing, we expect TerminalOutput spam
        } else {
            let username =
                conn.state.username().map(std::string::ToString::to_string);
            if self.rate_limiter.check(username).is_err() {
                let display_name =
                    conn.state.username().unwrap_or("(non-logged-in users)");
                log::info!("{}: ratelimit({})", conn.id, display_name);
                return Err(Error::RateLimited);
            }
        }

        log::debug!("{}: recv({})", conn.id, message.format_log());

        match conn.state {
            ConnectionState::Accepted { .. } => {
                self.handle_accepted_message(conn, message)
            }
            ConnectionState::LoggingIn { .. } => {
                self.handle_logging_in_message(conn, message)
            }
            ConnectionState::LoggedIn { .. } => {
                self.handle_logged_in_message(conn, message).map(|_| None)
            }
            ConnectionState::Streaming { .. } => {
                self.handle_streaming_message(conn, message).map(|_| None)
            }
            ConnectionState::Watching { .. } => {
                self.handle_watching_message(conn, message).map(|_| None)
            }
        }
    }

    fn poll_read_connection(
        &mut self,
        conn: &mut Connection<S>,
    ) -> component_future::Poll<(), Error> {
        match &mut conn.rsock {
            Some(ReadSocket::Connected(..)) => {
                if let Some(ReadSocket::Connected(s)) = conn.rsock.take() {
                    let fut = Box::new(
                        crate::protocol::Message::read_async(s)
                            .timeout(self.read_timeout)
                            .context(crate::error::ReadMessageWithTimeout),
                    );
                    conn.rsock = Some(ReadSocket::Reading(fut));
                } else {
                    unreachable!()
                }
                Ok(component_future::Async::DidWork)
            }
            Some(ReadSocket::Reading(fut)) => match fut.poll() {
                Ok(futures::Async::Ready((msg, s))) => {
                    let res = self.handle_message(conn, msg);
                    match res {
                        Ok(Some(fut)) => {
                            conn.rsock = Some(ReadSocket::Processing(s, fut));
                        }
                        Ok(None) => {
                            conn.rsock = Some(ReadSocket::Connected(s));
                        }
                        e @ Err(..) => {
                            conn.close(e.map(|_| ()));
                            conn.rsock = Some(ReadSocket::Connected(s));
                        }
                    }
                    Ok(component_future::Async::DidWork)
                }
                Ok(futures::Async::NotReady) => {
                    Ok(component_future::Async::NotReady)
                }
                Err(e) => classify_connection_error(e),
            },
            Some(ReadSocket::Processing(_, fut)) => {
                let (state, msg) = component_future::try_ready!(fut.poll());
                if let Some(ReadSocket::Processing(s, _)) = conn.rsock.take()
                {
                    conn.state = state;
                    conn.send_message(msg);
                    conn.rsock = Some(ReadSocket::Connected(s));
                } else {
                    unreachable!()
                }
                Ok(component_future::Async::DidWork)
            }
            _ => Ok(component_future::Async::NothingToDo),
        }
    }

    fn poll_write_connection(
        &mut self,
        conn: &mut Connection<S>,
    ) -> component_future::Poll<(), Error> {
        match &mut conn.wsock {
            Some(WriteSocket::Connected(..)) => {
                if let Some(msg) = conn.to_send.pop_front() {
                    if let Some(WriteSocket::Connected(s)) = conn.wsock.take()
                    {
                        log::debug!(
                            "{}: send({})",
                            conn.id,
                            msg.format_log()
                        );
                        let fut = msg
                            .write_async(s)
                            .timeout(self.read_timeout)
                            .context(crate::error::WriteMessageWithTimeout);
                        conn.wsock =
                            Some(WriteSocket::Writing(Box::new(fut)));
                    } else {
                        unreachable!()
                    }
                    Ok(component_future::Async::DidWork)
                } else if conn.closed {
                    Ok(component_future::Async::Ready(()))
                } else {
                    Ok(component_future::Async::NothingToDo)
                }
            }
            Some(WriteSocket::Writing(fut)) => match fut.poll() {
                Ok(futures::Async::Ready(s)) => {
                    conn.wsock = Some(WriteSocket::Connected(s));
                    Ok(component_future::Async::DidWork)
                }
                Ok(futures::Async::NotReady) => {
                    Ok(component_future::Async::NotReady)
                }
                Err(e) => classify_connection_error(e),
            },
            _ => Ok(component_future::Async::NothingToDo),
        }
    }

    fn streamers(&self) -> impl Iterator<Item = &Connection<S>> {
        self.connections.values().filter(|conn| match conn.state {
            ConnectionState::Streaming { .. } => true,
            _ => false,
        })
    }

    fn watchers(&self) -> impl Iterator<Item = &Connection<S>> {
        self.connections.values().filter(|conn| match conn.state {
            ConnectionState::Watching { .. } => true,
            _ => false,
        })
    }

    fn watchers_mut(&mut self) -> impl Iterator<Item = &mut Connection<S>> {
        self.connections
            .values_mut()
            .filter(|conn| match conn.state {
                ConnectionState::Watching { .. } => true,
                _ => false,
            })
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Server<S>
{
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> component_future::Poll<
            (),
            Error,
        >] = &[&Self::poll_accept, &Self::poll_read, &Self::poll_write];

    fn poll_accept(&mut self) -> component_future::Poll<(), Error> {
        if let Some(sock) = component_future::try_ready!(self.acceptor.poll())
        {
            let conn = Connection::new(sock);
            self.connections.insert(conn.id.to_string(), conn);
            Ok(component_future::Async::DidWork)
        } else {
            unreachable!()
        }
    }

    fn poll_read(&mut self) -> component_future::Poll<(), Error> {
        let mut did_work = false;
        let mut not_ready = false;

        let keys: Vec<_> = self.connections.keys().cloned().collect();
        for key in keys {
            let mut conn = self.connections.remove(&key).unwrap();
            match self.poll_read_connection(&mut conn) {
                Ok(component_future::Async::Ready(())) => {
                    self.handle_disconnect(&mut conn);
                    continue;
                }
                Ok(component_future::Async::DidWork) => {
                    did_work = true;
                }
                Ok(component_future::Async::NotReady) => {
                    not_ready = true;
                }
                Err(e) => {
                    log::error!(
                        "error reading from active connection: {}",
                        e
                    );
                    continue;
                }
                _ => {}
            }
            self.connections.insert(key.to_string(), conn);
        }

        if did_work {
            Ok(component_future::Async::DidWork)
        } else if not_ready {
            Ok(component_future::Async::NotReady)
        } else {
            Ok(component_future::Async::NothingToDo)
        }
    }

    fn poll_write(&mut self) -> component_future::Poll<(), Error> {
        let mut did_work = false;
        let mut not_ready = false;

        let keys: Vec<_> = self.connections.keys().cloned().collect();
        for key in keys {
            let mut conn = self.connections.remove(&key).unwrap();
            match self.poll_write_connection(&mut conn) {
                Ok(component_future::Async::Ready(())) => {
                    self.handle_disconnect(&mut conn);
                    continue;
                }
                Ok(component_future::Async::DidWork) => {
                    did_work = true;
                }
                Ok(component_future::Async::NotReady) => {
                    not_ready = true;
                }
                Err(e) => {
                    log::error!("error writing to active connection: {}", e);
                    continue;
                }
                _ => {}
            }
            self.connections.insert(key.to_string(), conn);
        }

        if did_work {
            Ok(component_future::Async::DidWork)
        } else if not_ready {
            Ok(component_future::Async::NotReady)
        } else {
            Ok(component_future::Async::NothingToDo)
        }
    }
}

fn classify_connection_error(e: Error) -> component_future::Poll<(), Error> {
    let source = match e {
        Error::ReadMessageWithTimeout { source } => source,
        Error::WriteMessageWithTimeout { source } => source,
        _ => return Err(e),
    };

    if source.is_inner() {
        let source = source.into_inner().unwrap();
        let tokio_err = match source {
            Error::ReadPacket {
                source: ref tokio_err,
            } => tokio_err,
            Error::WritePacket {
                source: ref tokio_err,
            } => tokio_err,
            Error::EOF => {
                return Ok(component_future::Async::Ready(()));
            }
            _ => {
                return Err(source);
            }
        };

        if tokio_err.kind() == tokio::io::ErrorKind::UnexpectedEof {
            Ok(component_future::Async::Ready(()))
        } else {
            Err(source)
        }
    } else if source.is_elapsed() {
        Err(Error::Timeout)
    } else {
        let source = source.into_timer().unwrap();
        Err(Error::TimerReadTimeout { source })
    }
}

#[must_use = "futures do nothing unless polled"]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    futures::Future for Server<S>
{
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        component_future::poll_future(self, Self::POLL_FNS)
    }
}
