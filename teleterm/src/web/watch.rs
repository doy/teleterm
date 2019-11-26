use crate::prelude::*;

use gotham::state::FromState as _;
use tokio_tungstenite::tungstenite;

#[derive(
    serde::Deserialize,
    gotham_derive::StateData,
    gotham_derive::StaticResponseExtender,
)]
pub struct QueryParams {
    id: String,
}

pub fn run(
    mut state: gotham::state::State,
) -> (gotham::state::State, hyper::Response<hyper::Body>) {
    let session = gotham::middleware::session::SessionData::<
        crate::web::SessionData,
    >::borrow_from(&state);
    let auth = if let Some(login) = &session.login {
        login.auth.clone()
    } else {
        return (
            state,
            hyper::Response::builder()
                .status(hyper::StatusCode::FORBIDDEN)
                .body(hyper::Body::empty())
                .unwrap(),
        );
    };

    let body = hyper::Body::take_from(&mut state);
    let headers = hyper::HeaderMap::take_from(&mut state);
    let config = crate::web::Config::borrow_from(&state);

    if crate::web::ws::requested(&headers) {
        let (response, stream) = match crate::web::ws::accept(&headers, body)
        {
            Ok(res) => res,
            Err(_) => {
                log::error!("failed to accept websocket request");
                return (
                    state,
                    hyper::Response::builder()
                        .status(hyper::StatusCode::BAD_REQUEST)
                        .body(hyper::Body::empty())
                        .unwrap(),
                );
            }
        };

        let query_params = QueryParams::borrow_from(&state);

        let (_, address) = config.server_address;
        let connector: crate::client::Connector<_> = Box::new(move || {
            Box::new(
                tokio::net::tcp::TcpStream::connect(&address)
                    .context(crate::error::Connect { address }),
            )
        });
        let client = crate::client::Client::watch(
            "teleterm-web",
            connector,
            &auth,
            &query_params.id,
        );

        tokio::spawn(
            Connection::new(
                gotham::state::request_id(&state),
                client,
                ConnectionState::Connecting(Box::new(
                    stream.context(crate::error::WebSocketAccept),
                )),
            )
            .map_err(|e| log::error!("{}", e)),
        );

        (state, response)
    } else {
        (
            state,
            hyper::Response::new(hyper::Body::from(
                "non-websocket request to websocket endpoint",
            )),
        )
    }
}

type WebSocketConnectionFuture = Box<
    dyn futures::Future<
            Item = tokio_tungstenite::WebSocketStream<
                hyper::upgrade::Upgraded,
            >,
            Error = Error,
        > + Send,
>;
type MessageSink = Box<
    dyn futures::Sink<SinkItem = tungstenite::Message, SinkError = Error>
        + Send,
>;
type MessageStream = Box<
    dyn futures::Stream<Item = tungstenite::Message, Error = Error> + Send,
>;

enum SenderState {
    Temporary,
    Connected(MessageSink),
    Sending(
        Box<dyn futures::Future<Item = MessageSink, Error = Error> + Send>,
    ),
    Flushing(
        Box<dyn futures::Future<Item = MessageSink, Error = Error> + Send>,
    ),
}

enum ConnectionState {
    Connecting(WebSocketConnectionFuture),
    Connected(SenderState, MessageStream),
}

impl ConnectionState {
    fn sink(&mut self) -> Option<&mut MessageSink> {
        match self {
            Self::Connected(sender, _) => match sender {
                SenderState::Connected(sink) => Some(sink),
                _ => None,
            },
            _ => None,
        }
    }

    fn send(&mut self, msg: tungstenite::Message) {
        match self {
            Self::Connected(sender, _) => {
                let fut =
                    match std::mem::replace(sender, SenderState::Temporary) {
                        SenderState::Connected(sink) => sink.send(msg),
                        _ => unreachable!(),
                    };
                *sender = SenderState::Sending(Box::new(fut));
            }
            _ => unreachable!(),
        }
    }
}

struct Connection<
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
> {
    id: String,
    client: crate::client::Client<S>,
    conn: ConnectionState,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Connection<S>
{
    fn new(
        id: &str,
        client: crate::client::Client<S>,
        conn: ConnectionState,
    ) -> Self {
        Self {
            client,
            id: id.to_string(),
            conn,
        }
    }

    fn handle_client_message(
        &mut self,
        msg: &crate::protocol::Message,
    ) -> Result<Option<tungstenite::Message>> {
        match msg {
            crate::protocol::Message::TerminalOutput { .. }
            | crate::protocol::Message::Disconnected
            | crate::protocol::Message::Resize { .. } => {
                let json = serde_json::to_string(msg)
                    .context(crate::error::SerializeMessage)?;
                Ok(Some(tungstenite::Message::Text(json)))
            }
            _ => Ok(None),
        }
    }

    fn handle_websocket_message(
        &mut self,
        msg: &tungstenite::Message,
    ) -> Result<()> {
        // TODO
        log::info!("websocket stream message for {}: {:?}", self.id, msg);
        Ok(())
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    Connection<S>
{
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> component_future::Poll<
            (),
            Error,
        >] = &[&Self::poll_client, &Self::poll_websocket_stream];

    fn poll_client(&mut self) -> component_future::Poll<(), Error> {
        // don't start up the client until the websocket connection is fully
        // established and isn't busy
        if self.conn.sink().is_none() {
            return Ok(component_future::Async::NothingToDo);
        };

        match component_future::try_ready!(self.client.poll()).unwrap() {
            crate::client::Event::Disconnect => {
                // TODO: better reconnect handling?
                return Ok(component_future::Async::Ready(()));
            }
            crate::client::Event::Connect => {}
            crate::client::Event::ServerMessage(msg) => {
                if let Some(msg) = self.handle_client_message(&msg)? {
                    self.conn.send(msg);
                }
            }
        }
        Ok(component_future::Async::DidWork)
    }

    fn poll_websocket_stream(&mut self) -> component_future::Poll<(), Error> {
        match &mut self.conn {
            ConnectionState::Connecting(fut) => {
                let stream = component_future::try_ready!(fut.poll());
                let (sink, stream) = stream.split();
                self.conn = ConnectionState::Connected(
                    SenderState::Connected(Box::new(
                        sink.sink_map_err(|e| Error::WebSocket { source: e }),
                    )),
                    Box::new(stream.context(crate::error::WebSocket)),
                );
                Ok(component_future::Async::DidWork)
            }
            ConnectionState::Connected(sender, stream) => match sender {
                SenderState::Temporary => unreachable!(),
                SenderState::Connected(_) => {
                    if let Some(msg) =
                        component_future::try_ready!(stream.poll())
                    {
                        self.handle_websocket_message(&msg)?;
                        Ok(component_future::Async::DidWork)
                    } else {
                        log::info!("disconnect for {}", self.id);
                        Ok(component_future::Async::Ready(()))
                    }
                }
                SenderState::Sending(fut) => {
                    let sink = component_future::try_ready!(fut.poll());
                    *sender = SenderState::Flushing(Box::new(sink.flush()));
                    Ok(component_future::Async::DidWork)
                }
                SenderState::Flushing(fut) => {
                    let sink = component_future::try_ready!(fut.poll());
                    *sender = SenderState::Connected(Box::new(sink));
                    Ok(component_future::Async::DidWork)
                }
            },
        }
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>
    futures::Future for Connection<S>
{
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        component_future::poll_future(self, Self::POLL_FNS)
    }
}
