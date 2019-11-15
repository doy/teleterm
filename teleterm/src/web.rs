mod ws;

use futures::{Future as _, Sink as _, Stream as _};
use gotham::router::builder::{DefineSingleRoute as _, DrawRoutes as _};
use gotham::state::FromState as _;

pub fn router() -> impl gotham::handler::NewHandler {
    gotham::router::builder::build_simple_router(|route| {
        route.get("/").to(root);
        route.get("/ws").to(handle_websocket_connection);
    })
}

fn root(state: gotham::state::State) -> (gotham::state::State, String) {
    log::info!("request for /");
    (state, "hello world".to_string())
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
        let req_id = gotham::state::request_id(&state).to_owned();
        let stream = stream
            .map_err(|e| {
                log::error!(
                    "error upgrading connection for websockets: {}",
                    e
                )
            })
            .and_then(move |stream| handle_websocket_stream(req_id, stream));
        tokio::spawn(stream);
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

fn handle_websocket_stream<S>(
    req_id: String,
    stream: S,
) -> impl futures::Future<Item = (), Error = ()>
where
    S: futures::Stream<
            Item = tokio_tungstenite::tungstenite::protocol::Message,
            Error = tokio_tungstenite::tungstenite::Error,
        > + futures::Sink<
            SinkItem = tokio_tungstenite::tungstenite::protocol::Message,
            SinkError = tokio_tungstenite::tungstenite::Error,
        >,
{
    let (sink, stream) = stream.split();
    sink.send_all(stream.map(move |msg| {
        handle_websocket_message(&req_id, &msg);
        msg
    }))
    .map_err(|e| log::error!("error during websocket stream: {}", e))
    .map(|_| log::info!("disconnect"))
}

fn handle_websocket_message(
    req_id: &str,
    msg: &tokio_tungstenite::tungstenite::protocol::Message,
) {
    // TODO
    log::info!("websocket stream message for {}: {:?}", req_id, msg);
}
