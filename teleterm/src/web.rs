mod view;
mod ws;

use crate::prelude::*;

use gotham::router::builder::{DefineSingleRoute as _, DrawRoutes as _};
use gotham::state::FromState as _;
use tokio_tungstenite::tungstenite;

#[derive(
    serde::Deserialize,
    gotham_derive::StateData,
    gotham_derive::StaticResponseExtender,
)]
struct WatchQueryParams {
    id: String,
}

#[derive(Clone, serde::Serialize)]
struct Config {
    title: String,
}

pub struct Server {
    server: Box<dyn futures::Future<Item = (), Error = ()> + Send>,
}

impl Server {
    pub fn new<T: std::net::ToSocketAddrs + 'static>(addr: T) -> Self {
        let data = Config {
            title: "teleterm".to_string(),
        };
        Self {
            server: Box::new(gotham::init_server(addr, router(&data))),
        }
    }
}

impl Server {
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> component_future::Poll<
            (),
            Error,
        >] = &[&Self::poll_web_server];

    fn poll_web_server(&mut self) -> component_future::Poll<(), Error> {
        component_future::try_ready!(self
            .server
            .poll()
            .map_err(|_| unreachable!()));
        Ok(component_future::Async::Ready(()))
    }
}

impl futures::Future for Server {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        component_future::poll_future(self, Self::POLL_FNS)
    }
}

fn router(data: &Config) -> impl gotham::handler::NewHandler {
    gotham::router::builder::build_simple_router(|route| {
        route.get("/").to_new_handler(serve_template(
            "text/html",
            view::INDEX_HTML_TMPL_NAME,
            data,
        ));
        route.get("/teleterm_web.js").to(serve_static(
            "application/javascript",
            &view::TELETERM_WEB_JS,
        ));
        route
            .get("/teleterm_web_bg.wasm")
            .to(serve_static("application/wasm", &view::TELETERM_WEB_WASM));
        route
            .get("/teleterm.css")
            .to(serve_static("text/css", &view::TELETERM_CSS));
        route.get("/list").to(handle_list);
        route
            .get("/watch")
            .with_query_string_extractor::<WatchQueryParams>()
            .to(handle_watch);
    })
}

fn serve_static(
    content_type: &'static str,
    s: &'static [u8],
) -> impl gotham::handler::Handler + Copy {
    move |state| {
        let response = hyper::Response::builder()
            .header("Content-Type", content_type)
            .body(hyper::Body::from(s))
            .unwrap();
        (state, response)
    }
}

fn serve_template(
    content_type: &'static str,
    name: &'static str,
    data: &Config,
) -> impl gotham::handler::NewHandler {
    let data = data.clone();
    move || {
        let data = data.clone();
        Ok(move |state| {
            let rendered = view::HANDLEBARS.render(name, &data).unwrap();
            let response = hyper::Response::builder()
                .header("Content-Type", content_type)
                .body(hyper::Body::from(rendered))
                .unwrap();
            (state, response)
        })
    }
}

fn handle_list(
    state: gotham::state::State,
) -> (gotham::state::State, hyper::Response<hyper::Body>) {
    let address = "127.0.0.1:4144";
    let (_, address) = crate::config::to_connect_address(address).unwrap();
    let connector: crate::client::Connector<_> = Box::new(move || {
        Box::new(
            tokio::net::tcp::TcpStream::connect(&address)
                .context(crate::error::Connect { address }),
        )
    });
    let client = crate::client::Client::list(
        "teleterm-web",
        connector,
        &crate::protocol::Auth::plain("test"),
    );
    let (w_sessions, r_sessions) = tokio::sync::oneshot::channel();
    let lister = Lister::new(client, w_sessions);
    tokio::spawn(lister.map_err(|e| log::warn!("error listing: {}", e)));
    match r_sessions.wait().unwrap() {
        Ok(sessions) => {
            let body = serde_json::to_string(&sessions).unwrap();
            (state, hyper::Response::new(hyper::Body::from(body)))
        }
        Err(e) => {
            log::warn!("error retrieving sessions: {}", e);
            (
                state,
                hyper::Response::new(hyper::Body::from(format!(
                    "error retrieving sessions: {}",
                    e
                ))),
            )
        }
    }
}

