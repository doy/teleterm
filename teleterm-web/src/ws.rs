use crate::prelude::*;
use wasm_bindgen::JsCast as _;

#[derive(Clone)]
pub(crate) enum WebSocketEvent {
    Connected(JsValue),
    Disconnected(JsValue),
    Message(MessageEvent),
    Error(ErrorEvent),
}

pub(crate) fn connect(
    url: &str,
    id: &str,
    msg: fn(String, WebSocketEvent) -> crate::Msg,
    orders: &mut impl Orders<crate::Msg>,
) -> WebSocket {
    let ws = WebSocket::new(url).unwrap();

    register_ws_handler(
        id,
        WebSocket::set_onopen,
        WebSocketEvent::Connected,
        msg,
        &ws,
        orders,
    );
    register_ws_handler(
        id,
        WebSocket::set_onclose,
        WebSocketEvent::Disconnected,
        msg,
        &ws,
        orders,
    );
    register_ws_handler(
        id,
        WebSocket::set_onmessage,
        WebSocketEvent::Message,
        msg,
        &ws,
        orders,
    );
    register_ws_handler(
        id,
        WebSocket::set_onerror,
        WebSocketEvent::Error,
        msg,
        &ws,
        orders,
    );

    ws
}

fn register_ws_handler<T, F>(
    id: &str,
    ws_cb_setter: fn(&WebSocket, Option<&js_sys::Function>),
    msg: F,
    ws_msg: fn(String, WebSocketEvent) -> crate::Msg,
    ws: &web_sys::WebSocket,
    orders: &mut impl Orders<crate::Msg>,
) where
    T: wasm_bindgen::convert::FromWasmAbi + 'static,
    F: Fn(T) -> WebSocketEvent + 'static,
{
    let (app, msg_mapper) = (orders.clone_app(), orders.msg_mapper());

    let id = id.to_string();
    let closure = Closure::new(move |data| {
        app.update(msg_mapper(ws_msg(id.clone(), msg(data))));
    });

    ws_cb_setter(ws, Some(closure.as_ref().unchecked_ref()));
    closure.forget();
}
