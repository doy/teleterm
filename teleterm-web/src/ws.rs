use crate::prelude::*;
use wasm_bindgen::JsCast as _;

pub(crate) fn connect(
    url: &str,
    orders: &mut impl Orders<crate::Msg>,
) -> WebSocket {
    let ws = WebSocket::new(url).unwrap();

    register_ws_handler(
        WebSocket::set_onopen,
        crate::Msg::Connected,
        &ws,
        orders,
    );
    register_ws_handler(
        WebSocket::set_onclose,
        crate::Msg::Disconnected,
        &ws,
        orders,
    );
    register_ws_handler(
        WebSocket::set_onmessage,
        crate::Msg::Message,
        &ws,
        orders,
    );
    register_ws_handler(
        WebSocket::set_onerror,
        crate::Msg::Error,
        &ws,
        orders,
    );

    ws
}

fn register_ws_handler<T, F>(
    ws_cb_setter: fn(&WebSocket, Option<&js_sys::Function>),
    msg: F,
    ws: &web_sys::WebSocket,
    orders: &mut impl Orders<crate::Msg>,
) where
    T: wasm_bindgen::convert::FromWasmAbi + 'static,
    F: Fn(T) -> crate::Msg + 'static,
{
    let (app, msg_mapper) = (orders.clone_app(), orders.msg_mapper());

    let closure = Closure::new(move |data| {
        app.update(msg_mapper(msg(data)));
    });

    ws_cb_setter(ws, Some(closure.as_ref().unchecked_ref()));
    closure.forget();
}
