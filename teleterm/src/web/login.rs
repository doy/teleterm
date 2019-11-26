use crate::prelude::*;

use gotham::state::FromState as _;

#[derive(
    serde::Deserialize,
    gotham_derive::StateData,
    gotham_derive::StaticResponseExtender,
)]
pub struct QueryParams {
    username: Option<String>,
}

#[derive(serde::Serialize)]
struct Response {
    username: Option<String>,
}

pub fn run(
    mut state: gotham::state::State,
) -> (gotham::state::State, hyper::Response<hyper::Body>) {
    let username = {
        let query_params = QueryParams::borrow_from(&state);
        query_params.username.clone()
    };
    let username = if let Some(username) = username {
        username
    } else {
        return (
            state,
            hyper::Response::builder()
                .status(hyper::StatusCode::BAD_REQUEST)
                .body(hyper::Body::empty())
                .unwrap(),
        );
    };

    let config = crate::web::Config::borrow_from(&state);

    let (_, address) = config.server_address;
    let connector: crate::client::Connector<_> = Box::new(move || {
        Box::new(
            tokio::net::tcp::TcpStream::connect(&address)
                .context(crate::error::Connect { address }),
        )
    });
    let auth = crate::protocol::Auth::plain(&username);
    let client = crate::client::Client::raw("teleterm-web", connector, &auth);

    let (w_login, r_login) = tokio::sync::oneshot::channel();

    tokio::spawn(
        Client::new(client, auth, w_login)
            // XXX if this happens, we might not have sent anything on the
            // channel, and so the wait might block forever
            .map_err(|e| log::error!("error logging in: {}", e)),
    );

    let session = gotham::middleware::session::SessionData::<
        crate::web::SessionData,
    >::borrow_mut_from(&mut state);

    match r_login.wait().unwrap() {
        Ok(login) => {
            session.login = Some(login);
        }
        Err(e) => {
            session.login = None;
            log::error!("error logging in: {}", e);
            return (
                state,
                hyper::Response::builder()
                    .status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
                    .body(hyper::Body::empty())
                    .unwrap(),
            );
        }
    }

    (
        state,
        hyper::Response::new(hyper::Body::from(
            serde_json::to_string(&Response {
                username: Some(username),
            })
            .unwrap(),
        )),
    )
}

pub(crate) struct Client<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    client: crate::client::Client<S>,
    auth: crate::protocol::Auth,
    w_login: Option<tokio::sync::oneshot::Sender<Result<super::LoginState>>>,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Client<S>
{
    pub(crate) fn new(
        client: crate::client::Client<S>,
        auth: crate::protocol::Auth,
        w_login: tokio::sync::oneshot::Sender<Result<super::LoginState>>,
    ) -> Self {
        Self {
            client,
            auth,
            w_login: Some(w_login),
        }
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Client<S>
{
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> component_future::Poll<
            (),
            Error,
        >] = &[&Self::poll_client];

    fn poll_client(&mut self) -> component_future::Poll<(), Error> {
        let res =
            match component_future::try_ready!(self.client.poll()).unwrap() {
                crate::client::Event::ServerMessage(msg) => match msg {
                    crate::protocol::Message::Disconnected => {
                        Err(Error::ServerDisconnected)
                    }
                    crate::protocol::Message::Error { msg } => {
                        Err(Error::Server { message: msg })
                    }
                    crate::protocol::Message::LoggedIn { username } => {
                        Ok(super::LoginState {
                            auth: self.auth.clone(),
                            username,
                        })
                    }
                    _ => {
                        return Ok(component_future::Async::DidWork);
                    }
                },
                _ => unreachable!(),
            };
        self.w_login.take().unwrap().send(res).unwrap();
        Ok(component_future::Async::Ready(()))
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    futures::Future for Client<S>
{
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        component_future::poll_future(self, Self::POLL_FNS)
    }
}
