mod prelude;
mod ws;

use crate::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

const WATCH_URL: &str = "ws://127.0.0.1:4145/watch";

#[derive(Clone)]
enum Msg {
    Connected(JsValue),
    Disconnected(JsValue),
    Message(MessageEvent),
    Error(JsValue),
}

struct Model {
    ws: WebSocket,
}

fn init(_: Url, orders: &mut impl Orders<Msg>) -> Init<Model> {
    log("init");
    let ws = ws::connect(WATCH_URL, orders);
    log("created ws");
    Init::new(Model { ws })
}

fn update(msg: Msg, model: &mut Model, _orders: &mut impl Orders<Msg>) {
    log("update");
    match msg {
        Msg::Connected(_) => {
            log("connected");
            match model.ws.send_with_str("ping1") {
                Ok(_) => log("sent ping1 successfully"),
                Err(e) => {
                    log(&format!("error sending ping: {:?}", e));
                    return;
                }
            }
            match model.ws.send_with_str("ping2") {
                Ok(_) => log("sent ping2 successfully"),
                Err(e) => {
                    log(&format!("error sending ping: {:?}", e));
                }
            }
        }
        Msg::Disconnected(_) => {
            log("disconnected");
        }
        Msg::Message(msg) => {
            log(&format!("message {:?}", msg));
        }
        Msg::Error(e) => {
            log(&format!("error {:?}", e));
        }
    }
}

fn view(_model: &Model) -> impl View<Msg> {
    log("view");
    vec![seed::h1!["it's a seed app"]]
}

#[wasm_bindgen(start)]
pub fn start() {
    log("start");
    seed::App::build(init, update, view).build_and_start();
}
