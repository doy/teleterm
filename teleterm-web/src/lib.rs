use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast as _;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

#[wasm_bindgen(start)]
pub fn main() -> Result<(), JsValue> {
    log("loaded");

    let ws = web_sys::WebSocket::new("ws://127.0.0.1:4145/watch")?;

    let msg_cb =
        Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
            log(&format!("message {:?}", event));
        }) as Box<dyn FnMut(web_sys::MessageEvent)>);
    ws.set_onmessage(Some(msg_cb.as_ref().unchecked_ref()));
    msg_cb.forget();

    let err_cb = Closure::wrap(Box::new(move |event: web_sys::ErrorEvent| {
        log(&format!("error {:?}", event));
    }) as Box<dyn FnMut(web_sys::ErrorEvent)>);
    ws.set_onerror(Some(err_cb.as_ref().unchecked_ref()));
    err_cb.forget();

    let cloned_ws = ws.clone();
    let open_cb = Closure::wrap(Box::new(move |_| {
        log("opened");
        match cloned_ws.send_with_str("ping1") {
            Ok(_) => log("sent ping1 successfully"),
            Err(e) => {
                log(&format!("error sending ping: {:?}", e));
                return;
            }
        }
        match cloned_ws.send_with_str("ping2") {
            Ok(_) => log("sent ping2 successfully"),
            Err(e) => {
                log(&format!("error sending ping: {:?}", e));
            }
        }
    }) as Box<dyn FnMut(JsValue)>);
    ws.set_onopen(Some(open_cb.as_ref().unchecked_ref()));
    open_cb.forget();

    Ok(())
}
