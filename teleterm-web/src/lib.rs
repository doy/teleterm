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
    Watch(ws::WebSocketEvent),
}

struct Model {
    watch_conn: WebSocket,
}

fn init(_: Url, orders: &mut impl Orders<Msg>) -> Init<Model> {
    log("init");
    let watch_conn = ws::connect(WATCH_URL, Msg::Watch, orders);
    Init::new(Model { watch_conn })
}

fn update(msg: Msg, model: &mut Model, _orders: &mut impl Orders<Msg>) {
    log("update");
    match msg {
        Msg::Watch(event) => match event {
            ws::WebSocketEvent::Connected(_) => {
                log("connected");
                match model.watch_conn.send_with_str("ping1") {
                    Ok(_) => log("sent ping1 successfully"),
                    Err(e) => {
                        log(&format!("error sending ping: {:?}", e));
                        return;
                    }
                }
                match model.watch_conn.send_with_str("ping2") {
                    Ok(_) => log("sent ping2 successfully"),
                    Err(e) => {
                        log(&format!("error sending ping: {:?}", e));
                    }
                }
            }
            ws::WebSocketEvent::Disconnected(_) => {
                log("disconnected");
            }
            ws::WebSocketEvent::Message(msg) => {
                log(&format!("message {:?}", msg));
            }
            ws::WebSocketEvent::Error(e) => {
                log(&format!("error {:?}", e));
            }
        },
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
