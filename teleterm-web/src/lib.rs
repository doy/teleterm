mod model;
mod prelude;
mod ws;

use crate::prelude::*;

#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
enum Msg {
    List(seed::fetch::ResponseDataResult<Vec<crate::model::Session>>),
    Refresh,
    StartWatching(String),
    Watch(String, ws::WebSocketEvent),
}

fn init(_: Url, orders: &mut impl Orders<Msg>) -> Init<crate::model::Model> {
    log::trace!("init");
    let model = crate::model::Model::default();
    orders.perform_cmd(model.list());
    Init::new(model)
}

fn update(
    msg: Msg,
    model: &mut crate::model::Model,
    orders: &mut impl Orders<Msg>,
) {
    log::trace!("update");
    match msg {
        Msg::List(sessions) => match sessions {
            Ok(sessions) => {
                log::debug!("got sessions");
                model.update_sessions(sessions);
            }
            Err(e) => {
                log::error!("error getting sessions: {:?}", e);
            }
        },
        Msg::Refresh => {
            log::debug!("refreshing");
            orders.perform_cmd(model.list());
        }
        Msg::StartWatching(id) => {
            log::debug!("watching {}", id);
            model.watch(&id, orders);
        }
        Msg::Watch(id, event) => match event {
            ws::WebSocketEvent::Connected(_) => {
                log::info!("connected");
            }
            ws::WebSocketEvent::Disconnected(_) => {
                log::info!("disconnected");
            }
            ws::WebSocketEvent::Message(msg) => {
                log::info!("message from id {}: {:?}", id, msg);
            }
            ws::WebSocketEvent::Error(e) => {
                log::error!("error from id {}: {:?}", id, e);
            }
        },
    }
}

fn view(model: &crate::model::Model) -> impl View<Msg> {
    log::trace!("view");
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
    console_log::init_with_level(log::Level::Debug).unwrap();
    log::debug!("start");
    seed::App::build(init, update, view).build_and_start();
}
