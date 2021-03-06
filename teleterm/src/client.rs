use crate::prelude::*;
use rand::Rng as _;
use std::io::Read as _;

const HEARTBEAT_DURATION: std::time::Duration =
    std::time::Duration::from_secs(30);
const RECONNECT_BACKOFF_BASE: std::time::Duration =
    std::time::Duration::from_secs(1);
const RECONNECT_BACKOFF_FACTOR: f32 = 2.0;
const RECONNECT_BACKOFF_MAX: std::time::Duration =
    std::time::Duration::from_secs(60);

const OAUTH_LISTEN_ADDRESS: &str = "127.0.0.1:44141";
const OAUTH_BROWSER_SUCCESS_MESSAGE: &str = "authenticated successfully! now close this page and return to your terminal.";

enum ReadSocket<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    NotConnected,
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
                    Item = crate::protocol::Message,
                    Error = Error,
                > + Send,
        >,
    ),
}

enum WriteSocket<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    NotConnected,
    Connecting(
        Box<
            dyn futures::Future<Item = S, Error = crate::error::Error> + Send,
        >,
    ),
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

pub enum Event {
    ServerMessage(crate::protocol::Message),
    Disconnect,
    Connect,
}

pub type Connector<S> = Box<
    dyn Fn() -> Box<
            dyn futures::Future<Item = S, Error = crate::error::Error> + Send,
        > + Send,
>;

