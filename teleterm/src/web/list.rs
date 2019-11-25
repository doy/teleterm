use crate::prelude::*;

use gotham::state::FromState as _;

pub fn run(
    state: gotham::state::State,
) -> (gotham::state::State, hyper::Response<hyper::Body>) {
    let session = gotham::middleware::session::SessionData::<
        crate::web::SessionData,
    >::borrow_from(&state);
    let auth = if let Some(username) = &session.username {
        crate::protocol::Auth::plain(username)
    } else {
        return (
            state,
            hyper::Response::builder()
                .status(hyper::StatusCode::FORBIDDEN)
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
    let client =
        crate::client::Client::list("teleterm-web", connector, &auth);

    let (w_sessions, r_sessions) = tokio::sync::oneshot::channel();

    tokio::spawn(
        Lister::new(client, w_sessions)
            .map_err(|e| log::warn!("error listing: {}", e)),
    );

    match r_sessions.wait().unwrap() {
        Ok(sessions) => {
            let body = serde_json::to_string(&sessions).unwrap();
            (state, hyper::Response::new(hyper::Body::from(body)))
        }
        Err(e) => {
            log::warn!("error retrieving sessions: {}", e);
            (
                state,
                hyper::Response::new(hyper::Body::from(format!(
                    "error retrieving sessions: {}",
                    e
                ))),
            )
        }
    }
}

struct Lister<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    client: crate::client::Client<S>,
    w_sessions: Option<
        tokio::sync::oneshot::Sender<Result<Vec<crate::protocol::Session>>>,
    >,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Lister<S>
{
    fn new(
        client: crate::client::Client<S>,
        w_sessions: tokio::sync::oneshot::Sender<
            Result<Vec<crate::protocol::Session>>,
        >,
    ) -> Self {
        Self {
            client,
            w_sessions: Some(w_sessions),
        }
    }

    fn server_message(
        &mut self,
        msg: crate::protocol::Message,
    ) -> Option<Result<Vec<crate::protocol::Session>>> {
        match msg {
            crate::protocol::Message::Sessions { sessions } => {
                Some(Ok(sessions))
            }
            crate::protocol::Message::Disconnected => {
                Some(Err(Error::ServerDisconnected))
            }
            crate::protocol::Message::Error { msg } => {
                Some(Err(Error::Server { message: msg }))
            }
            msg => Some(Err(crate::error::Error::UnexpectedMessage {
                message: msg,
            })),
        }
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Lister<S>
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
        match component_future::try_ready!(self.client.poll()).unwrap() {
            crate::client::Event::Disconnect => {
                let res = Err(Error::ServerDisconnected);
                self.w_sessions.take().unwrap().send(res).unwrap();
                return Ok(component_future::Async::Ready(()));
            }
            crate::client::Event::Connect => {
                self.client
                    .send_message(crate::protocol::Message::list_sessions());
            }
            crate::client::Event::ServerMessage(msg) => {
                if let Some(res) = self.server_message(msg) {
                    self.w_sessions.take().unwrap().send(res).unwrap();
                    return Ok(component_future::Async::Ready(()));
                }
            }
        }
        Ok(component_future::Async::DidWork)
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    futures::Future for Lister<S>
{
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        component_future::poll_future(self, Self::POLL_FNS)
    }
}