struct Lister<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    client: crate::client::Client<S>,
    w_sessions: Option<
        tokio::sync::oneshot::Sender<Result<Vec<crate::protocol::Session>>>,
    >,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Lister<S>
{
    fn new(
        client: crate::client::Client<S>,
        w_sessions: tokio::sync::oneshot::Sender<
            Result<Vec<crate::protocol::Session>>,
        >,
    ) -> Self {
        Self {
            client,
            w_sessions: Some(w_sessions),
        }
    }

    fn server_message(
        &mut self,
        msg: crate::protocol::Message,
    ) -> Option<Result<Vec<crate::protocol::Session>>> {
        match msg {
            crate::protocol::Message::Sessions { sessions } => {
                Some(Ok(sessions))
            }
            crate::protocol::Message::Disconnected => {
                Some(Err(Error::ServerDisconnected))
            }
            crate::protocol::Message::Error { msg } => {
                Some(Err(Error::Server { message: msg }))
            }
            msg => Some(Err(crate::error::Error::UnexpectedMessage {
                message: msg,
            })),
        }
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Lister<S>
{
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> component_future::Poll<
            (),
            Error,
        >] = &[&Self::poll_client];

    fn poll_client(&mut self) -> component_future::Poll<(), Error> {
        match component_future::try_ready!(self.client.poll()).unwrap() {
            crate::client::Event::Disconnect => {
                let res = Err(Error::ServerDisconnected);
                self.w_sessions.take().unwrap().send(res).unwrap();
                return Ok(component_future::Async::Ready(()));
            }
            crate::client::Event::Connect => {
                self.client
                    .send_message(crate::protocol::Message::list_sessions());
            }
            crate::client::Event::ServerMessage(msg) => {
                if let Some(res) = self.server_message(msg) {
                    self.w_sessions.take().unwrap().send(res).unwrap();
                    return Ok(component_future::Async::Ready(()));
                }
            }
        }
        Ok(component_future::Async::DidWork)
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    futures::Future for Lister<S>
{
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        component_future::poll_future(self, Self::POLL_FNS)
    }
}

fn handle_watch(
    mut state: gotham::state::State,
) -> (gotham::state::State, hyper::Response<hyper::Body>) {
    let body = hyper::Body::take_from(&mut state);
    let headers = hyper::HeaderMap::take_from(&mut state);
    if ws::requested(&headers) {
        let (response, stream) = match ws::accept(&headers, body) {
            Ok(res) => res,
            Err(_) => {
                log::error!("failed to accept websocket request");
                return (
                    state,
                    hyper::Response::builder()
                        .status(hyper::StatusCode::BAD_REQUEST)
                        .body(hyper::Body::empty())
                        .unwrap(),
                );
            }
        };

        let query_params = WatchQueryParams::borrow_from(&state);
        let address = "127.0.0.1:4144";
        let (_, address) =
            crate::config::to_connect_address(address).unwrap();
        let connector: crate::client::Connector<_> = Box::new(move || {
            Box::new(
                tokio::net::tcp::TcpStream::connect(&address)
                    .context(crate::error::Connect { address }),
            )
        });
        let client = crate::client::Client::watch(
            "teleterm-web",
            connector,
            &crate::protocol::Auth::plain("test"),
            &query_params.id,
        );
        let conn = Connection::new(
            gotham::state::request_id(&state),
            client,
            ConnectionState::Connecting(Box::new(
                stream.context(crate::error::WebSocketAccept),
            )),
        );

        tokio::spawn(conn.map_err(|e| log::error!("{}", e)));

        (state, response)
    } else {
        (
            state,
            hyper::Response::new(hyper::Body::from(
                "non-websocket request to websocket endpoint",
            )),
        )
    }
}

type WebSocketConnectionFuture = Box<
    dyn futures::Future<
            Item = tokio_tungstenite::WebSocketStream<
                hyper::upgrade::Upgraded,
            >,
            Error = Error,
        > + Send,
>;
type MessageSink = Box<
    dyn futures::Sink<SinkItem = tungstenite::Message, SinkError = Error>
        + Send,
>;
type MessageStream = Box<
    dyn futures::Stream<Item = tungstenite::Message, Error = Error> + Send,
>;

enum SenderState {
    Temporary,
    Connected(MessageSink),
    Sending(
        Box<dyn futures::Future<Item = MessageSink, Error = Error> + Send>,
    ),
    Flushing(
        Box<dyn futures::Future<Item = MessageSink, Error = Error> + Send>,
    ),
}

enum ConnectionState {
    Connecting(WebSocketConnectionFuture),
    Connected(SenderState, MessageStream),
}

impl ConnectionState {
    fn sink(&mut self) -> Option<&mut MessageSink> {
        match self {
            Self::Connected(sender, _) => match sender {
                SenderState::Connected(sink) => Some(sink),
                _ => None,
            },
            _ => None,
        }
    }

    fn send(&mut self, msg: tungstenite::Message) {
        match self {
            Self::Connected(sender, _) => {
                let fut =
                    match std::mem::replace(sender, SenderState::Temporary) {
                        SenderState::Connected(sink) => sink.send(msg),
                        _ => unreachable!(),
                    };
                *sender = SenderState::Sending(Box::new(fut));
            }
            _ => unreachable!(),
        }
    }
}

struct Connection<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    id: String,
    client: crate::client::Client<S>,
    conn: ConnectionState,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Connection<S>
{
    fn new(
        id: &str,
        client: crate::client::Client<S>,
        conn: ConnectionState,
    ) -> Self {
        Self {
            client,
            id: id.to_string(),
            conn,
        }
    }

    fn handle_client_message(
        &mut self,
        msg: &crate::protocol::Message,
    ) -> Result<Option<tungstenite::Message>> {
        match msg {
            crate::protocol::Message::TerminalOutput { .. }
            | crate::protocol::Message::Disconnected
            | crate::protocol::Message::Resize { .. } => {
                let json = serde_json::to_string(msg)
                    .context(crate::error::SerializeMessage)?;
                Ok(Some(tungstenite::Message::Text(json)))
            }
            _ => Ok(None),
        }
    }

    fn handle_websocket_message(
        &mut self,
        msg: &tungstenite::Message,
    ) -> Result<()> {
        // TODO
        log::info!("websocket stream message for {}: {:?}", self.id, msg);
        Ok(())
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Connection<S>
{
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> component_future::Poll<
            (),
            Error,
        >] = &[&Self::poll_client, &Self::poll_websocket_stream];

    fn poll_client(&mut self) -> component_future::Poll<(), Error> {
        // don't start up the client until the websocket connection is fully
        // established and isn't busy
        if self.conn.sink().is_none() {
            return Ok(component_future::Async::NothingToDo);
        };

        match component_future::try_ready!(self.client.poll()).unwrap() {
            crate::client::Event::Disconnect => {
                // TODO: better reconnect handling?
                return Ok(component_future::Async::Ready(()));
            }
            crate::client::Event::Connect => {}
            crate::client::Event::ServerMessage(msg) => {
                if let Some(msg) = self.handle_client_message(&msg)? {
                    self.conn.send(msg);
                }
            }
        }
        Ok(component_future::Async::DidWork)
    }

    fn poll_websocket_stream(&mut self) -> component_future::Poll<(), Error> {
        match &mut self.conn {
            ConnectionState::Connecting(fut) => {
                let stream = component_future::try_ready!(fut.poll());
                let (sink, stream) = stream.split();
                self.conn = ConnectionState::Connected(
                    SenderState::Connected(Box::new(
                        sink.sink_map_err(|e| Error::WebSocket { source: e }),
                    )),
                    Box::new(stream.context(crate::error::WebSocket)),
                );
                Ok(component_future::Async::DidWork)
            }
            ConnectionState::Connected(sender, stream) => match sender {
                SenderState::Temporary => unreachable!(),
                SenderState::Connected(_) => {
                    if let Some(msg) =
                        component_future::try_ready!(stream.poll())
                    {
                        self.handle_websocket_message(&msg)?;
                        Ok(component_future::Async::DidWork)
                    } else {
                        log::info!("disconnect for {}", self.id);
                        Ok(component_future::Async::Ready(()))
                    }
                }
                SenderState::Sending(fut) => {
                    let sink = component_future::try_ready!(fut.poll());
                    *sender = SenderState::Flushing(Box::new(sink.flush()));
                    Ok(component_future::Async::DidWork)
                }
                SenderState::Flushing(fut) => {
                    let sink = component_future::try_ready!(fut.poll());
                    *sender = SenderState::Connected(Box::new(sink));
                    Ok(component_future::Async::DidWork)
                }
            },
        }
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    futures::Future for Connection<S>
{
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        component_future::poll_future(self, Self::POLL_FNS)
    }
}
