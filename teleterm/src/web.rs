mod list;
mod view;
mod watch;
mod ws;

use crate::prelude::*;

use gotham::router::builder::{DefineSingleRoute as _, DrawRoutes as _};
use gotham::state::FromState as _;

#[derive(Clone, serde::Serialize, gotham_derive::StateData)]
struct Config {
    title: String,
    server_address: (String, std::net::SocketAddr),
    public_address: String,
}

pub struct Server {
    server: Box<dyn futures::Future<Item = (), Error = ()> + Send>,
}

impl Server {
    pub fn new(
        listen_address: std::net::SocketAddr,
        public_address: String,
        server_address: (String, std::net::SocketAddr),
    ) -> Self {
        let data = Config {
            title: "teleterm".to_string(),
            server_address,
            public_address,
        };
        Self {
            server: Box::new(gotham::init_server(
                listen_address,
                router(&data),
            )),
        }
    }
}

impl futures::Future for Server {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        self.server.poll().map_err(|_| unreachable!())
    }
}

fn router(data: &Config) -> impl gotham::handler::NewHandler {
    let (chain, pipeline) = gotham::pipeline::single::single_pipeline(
        gotham::pipeline::new_pipeline()
            .add(gotham::middleware::state::StateMiddleware::new(
                data.clone(),
            ))
            .build(),
    );
    gotham::router::builder::build_router(chain, pipeline, |route| {
        route
            .get("/")
            .to(serve_template("text/html", view::INDEX_HTML_TMPL_NAME));
        route.get("/teleterm_web.js").to(serve_static(
            "application/javascript",
            &view::TELETERM_WEB_JS,
        ));
        route
            .get("/teleterm_web_bg.wasm")
            .to(serve_static("application/wasm", &view::TELETERM_WEB_WASM));
        route
            .get("/teleterm.css")
            .to(serve_static("text/css", &view::TELETERM_CSS));
        route.get("/list").to(list::run);
        route
            .get("/watch")
            .with_query_string_extractor::<watch::QueryParams>()
            .to(watch::run);
    })
}

fn serve_static(
    content_type: &'static str,
    s: &'static [u8],
) -> impl gotham::handler::Handler + Copy {
    move |state| {
        let response = hyper::Response::builder()
            .header("Content-Type", content_type)
            .body(hyper::Body::from(s))
            .unwrap();
        (state, response)
    }
}

fn serve_template(
    content_type: &'static str,
    name: &'static str,
) -> impl gotham::handler::Handler + Copy {
    move |state| {
        let config = Config::borrow_from(&state);
        let rendered = view::HANDLEBARS.render(name, &config).unwrap();
        let response = hyper::Response::builder()
            .header("Content-Type", content_type)
            .body(hyper::Body::from(rendered))
            .unwrap();
        (state, response)
    }
}
