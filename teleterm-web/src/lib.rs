mod config;
mod model;
mod prelude;
mod protocol;
mod views;
mod ws;

use crate::prelude::*;

#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
enum Msg {
    Login,
    LoggedIn(seed::fetch::ResponseDataResult<()>),
    List(seed::fetch::ResponseDataResult<Vec<crate::protocol::Session>>),
    Refresh,
    StartWatching(String),
    Watch(String, crate::ws::WebSocketEvent),
    StopWatching,
}

fn init(_: Url, orders: &mut impl Orders<Msg>) -> Init<crate::model::Model> {
    log::trace!("init");
    orders.send_msg(Msg::Login);
    Init::new(crate::model::Model::new(crate::config::Config::load()))
}

fn update(
    msg: Msg,
    model: &mut crate::model::Model,
    orders: &mut impl Orders<Msg>,
) {
    log::trace!("update");
    model.update(msg, orders);
}

fn view(model: &crate::model::Model) -> impl View<Msg> {
    log::trace!("view");
    crate::views::page::render(model)
}

#[wasm_bindgen(start)]
pub fn start() {
    console_log::init_with_level(log::Level::Debug).unwrap();
    log::debug!("start");
    seed::App::build(init, update, view).build_and_start();
}
