mod ws;

use crate::prelude::*;

use gotham::router::builder::{DefineSingleRoute as _, DrawRoutes as _};
use gotham::state::FromState as _;
use lazy_static::lazy_static;
use lazy_static_include::*;

lazy_static_include::lazy_static_include_bytes!(
    INDEX_HTML,
    "static/index.html"
);
lazy_static_include::lazy_static_include_bytes!(
    TELETERM_WEB_JS,
    "static/teleterm_web.js"
);
lazy_static_include::lazy_static_include_bytes!(
    TELETERM_WEB_WASM,
    "static/teleterm_web_bg.wasm"
);

#[derive(
    serde::Deserialize,
    gotham_derive::StateData,
    gotham_derive::StaticResponseExtender,
)]
struct WatchQueryParams {
    id: String,
}

pub struct Server {
    server: Box<dyn futures::Future<Item = (), Error = ()> + Send>,
}

impl Server {
    pub fn new<T: std::net::ToSocketAddrs + 'static>(addr: T) -> Self {
        Self {
            server: Box::new(gotham::init_server(addr, router())),
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

pub fn router() -> impl gotham::handler::NewHandler {
    gotham::router::builder::build_simple_router(|route| {
        route.get("/").to(serve_static("text/html", &INDEX_HTML));
        route
            .get("/teleterm_web.js")
            .to(serve_static("application/javascript", &TELETERM_WEB_JS));
        route
            .get("/teleterm_web_bg.wasm")
            .to(serve_static("application/wasm", &TELETERM_WEB_WASM));
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

        let stream = stream
            .context(crate::error::WebSocketAccept)
            .map(|stream| stream.context(crate::error::WebSocket))
            .flatten_stream();

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
            Box::new(stream),
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

type MessageStream = Box<
    dyn futures::Stream<
            Item = tokio_tungstenite::tungstenite::protocol::Message,
            Error = Error,
        > + Send,
>;

struct Connection<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    id: String,
    client: crate::client::Client<S>,
    stream: MessageStream,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Connection<S>
{
    fn new(
        id: &str,
        client: crate::client::Client<S>,
        stream: MessageStream,
    ) -> Self {
        Self {
            client,
            id: id.to_string(),
            stream,
        }
    }

    fn handle_client_message(
        &mut self,
        msg: &crate::protocol::Message,
    ) -> Result<()> {
        // TODO
        log::info!("teleterm client message for {}: {:?}", self.id, msg);
        Ok(())
    }

    fn handle_websocket_message(
        &mut self,
        msg: &tokio_tungstenite::tungstenite::protocol::Message,
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
        match component_future::try_ready!(self.client.poll()).unwrap() {
            crate::client::Event::Disconnect => {
                // TODO: better reconnect handling?
                return Ok(component_future::Async::Ready(()));
            }
            crate::client::Event::Connect => {}
            crate::client::Event::ServerMessage(msg) => {
                self.handle_client_message(&msg)?;
            }
        }
        Ok(component_future::Async::DidWork)
    }

    fn poll_websocket_stream(&mut self) -> component_future::Poll<(), Error> {
        if let Some(msg) = component_future::try_ready!(self.stream.poll()) {
            self.handle_websocket_message(&msg)?;
            Ok(component_future::Async::DidWork)
        } else {
            log::info!("disconnect for {}", self.id);
            Ok(component_future::Async::Ready(()))
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