pub struct Client<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    connect: Connector<S>,
    auth: crate::protocol::Auth,
    auth_client: crate::protocol::AuthClient,

    term_type: String,

    heartbeat_timer: tokio::timer::Interval,
    reconnect_timer: Option<tokio::timer::Delay>,
    reconnect_backoff_amount: std::time::Duration,
    last_server_time: std::time::Instant,

    rsock: ReadSocket<S>,
    wsock: WriteSocket<S>,

    // `raw` means to just connect and send Login, then forward all messages
    // as ServerMessage events rather than handling connection messages
    // internally. Connect and Disconnect events will not be sent.
    raw: bool,
    on_login: Vec<crate::protocol::Message>,
    to_send: std::collections::VecDeque<crate::protocol::Message>,

    last_error: Option<String>,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Client<S>
{
    pub fn stream(
        term_type: &str,
        connect: Connector<S>,
        auth: &crate::protocol::Auth,
        auth_client: crate::protocol::AuthClient,
    ) -> Self {
        Self::new(
            term_type,
            connect,
            auth,
            auth_client,
            &[crate::protocol::Message::start_streaming()],
            false,
        )
    }

    pub fn watch(
        term_type: &str,
        connect: Connector<S>,
        auth: &crate::protocol::Auth,
        auth_client: crate::protocol::AuthClient,
        id: &str,
    ) -> Self {
        Self::new(
            term_type,
            connect,
            auth,
            auth_client,
            &[crate::protocol::Message::start_watching(id)],
            false,
        )
    }

    pub fn list(
        term_type: &str,
        connect: Connector<S>,
        auth: &crate::protocol::Auth,
        auth_client: crate::protocol::AuthClient,
    ) -> Self {
        Self::new(term_type, connect, auth, auth_client, &[], false)
    }

    pub fn raw(
        term_type: &str,
        connect: Connector<S>,
        auth: &crate::protocol::Auth,
        auth_client: crate::protocol::AuthClient,
    ) -> Self {
        Self::new(term_type, connect, auth, auth_client, &[], true)
    }

    fn new(
        term_type: &str,
        connect: Connector<S>,
        auth: &crate::protocol::Auth,
        auth_client: crate::protocol::AuthClient,
        on_login: &[crate::protocol::Message],
        raw: bool,
    ) -> Self {
        let heartbeat_timer =
            tokio::timer::Interval::new_interval(HEARTBEAT_DURATION);

        Self {
            connect,
            auth: auth.clone(),
            auth_client,

            term_type: term_type.to_string(),

            heartbeat_timer,
            reconnect_timer: None,
            reconnect_backoff_amount: RECONNECT_BACKOFF_BASE,
            last_server_time: std::time::Instant::now(),

            rsock: ReadSocket::NotConnected,
            wsock: WriteSocket::NotConnected,

            raw,
            on_login: on_login.to_vec(),
            to_send: std::collections::VecDeque::new(),

            last_error: None,
        }
    }

    pub fn send_message(&mut self, msg: crate::protocol::Message) {
        self.to_send.push_back(msg);
    }

    pub fn reconnect(&mut self) {
        self.rsock = ReadSocket::NotConnected;
        self.wsock = WriteSocket::NotConnected;
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_ref().map(std::string::String::as_str)
    }

    fn set_reconnect_timer(&mut self) {
        let delay = rand::thread_rng().gen_range(
            self.reconnect_backoff_amount / 2,
            self.reconnect_backoff_amount,
        );
        let delay = delay.max(RECONNECT_BACKOFF_BASE);
        self.reconnect_timer =
            Some(tokio::timer::Delay::new(std::time::Instant::now() + delay));
        self.reconnect_backoff_amount = self
            .reconnect_backoff_amount
            .mul_f32(RECONNECT_BACKOFF_FACTOR);
        self.reconnect_backoff_amount =
            self.reconnect_backoff_amount.min(RECONNECT_BACKOFF_MAX);
    }

    fn reset_reconnect_timer(&mut self) {
        self.reconnect_timer = None;
        self.reconnect_backoff_amount = RECONNECT_BACKOFF_BASE;
    }

    fn has_seen_server_recently(&self) -> bool {
        let since_last_server =
            std::time::Instant::now().duration_since(self.last_server_time);
        if since_last_server > HEARTBEAT_DURATION * 2 {
            return false;
        }

        true
    }

    fn handle_successful_connection(&mut self, s: S) -> Result<()> {
        self.last_server_time = std::time::Instant::now();

        log::info!("connected to server");

        let (rs, ws) = s.split();
        self.rsock =
            ReadSocket::Connected(crate::protocol::FramedReader::new(rs));
        self.wsock =
            WriteSocket::Connected(crate::protocol::FramedWriter::new(ws));

        self.to_send.clear();
        self.send_message(crate::protocol::Message::login(
            &self.auth,
            self.auth_client,
            &self.term_type,
            crate::term::Size::get()?,
        ));

        Ok(())
    }

    fn handle_message(
        &mut self,
        msg: crate::protocol::Message,
    ) -> Result<(
        component_future::Async<Option<Event>>,
        Option<
            Box<
                dyn futures::Future<
                        Item = crate::protocol::Message,
                        Error = Error,
                    > + Send,
            >,
        >,
    )> {
        log::debug!("recv_message({})", msg.format_log());

        if !self.raw {
            match msg {
                crate::protocol::Message::OauthCliRequest { url, id } => {
                    let mut state = None;
                    let parsed_url = url::Url::parse(&url).unwrap();
                    for (k, v) in parsed_url.query_pairs() {
                        if k == "state" {
                            state = Some(v);
                        }
                    }
                    open::that(url).context(crate::error::OpenLink)?;
                    return Ok((
                        component_future::Async::DidWork,
                        Some(self.wait_for_oauth_response(
                            state.map(|s| s.to_string()),
                            &id,
                        )?),
                    ));
                }
                crate::protocol::Message::LoggedIn { username } => {
                    log::info!(
                        "successfully logged into server as {}",
                        username
                    );
                    self.reset_reconnect_timer();
                    for msg in &self.on_login {
                        self.to_send.push_back(msg.clone());
                    }
                    self.last_error = None;
                    return Ok((
                        component_future::Async::Ready(Some(Event::Connect)),
                        None,
                    ));
                }
                crate::protocol::Message::Heartbeat => {
                    return Ok((component_future::Async::DidWork, None));
                }
                _ => {}
            }
        }
        Ok((
            component_future::Async::Ready(Some(Event::ServerMessage(msg))),
            None,
        ))
    }

    fn wait_for_oauth_response(
        &self,
        state: Option<String>,
        id: &str,
    ) -> Result<
        Box<
            dyn futures::Future<
                    Item = crate::protocol::Message,
                    Error = Error,
                > + Send,
        >,
    > {
        lazy_static::lazy_static! {
            static ref RE: regex::Regex = regex::Regex::new(
                r"^GET (/[^ ]*) HTTP/[0-9.]+$"
            ).unwrap();
        }

        let auth_type = self.auth.auth_type();
        let id = id.to_string();
        let address = OAUTH_LISTEN_ADDRESS
            .parse()
            .context(crate::error::ParseAddr)?;
        let listener = tokio::net::TcpListener::bind(&address)
            .context(crate::error::Bind { address })?;
        Ok(Box::new(
            listener
                .incoming()
                .into_future()
                .map_err(|(e, _)| e)
                .context(crate::error::Acceptor)
                .and_then(|(sock, _)| {
                    let sock = sock.unwrap();
                    tokio::io::lines(std::io::BufReader::new(sock))
                        .into_future()
                        .map_err(|(e, _)| e)
                        .context(crate::error::ReadSocket)
                })
                .and_then(move |(buf, lines)| {
                    let buf = buf.unwrap();
                    let path = &RE
                        .captures(&buf)
                        .context(crate::error::ParseHttpRequest)?[1];
                    let base = url::Url::parse(&format!(
                        "http://{}",
                        OAUTH_LISTEN_ADDRESS
                    ))
                    .unwrap();
                    let url = base
                        .join(path)
                        .context(crate::error::ParseHttpRequestPath)?;
                    let mut req_code = None;
                    let mut req_state = None;
                    for (k, v) in url.query_pairs() {
                        if k == "code" {
                            req_code = Some(v.to_string());
                        }
                        if k == "state" {
                            req_state = Some(v.to_string());
                        }
                    }
                    if state != req_state {
                        return Err(Error::ParseHttpRequestCsrf);
                    }
                    let code = if let Some(code) = req_code {
                        code
                    } else {
                        return Err(Error::ParseHttpRequestMissingCode);
                    };
                    Ok((
                        crate::protocol::Message::oauth_cli_response(&code),
                        lines.into_inner().into_inner(),
                    ))
                })
                .and_then(move |(msg, sock)| {
                    save_client_auth_id(auth_type, &id).map(|_| (msg, sock))
                })
                .and_then(|(msg, sock)| {
                    let response = format!(
                        "HTTP/1.1 200 OK\n\n{}",
                        OAUTH_BROWSER_SUCCESS_MESSAGE
                    );
                    tokio::io::write_all(sock, response)
                        .context(crate::error::WriteSocket)
                        .map(|_| msg)
                }),
        ))
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Client<S>
{
    // XXX rustfmt does a terrible job here
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> component_future::Poll<
            Option<Event>,
            Error,
        >] = &[
        &Self::poll_reconnect_server,
        &Self::poll_read_server,
        &Self::poll_write_server,
        &Self::poll_heartbeat,
    ];

    fn poll_reconnect_server(
        &mut self,
    ) -> component_future::Poll<Option<Event>, Error> {
        match &mut self.wsock {
            WriteSocket::NotConnected => {
                if let Some(timer) = &mut self.reconnect_timer {
                    component_future::try_ready!(timer
                        .poll()
                        .context(crate::error::TimerReconnect));
                }

                self.set_reconnect_timer();
                self.wsock = WriteSocket::Connecting((self.connect)());
            }
            WriteSocket::Connecting(ref mut fut) => match fut.poll() {
                Ok(futures::Async::Ready(s)) => {
                    self.handle_successful_connection(s)?;
                }
                Ok(futures::Async::NotReady) => {
                    return Ok(component_future::Async::NotReady);
                }
                Err(e) => {
                    if self.raw {
                        return Err(e);
                    }

                    log::warn!("error while connecting, reconnecting: {}", e);
                    self.reconnect();
                    self.last_error = Some(format!("{}", e));
                    return Ok(component_future::Async::Ready(Some(
                        Event::Disconnect,
                    )));
                }
            },
            WriteSocket::Connected(..) | WriteSocket::Writing(..) => {
                if self.has_seen_server_recently() || self.raw {
                    return Ok(component_future::Async::NothingToDo);
                } else {
                    log::warn!(
                        "haven't seen server in a while, reconnecting",
                    );
                    self.reconnect();
                    self.last_error =
                        Some("haven't seen server in a while".to_string());
                    return Ok(component_future::Async::Ready(Some(
                        Event::Disconnect,
                    )));
                }
            }
        }

        Ok(component_future::Async::DidWork)
    }

    fn poll_read_server(
        &mut self,
    ) -> component_future::Poll<Option<Event>, Error> {
        match &mut self.rsock {
            ReadSocket::NotConnected => {
                Ok(component_future::Async::NothingToDo)
            }
            ReadSocket::Connected(..) => {
                if let ReadSocket::Connected(s) = std::mem::replace(
                    &mut self.rsock,
                    ReadSocket::NotConnected,
                ) {
                    let fut = crate::protocol::Message::read_async(s);
                    self.rsock = ReadSocket::Reading(Box::new(fut));
                } else {
                    unreachable!()
                }
                Ok(component_future::Async::DidWork)
            }
            ReadSocket::Reading(ref mut fut) => match fut.poll() {
                Ok(futures::Async::Ready((msg, s))) => {
                    self.last_server_time = std::time::Instant::now();
                    match self.handle_message(msg) {
                        Ok((poll, fut)) => {
                            if let Some(fut) = fut {
                                self.rsock = ReadSocket::Processing(s, fut);
                            } else {
                                self.rsock = ReadSocket::Connected(s);
                            }
                            Ok(poll)
                        }
                        Err(e) => {
                            if self.raw {
                                return Err(e);
                            }

                            log::warn!(
                                "error handling message, reconnecting: {}",
                                e
                            );
                            self.reconnect();
                            self.last_error = Some(format!("{}", e));
                            Ok(component_future::Async::Ready(Some(
                                Event::Disconnect,
                            )))
                        }
                    }
                }
                Ok(futures::Async::NotReady) => {
                    Ok(component_future::Async::NotReady)
                }
                Err(e) => {
                    if self.raw {
                        return Err(e);
                    }

                    log::warn!("error reading message, reconnecting: {}", e);
                    self.reconnect();
                    self.last_error = Some(format!("{}", e));
                    Ok(component_future::Async::Ready(Some(
                        Event::Disconnect,
                    )))
                }
            },
            ReadSocket::Processing(_, fut) => match fut.poll() {
                Ok(futures::Async::Ready(msg)) => {
                    if let ReadSocket::Processing(s, _) = std::mem::replace(
                        &mut self.rsock,
                        ReadSocket::NotConnected,
                    ) {
                        self.rsock = ReadSocket::Connected(s);
                        self.send_message(msg);
                    } else {
                        unreachable!()
                    }
                    Ok(component_future::Async::DidWork)
                }
                Ok(futures::Async::NotReady) => {
                    Ok(component_future::Async::NotReady)
                }
                Err(e) => {
                    if self.raw {
                        return Err(e);
                    }

                    log::warn!(
                        "error processing message, reconnecting: {}",
                        e
                    );
                    self.reconnect();
                    self.last_error = Some(format!("{}", e));
                    Ok(component_future::Async::Ready(Some(
                        Event::Disconnect,
                    )))
                }
            },
        }
    }

    fn poll_write_server(
        &mut self,
    ) -> component_future::Poll<Option<Event>, Error> {
        match &mut self.wsock {
            WriteSocket::NotConnected | WriteSocket::Connecting(..) => {
                Ok(component_future::Async::NothingToDo)
            }
            WriteSocket::Connected(..) => {
                if self.to_send.is_empty() {
                    return Ok(component_future::Async::NothingToDo);
                }

                if let WriteSocket::Connected(s) = std::mem::replace(
                    &mut self.wsock,
                    WriteSocket::NotConnected,
                ) {
                    let msg = self.to_send.pop_front().unwrap();
                    log::debug!("send_message({})", msg.format_log());
                    let fut = msg.write_async(s);
                    self.wsock = WriteSocket::Writing(Box::new(fut));
                } else {
                    unreachable!()
                }

                Ok(component_future::Async::DidWork)
            }
            WriteSocket::Writing(ref mut fut) => match fut.poll() {
                Ok(futures::Async::Ready(s)) => {
                    self.wsock = WriteSocket::Connected(s);
                    Ok(component_future::Async::DidWork)
                }
                Ok(futures::Async::NotReady) => {
                    Ok(component_future::Async::NotReady)
                }
                Err(e) => {
                    if self.raw {
                        return Err(e);
                    }

                    log::warn!("error writing message, reconnecting: {}", e);
                    self.reconnect();
                    self.last_error = Some(format!("{}", e));
                    Ok(component_future::Async::Ready(Some(
                        Event::Disconnect,
                    )))
                }
            },
        }
    }

    fn poll_heartbeat(
        &mut self,
    ) -> component_future::Poll<Option<Event>, Error> {
        let _ = component_future::try_ready!(self
            .heartbeat_timer
            .poll()
            .context(crate::error::TimerHeartbeat));
        self.send_message(crate::protocol::Message::heartbeat());
        Ok(component_future::Async::DidWork)
    }
}

#[must_use = "streams do nothing unless polled"]
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    futures::Stream for Client<S>
{
    type Item = Event;
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        component_future::poll_stream(self, Self::POLL_FNS)
    }
}

pub fn load_client_auth_id(
    auth: crate::protocol::AuthType,
) -> Option<String> {
    client_id_file(auth, true).and_then(|id_file| {
        std::fs::File::open(id_file).ok().and_then(|mut file| {
            let mut id = vec![];
            file.read_to_end(&mut id).ok().map(|_| {
                std::string::String::from_utf8_lossy(&id).to_string()
            })
        })
    })
}

fn save_client_auth_id(
    auth: crate::protocol::AuthType,
    id: &str,
) -> impl futures::Future<Item = (), Error = Error> {
    let id_file = client_id_file(auth, false).unwrap();
    let id = id.to_string();
    tokio::fs::File::create(id_file.clone())
        .with_context(move || crate::error::CreateFile {
            filename: id_file.to_string_lossy().to_string(),
        })
        .and_then(|file| {
            tokio::io::write_all(file, id).context(crate::error::WriteFile)
        })
        .map(|_| ())
}

fn client_id_file(
    auth: crate::protocol::AuthType,
    must_exist: bool,
) -> Option<std::path::PathBuf> {
    let filename = format!("client-oauth-{}", auth.name());
    crate::dirs::Dirs::new().data_file(&filename, must_exist)
}
