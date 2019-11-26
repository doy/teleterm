mod disk_session;
mod list;
mod login;
mod logout;
mod view;
mod watch;
mod ws;

use crate::prelude::*;

use gotham::router::builder::{DefineSingleRoute as _, DrawRoutes as _};
use gotham::state::FromState as _;

#[derive(Clone, gotham_derive::StateData)]
struct Config {
    server_address: (String, std::net::SocketAddr),
    public_address: String,
    allowed_login_methods:
        std::collections::HashSet<crate::protocol::AuthType>,
    oauth_configs: std::collections::HashMap<
        crate::protocol::AuthType,
        crate::oauth::Config,
    >,
}

impl Config {
    fn allowed_oauth_login_methods(
        &self,
    ) -> impl Iterator<Item = crate::protocol::AuthType> + '_ {
        self.allowed_login_methods
            .iter()
            .copied()
            .filter(|ty| ty.is_oauth())
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub(crate) struct LoginState {
    auth: crate::protocol::Auth,
    username: String,
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
struct SessionData {
    login: Option<LoginState>,
}

#[derive(Debug, serde::Serialize)]
struct WebConfig<'a> {
    username: Option<&'a str>,
    public_address: &'a str,
    allowed_login_methods:
        &'a std::collections::HashSet<crate::protocol::AuthType>,
    oauth_login_urls:
        std::collections::HashMap<crate::protocol::AuthType, String>,
}

impl<'a> WebConfig<'a> {
    fn new(config: &'a Config, session: &'a SessionData) -> Result<Self> {
        let mut oauth_login_urls = std::collections::HashMap::new();
        for ty in config.allowed_oauth_login_methods() {
            let oauth_config = config
                .oauth_configs
                .get(&ty)
                .context(crate::error::AuthTypeMissingOauthConfig { ty })?;
            let client = ty.oauth_client(oauth_config, None).unwrap();
            oauth_login_urls.insert(ty, client.generate_authorize_url());
        }
        Ok(Self {
            username: session
                .login
                .as_ref()
                .map(|login| login.username.as_str()),
            public_address: &config.public_address,
            allowed_login_methods: &config.allowed_login_methods,
            oauth_login_urls,
        })
    }
}

pub struct Server {
    server: Box<dyn futures::Future<Item = (), Error = ()> + Send>,
}

impl Server {
    pub fn new(
        listen_address: std::net::SocketAddr,
        public_address: String,
        server_address: (String, std::net::SocketAddr),
        allowed_login_methods: std::collections::HashSet<
            crate::protocol::AuthType,
        >,
        oauth_configs: std::collections::HashMap<
            crate::protocol::AuthType,
            crate::oauth::Config,
        >,
    ) -> Self {
        let data = Config {
            server_address,
            public_address,
            allowed_login_methods,
            oauth_configs,
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
            .add(
                gotham::middleware::session::NewSessionMiddleware::new(
                    disk_session::DiskSession,
                )
                .insecure()
                .with_cookie_name("teleterm")
                .with_session_type::<SessionData>(),
            )
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
        route
            .get("/login")
            .with_query_string_extractor::<login::QueryParams>()
            .to(login::run);
        route.get("/logout").to(logout::run);
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
        let session = gotham::middleware::session::SessionData::<
            crate::web::SessionData,
        >::borrow_from(&state);
        let web_config = match WebConfig::new(config, session) {
            Ok(config) => config,
            Err(e) => {
                // this means that the server configuration is incorrect, and
                // there's nothing the client can do about it
                return (
                    state,
                    hyper::Response::builder()
                        .status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
                        .body(hyper::Body::from(format!("{}", e)))
                        .unwrap(),
                );
            }
        };
        let rendered = view::HANDLEBARS.render(name, &web_config).unwrap();
        let response = hyper::Response::builder()
            .header("Content-Type", content_type)
            .body(hyper::Body::from(rendered))
            .unwrap();
        (state, response)
    }
}
