// from https://github.com/gotham-rs/gotham/blob/master/examples/websocket/src/main.rs

use futures::Future as _;

const PROTO_WEBSOCKET: &str = "websocket";
const SEC_WEBSOCKET_KEY: &str = "Sec-WebSocket-Key";
const SEC_WEBSOCKET_ACCEPT: &str = "Sec-WebSocket-Accept";

pub fn requested(headers: &hyper::HeaderMap) -> bool {
    headers.get(hyper::header::UPGRADE)
        == Some(&hyper::header::HeaderValue::from_static(PROTO_WEBSOCKET))
}

pub fn accept(
    headers: &hyper::HeaderMap,
    body: hyper::Body,
) -> Result<
    (
        hyper::Response<hyper::Body>,
        impl futures::Future<
            Item = tokio_tungstenite::WebSocketStream<
                hyper::upgrade::Upgraded,
            >,
            Error = hyper::Error,
        >,
    ),
    (),
> {
    let res = response(headers)?;
    let ws = body.on_upgrade().map(|upgraded| {
        tokio_tungstenite::WebSocketStream::from_raw_socket(
            upgraded,
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
    });

    Ok((res, ws))
}

fn response(
    headers: &hyper::HeaderMap,
) -> Result<hyper::Response<hyper::Body>, ()> {
    let key = headers.get(SEC_WEBSOCKET_KEY).ok_or(())?;

    Ok(hyper::Response::builder()
        .header(hyper::header::UPGRADE, PROTO_WEBSOCKET)
        .header(hyper::header::CONNECTION, "upgrade")
        .header(SEC_WEBSOCKET_ACCEPT, accept_key(key.as_bytes()))
        .status(hyper::StatusCode::SWITCHING_PROTOCOLS)
        .body(hyper::Body::empty())
        .unwrap())
}

fn accept_key(key: &[u8]) -> String {
    const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut sha1 = sha1::Sha1::default();
    sha1.update(key);
    sha1.update(WS_GUID);
    base64::encode(&sha1.digest().bytes())
}
