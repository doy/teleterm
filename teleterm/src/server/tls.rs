use crate::prelude::*;

pub struct Server {
    server: super::Server<tokio_tls::TlsStream<tokio::net::TcpStream>>,
    acceptor: Box<
        dyn futures::stream::Stream<
                Item = tokio_tls::Accept<tokio::net::TcpStream>,
                Error = Error,
            > + Send,
    >,
    sock_w: tokio::sync::mpsc::Sender<
        tokio_tls::TlsStream<tokio::net::TcpStream>,
    >,
    accepting_sockets: Vec<tokio_tls::Accept<tokio::net::TcpStream>>,
}

impl Server {
    pub fn new(
        acceptor: Box<
            dyn futures::stream::Stream<
                    Item = tokio_tls::Accept<tokio::net::TcpStream>,
                    Error = Error,
                > + Send,
        >,
        read_timeout: std::time::Duration,
        allowed_login_methods: std::collections::HashSet<
            crate::protocol::AuthType,
        >,
        oauth_configs: std::collections::HashMap<
            crate::protocol::AuthType,
            crate::oauth::Config,
        >,
    ) -> Self {
        let (tls_sock_w, tls_sock_r) = tokio::sync::mpsc::channel(100);
        Self {
            server: super::Server::new(
                Box::new(
                    tls_sock_r.context(crate::error::SocketChannelReceive),
                ),
                read_timeout,
                allowed_login_methods,
                oauth_configs,
            ),
            acceptor,
            sock_w: tls_sock_w,
            accepting_sockets: vec![],
        }
    }
}

impl Server {
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> component_future::Poll<
            (),
            Error,
        >] = &[
        &Self::poll_accept,
        &Self::poll_handshake_connections,
        &Self::poll_server,
    ];

    fn poll_accept(&mut self) -> component_future::Poll<(), Error> {
        if let Some(sock) = component_future::try_ready!(self.acceptor.poll())
        {
            self.accepting_sockets.push(sock);
            Ok(component_future::Async::DidWork)
        } else {
            Err(Error::SocketChannelClosed)
        }
    }

    fn poll_handshake_connections(
        &mut self,
    ) -> component_future::Poll<(), Error> {
        let mut did_work = false;
        let mut not_ready = false;

        let mut i = 0;
        while i < self.accepting_sockets.len() {
            let sock = self.accepting_sockets.get_mut(i).unwrap();
            match sock.poll() {
                Ok(futures::Async::Ready(sock)) => {
                    self.accepting_sockets.swap_remove(i);
                    self.sock_w.try_send(sock).unwrap_or_else(|e| {
                        log::warn!(
                            "failed to send connected tls socket: {}",
                            e
                        );
                    });
                    did_work = true;
                    continue;
                }
                Ok(futures::Async::NotReady) => {
                    not_ready = true;
                }
                Err(e) => {
                    log::warn!("failed to accept tls connection: {}", e);
                    self.accepting_sockets.swap_remove(i);
                    continue;
                }
            }
            i += 1;
        }

        if did_work {
            Ok(component_future::Async::DidWork)
        } else if not_ready {
            Ok(component_future::Async::NotReady)
        } else {
            Ok(component_future::Async::NothingToDo)
        }
    }

    fn poll_server(&mut self) -> component_future::Poll<(), Error> {
        component_future::try_ready!(self.server.poll());
        Ok(component_future::Async::Ready(()))
    }
}

#[must_use = "futures do nothing unless polled"]
impl futures::future::Future for Server {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        component_future::poll_future(self, Self::POLL_FNS)
    }
}
