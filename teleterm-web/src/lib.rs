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
    Login(String),
    LoggedIn(seed::fetch::ResponseDataResult<crate::protocol::LoginResponse>),
    Refresh,
    List(seed::fetch::ResponseDataResult<Vec<crate::protocol::Session>>),
    StartWatching(String),
    Watch(String, crate::ws::WebSocketEvent),
    StopWatching,
    Logout,
    LoggedOut(seed::fetch::FetchObject<()>),
}

fn after_mount(
    _url: Url,
    orders: &mut impl Orders<Msg>,
) -> AfterMount<crate::model::Model> {
    log::trace!("after_mount");
    AfterMount::new(crate::model::Model::new(
        crate::config::Config::load(),
        orders,
    ))
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
    seed::App::builder(update, view)
        .after_mount(after_mount)
        .build_and_start();
}
