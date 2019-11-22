mod prelude;
mod ws;

use crate::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

const LIST_URL: &str = "http://127.0.0.1:4145/list";
const WATCH_URL: &str = "ws://127.0.0.1:4145/watch";

#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
enum Msg {
    List(seed::fetch::ResponseDataResult<Vec<Session>>),
    Refresh,
    StartWatching(String),
    Watch(ws::WebSocketEvent),
}

struct WatchConn {
    id: String,
    #[allow(dead_code)] // no idea why it thinks this is dead
    ws: WebSocket,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct Session {
    id: String,
    username: String,
}

#[derive(Default)]
struct Model {
    sessions: Vec<Session>,
    watch_conn: Option<WatchConn>,
}

impl Model {
    fn list(&self) -> impl futures::Future<Item = Msg, Error = Msg> {
        seed::Request::new(LIST_URL).fetch_json_data(Msg::List)
    }

    fn watch(&mut self, id: &str, orders: &mut impl Orders<Msg>) {
        let ws = ws::connect(WATCH_URL, Msg::Watch, orders);
        self.watch_conn = Some(WatchConn {
            id: id.to_string(),
            ws,
        })
    }
}

fn init(_: Url, orders: &mut impl Orders<Msg>) -> Init<Model> {
    log("init");
    let model = Model::default();
    orders.perform_cmd(model.list());
    Init::new(model)
}

fn update(msg: Msg, model: &mut Model, orders: &mut impl Orders<Msg>) {
    log("update");
    match msg {
        Msg::List(sessions) => match sessions {
            Ok(sessions) => {
                log("got sessions");
                model.sessions = sessions;
            }
            Err(e) => {
                log(&format!("error getting sessions: {:?}", e));
            }
        },
        Msg::Refresh => {
            orders.perform_cmd(model.list());
        }
        Msg::StartWatching(id) => {
            log(&format!("watching {}", id));
            model.watch(&id, orders);
        }
        Msg::Watch(event) => match event {
            ws::WebSocketEvent::Connected(_) => {
                log("connected");
            }
            ws::WebSocketEvent::Disconnected(_) => {
                log("disconnected");
                model.watch_conn = None;
            }
            ws::WebSocketEvent::Message(msg) => {
                log(&format!(
                    "message from id {}: {:?}",
                    model.watch_conn.as_ref().unwrap().id,
                    msg
                ));
            }
            ws::WebSocketEvent::Error(e) => {
                log(&format!(
                    "error from id {}: {:?}",
                    model.watch_conn.as_ref().unwrap().id,
                    e
                ));
            }
        },
    }
}

fn view(model: &Model) -> impl View<Msg> {
    log("view");
    let mut list = vec![];
    for session in &model.sessions {
        list.push(seed::li![seed::button![
            simple_ev(Ev::Click, Msg::StartWatching(session.id.clone())),
            format!("{}: {}", session.username, session.id),
        ]]);
    }
    vec![
        seed::h1!["it's a seed app"],
        seed::ul![list],
        seed::button![simple_ev(Ev::Click, Msg::Refresh), "refresh"],
    ]
}

#[wasm_bindgen(start)]
pub fn start() {
    log("start");
    seed::App::build(init, update, view).build_and_start();
}
