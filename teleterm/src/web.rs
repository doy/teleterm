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
        route.get("/ws").to(handle_websocket_connection);
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

fn handle_websocket_connection(
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
        let conn = Connection::new(
            gotham::state::request_id(&state),
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

struct Connection {
    id: String,
    stream: MessageStream,
}

impl Connection {
    fn new(id: &str, stream: MessageStream) -> Self {
        Self {
            id: id.to_string(),
            stream,
        }
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

impl Connection {
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> component_future::Poll<
            (),
            Error,
        >] = &[&Self::poll_websocket_stream];

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

impl futures::Future for Connection {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        component_future::poll_future(self, Self::POLL_FNS)
    }
}
