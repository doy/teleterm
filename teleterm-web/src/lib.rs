mod model;
mod prelude;
mod ws;

use crate::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
enum Msg {
    List(seed::fetch::ResponseDataResult<Vec<crate::model::Session>>),
    Refresh,
    StartWatching(String),
    Watch(ws::WebSocketEvent),
}

fn init(_: Url, orders: &mut impl Orders<Msg>) -> Init<crate::model::Model> {
    log("init");
    let model = crate::model::Model::default();
    orders.perform_cmd(model.list());
    Init::new(model)
}

fn update(
    msg: Msg,
    model: &mut crate::model::Model,
    orders: &mut impl Orders<Msg>,
) {
    log("update");
    match msg {
        Msg::List(sessions) => match sessions {
            Ok(sessions) => {
                log("got sessions");
                model.update_sessions(sessions);
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
                model.watch_disconnect();
            }
            ws::WebSocketEvent::Message(msg) => {
                log(&format!(
                    "message from id {}: {:?}",
                    model.watch_id().unwrap(),
                    msg
                ));
            }
            ws::WebSocketEvent::Error(e) => {
                log(&format!(
                    "error from id {}: {:?}",
                    model.watch_id().unwrap(),
                    e
                ));
            }
        },
    }
}

fn view(model: &crate::model::Model) -> impl View<Msg> {
    log("view");
    let mut list = vec![];
    for session in model.sessions() {
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
