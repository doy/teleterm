use crate::prelude::*;
use tokio::util::FutureExt as _;

pub mod tls;

enum ReadSocket<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    Connected(crate::protocol::FramedReadHalf<S>),
    Reading(
        Box<
            dyn futures::future::Future<
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
            dyn futures::future::Future<
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
            dyn futures::future::Future<
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

#[derive(Debug, Clone)]
// XXX https://github.com/rust-lang/rust/issues/64362
#[allow(dead_code)]
enum ConnectionState {
    Accepted,
    LoggingIn {
        term_info: TerminalInfo,
    },
    LoggedIn {
        username: String,
        term_info: TerminalInfo,
    },
    Streaming {
        username: String,
        term_info: TerminalInfo,
        saved_data: crate::term::Buffer,
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

    fn term_info_mut(&mut self) -> Option<&mut TerminalInfo> {
        match self {
            Self::Accepted => None,
            Self::LoggingIn { term_info, .. } => Some(term_info),
            Self::LoggedIn { term_info, .. } => Some(term_info),
            Self::Streaming { term_info, .. } => Some(term_info),
            Self::Watching { term_info, .. } => Some(term_info),
        }
    }

    fn saved_data(&self) -> Option<&crate::term::Buffer> {
        match self {
            Self::Accepted => None,
            Self::LoggingIn { .. } => None,
            Self::LoggedIn { .. } => None,
            Self::Streaming { saved_data, .. } => Some(saved_data),
            Self::Watching { .. } => None,
        }
    }

    fn saved_data_mut(&mut self) -> Option<&mut crate::term::Buffer> {
        match self {
            Self::Accepted => None,
            Self::LoggingIn { .. } => None,
            Self::LoggedIn { .. } => None,
            Self::Streaming { saved_data, .. } => Some(saved_data),
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
        size: &crate::term::Size,
    ) {
        if let Self::Accepted = self {
            *self = Self::LoggedIn {
                username: username.to_string(),
                term_info: TerminalInfo {
                    term: term_type.to_string(),
                    size: size.clone(),
                },
            };
        } else {
            unreachable!()
        }
    }

    fn login_oauth(
        &mut self,
        term_type: &str,
        size: &crate::term::Size,
        username: &str,
    ) {
        if let Self::Accepted = self {
            *self = Self::LoggedIn {
                username: username.to_string(),
                term_info: TerminalInfo {
                    term: term_type.to_string(),
                    size: size.clone(),
                },
            };
        } else {
            unreachable!()
        }
    }

    fn login_oauth_start(
        &mut self,
        term_type: &str,
        size: &crate::term::Size,
    ) {
        if let Self::Accepted = self {
            *self = Self::LoggingIn {
                term_info: TerminalInfo {
                    term: term_type.to_string(),
                    size: size.clone(),
                },
            };
        } else {
            unreachable!()
        }
    }

    fn login_oauth_finish(&mut self, username: &str) {
        if let Self::LoggingIn { term_info } = self {
            *self = Self::LoggedIn {
                username: username.to_string(),
                term_info: term_info.clone(),
            };
        } else {
            unreachable!()
        }
    }

    fn stream(&mut self, buffer_size: usize) {
        if let Self::LoggedIn {
            username,
            term_info,
        } = std::mem::replace(self, Self::Accepted)
        {
            *self = Self::Streaming {
                username,
                term_info,
                saved_data: crate::term::Buffer::new(buffer_size),
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
    oauth_client: Option<Box<dyn crate::oauth::Oauth + Send>>,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Connection<S>
{
    fn new(s: S, buffer_size: usize) -> Self {
        let (rs, ws) = s.split();
        let id = format!("{}", uuid::Uuid::new_v4());
        log::info!("{}: new connection", id);

        Self {
            id,
            rsock: Some(ReadSocket::Connected(
                crate::protocol::FramedReader::new(rs, buffer_size),
            )),
            wsock: Some(WriteSocket::Connected(
                crate::protocol::FramedWriter::new(ws, buffer_size),
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
            .saved_data()
            .map_or("", crate::term::Buffer::title);

        // i don't really care if things break for a connection that has been
        // idle for 136 years
        #[allow(clippy::cast_possible_truncation)]
        Some(crate::protocol::Session {
            id: self.id.clone(),
            username: username.clone(),
            term_type: term_info.term.clone(),
            size: term_info.size.clone(),
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
    buffer_size: usize,
    read_timeout: std::time::Duration,
    sock_stream: Box<
        dyn futures::stream::Stream<Item = Connection<S>, Error = Error>
            + Send,
    >,
    connections: std::collections::HashMap<String, Connection<S>>,
    rate_limiter: ratelimit_meter::KeyedRateLimiter<Option<String>>,
    allowed_auth_types: std::collections::HashSet<crate::protocol::AuthType>,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Server<S>
{
    pub fn new(
        buffer_size: usize,
        read_timeout: std::time::Duration,
        sock_r: tokio::sync::mpsc::Receiver<S>,
        allowed_auth_types: std::collections::HashSet<
            crate::protocol::AuthType,
        >,
    ) -> Self {
        let sock_stream = sock_r
            .map(move |s| Connection::new(s, buffer_size))
            .context(crate::error::SocketChannelReceive);
        Self {
            buffer_size,
            read_timeout,
            sock_stream: Box::new(sock_stream),
            connections: std::collections::HashMap::new(),
            rate_limiter: ratelimit_meter::KeyedRateLimiter::new(
                std::num::NonZeroU32::new(300).unwrap(),
                std::time::Duration::from_secs(60),
            ),
            allowed_auth_types,
        }
    }

    fn handle_message_login(
        &mut self,
        conn: &mut Connection<S>,
        auth: &crate::protocol::Auth,
        term_type: &str,
        size: crate::term::Size,
    ) -> Result<
        Option<
            Box<
                dyn futures::future::Future<
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
                conn.state.login_plain(username, term_type, &size);
                conn.send_message(crate::protocol::Message::logged_in(
                    username,
                ));
            }
            oauth if oauth.is_oauth() => {
                let (refresh, client) = match oauth {
                    crate::protocol::Auth::RecurseCenter { id } => {
                        // XXX this needs some kind of real configuration
                        // system
                        let client_id =
                            std::env::var("TT_RECURSE_CENTER_CLIENT_ID")
                                .unwrap();
                        let client_secret =
                            std::env::var("TT_RECURSE_CENTER_CLIENT_SECRET")
                                .unwrap();
                        let redirect_url =
                            std::env::var("TT_RECURSE_CENTER_REDIRECT_URL")
                                .unwrap();
                        let redirect_url =
                            url::Url::parse(&redirect_url).unwrap();

                        (
                            id.is_some(),
                            Box::new(crate::oauth::RecurseCenter::new(
                                &client_id,
                                &client_secret,
                                redirect_url,
                                &id.clone().unwrap_or_else(|| {
                                    format!("{}", uuid::Uuid::new_v4())
                                }),
                            )),
                        )
                    }
                    _ => unreachable!(),
                };

                conn.oauth_client = Some(client);
                let client = conn.oauth_client.as_ref().unwrap();

                log::info!(
                    "{}: login(oauth({}), {:?})",
                    conn.id,
                    auth.name(),
                    client.user_id()
                );

                if refresh {
                    let term_type = term_type.to_string();
                    let client = conn.oauth_client.take().unwrap();
                    let mut new_state = conn.state.clone();
                    let token_filename = client.server_token_file();
                    let fut = tokio::fs::File::open(token_filename.clone())
                        .with_context(move || crate::error::OpenFile {
                            filename: token_filename
                                .to_string_lossy()
                                .to_string(),
                        })
                        .and_then(|file| {
                            tokio::io::lines(std::io::BufReader::new(file))
                                .into_future()
                                .map_err(|(e, _)| e)
                                .context(crate::error::ReadFile)
                        })
                        .and_then(|(refresh_token, _)| {
                            // XXX unwrap here isn't super safe
                            let refresh_token = refresh_token.unwrap();
                            client
                                .get_access_token_from_refresh_token(
                                    refresh_token.trim(),
                                )
                                .and_then(|access_token| {
                                    client.get_username_from_access_token(
                                        &access_token,
                                    )
                                })
                        })
                        .map(move |username| {
                            new_state
                                .login_oauth(&term_type, &size, &username);
                            (
                                new_state,
                                crate::protocol::Message::logged_in(
                                    &username,
                                ),
                            )
                        });
                    return Ok(Some(Box::new(fut)));
                } else {
                    conn.state.login_oauth_start(term_type, &size);
                    let authorize_url = client.generate_authorize_url();
                    let user_id = client.user_id().to_string();
                    conn.send_message(
                        crate::protocol::Message::oauth_request(
                            &authorize_url,
                            &user_id,
                        ),
                    );
                }
            }
            _ => unreachable!(),
        }

        Ok(None)
    }

    fn handle_message_start_streaming(
        &mut self,
        conn: &mut Connection<S>,
    ) -> Result<()> {
        let username = conn.state.username().unwrap();

        log::info!("{}: stream({})", conn.id, username);
        conn.state.stream(self.buffer_size);

        Ok(())
    }

    fn handle_message_start_watching(
        &mut self,
        conn: &mut Connection<S>,
        id: String,
    ) -> Result<()> {
        let username = conn.state.username().unwrap();

        if let Some(stream_conn) = self.connections.get(&id) {
            let data = stream_conn
                .state
                .saved_data()
                .map(crate::term::Buffer::contents)
                .ok_or_else(|| Error::InvalidWatchId {
                    id: id.to_string(),
                })?;

            log::info!("{}: watch({}, {})", conn.id, username, id);
            conn.state.watch(&id);
            conn.send_message(crate::protocol::Message::terminal_output(
                data,
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
        let saved_data = conn.state.saved_data_mut().unwrap();

        saved_data.append_server(data);
        for watch_conn in self.watchers_mut() {
            let watch_id = watch_conn.state.watch_id().unwrap();
            if conn.id == watch_id {
                watch_conn.send_message(
                    crate::protocol::Message::terminal_output(data),
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

        Ok(())
    }

    fn handle_message_oauth_response(
        &mut self,
        conn: &mut Connection<S>,
        code: &str,
    ) -> Result<
        Option<
            Box<
                dyn futures::future::Future<
                        Item = (ConnectionState, crate::protocol::Message),
                        Error = Error,
                    > + Send,
            >,
        >,
    > {
        let client = conn.oauth_client.take().ok_or_else(|| {
            Error::UnexpectedMessage {
                message: crate::protocol::Message::oauth_response(code),
            }
        })?;

        let mut new_state = conn.state.clone();
        let fut = client
            .get_access_token_from_auth_code(code)
            .and_then(|token| client.get_username_from_access_token(&token))
            .map(|username| {
                new_state.login_oauth_finish(&username);
                (new_state, crate::protocol::Message::logged_in(&username))
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
                dyn futures::future::Future<
                        Item = (ConnectionState, crate::protocol::Message),
                        Error = Error,
                    > + Send,
            >,
        >,
    > {
        match message {
            crate::protocol::Message::Login {
                auth,
                term_type,
                size,
                ..
            } => self.handle_message_login(conn, &auth, &term_type, size),
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
                dyn futures::future::Future<
                        Item = (ConnectionState, crate::protocol::Message),
                        Error = Error,
                    > + Send,
            >,
        >,
    > {
        match message {
            crate::protocol::Message::OauthResponse { code } => {
                self.handle_message_oauth_response(conn, &code)
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
                dyn futures::future::Future<
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
    ) -> crate::component_future::Poll<(), Error> {
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
                Ok(crate::component_future::Async::DidWork)
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
                    Ok(crate::component_future::Async::DidWork)
                }
                Ok(futures::Async::NotReady) => {
                    Ok(crate::component_future::Async::NotReady)
                }
                Err(e) => classify_connection_error(e),
            },
            Some(ReadSocket::Processing(_, fut)) => {
                let (state, msg) = try_ready!(fut.poll());
                if let Some(ReadSocket::Processing(s, _)) = conn.rsock.take()
                {
                    conn.state = state;
                    conn.send_message(msg);
                    conn.rsock = Some(ReadSocket::Connected(s));
                } else {
                    unreachable!()
                }
                Ok(crate::component_future::Async::DidWork)
            }
            _ => Ok(crate::component_future::Async::NothingToDo),
        }
    }

    fn poll_write_connection(
        &mut self,
        conn: &mut Connection<S>,
    ) -> crate::component_future::Poll<(), Error> {
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
                    Ok(crate::component_future::Async::DidWork)
                } else if conn.closed {
                    Ok(crate::component_future::Async::Ready(()))
                } else {
                    Ok(crate::component_future::Async::NothingToDo)
                }
            }
            Some(WriteSocket::Writing(fut)) => match fut.poll() {
                Ok(futures::Async::Ready(s)) => {
                    conn.wsock = Some(WriteSocket::Connected(s));
                    Ok(crate::component_future::Async::DidWork)
                }
                Ok(futures::Async::NotReady) => {
                    Ok(crate::component_future::Async::NotReady)
                }
                Err(e) => classify_connection_error(e),
            },
            _ => Ok(crate::component_future::Async::NothingToDo),
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
            -> crate::component_future::Poll<
            (),
            Error,
        >] = &[
        &Self::poll_new_connections,
        &Self::poll_read,
        &Self::poll_write,
    ];

    fn poll_new_connections(
        &mut self,
    ) -> crate::component_future::Poll<(), Error> {
        if let Some(conn) = try_ready!(self.sock_stream.poll()) {
            self.connections.insert(conn.id.to_string(), conn);
            Ok(crate::component_future::Async::DidWork)
        } else {
            Err(Error::SocketChannelClosed)
        }
    }

    fn poll_read(&mut self) -> crate::component_future::Poll<(), Error> {
        let mut did_work = false;
        let mut not_ready = false;

        let keys: Vec<_> = self.connections.keys().cloned().collect();
        for key in keys {
            let mut conn = self.connections.remove(&key).unwrap();
            match self.poll_read_connection(&mut conn) {
                Ok(crate::component_future::Async::Ready(())) => {
                    self.handle_disconnect(&mut conn);
                    continue;
                }
                Ok(crate::component_future::Async::DidWork) => {
                    did_work = true;
                }
                Ok(crate::component_future::Async::NotReady) => {
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
            Ok(crate::component_future::Async::DidWork)
        } else if not_ready {
            Ok(crate::component_future::Async::NotReady)
        } else {
            Ok(crate::component_future::Async::NothingToDo)
        }
    }

    fn poll_write(&mut self) -> crate::component_future::Poll<(), Error> {
        let mut did_work = false;
        let mut not_ready = false;

        let keys: Vec<_> = self.connections.keys().cloned().collect();
        for key in keys {
            let mut conn = self.connections.remove(&key).unwrap();
            match self.poll_write_connection(&mut conn) {
                Ok(crate::component_future::Async::Ready(())) => {
                    self.handle_disconnect(&mut conn);
                    continue;
                }
                Ok(crate::component_future::Async::DidWork) => {
                    did_work = true;
                }
                Ok(crate::component_future::Async::NotReady) => {
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
            Ok(crate::component_future::Async::DidWork)
        } else if not_ready {
            Ok(crate::component_future::Async::NotReady)
        } else {
            Ok(crate::component_future::Async::NothingToDo)
        }
    }
}

fn classify_connection_error(
    e: Error,
) -> crate::component_future::Poll<(), Error> {
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
                return Ok(crate::component_future::Async::Ready(()));
            }
            _ => {
                return Err(source);
            }
        };

        if tokio_err.kind() == tokio::io::ErrorKind::UnexpectedEof {
            Ok(crate::component_future::Async::Ready(()))
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
    futures::future::Future for Server<S>
{
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        crate::component_future::poll_future(self, Self::POLL_FNS)
    }
}
