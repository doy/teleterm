use crate::prelude::*;

pub struct Server {
    server: super::Server<tokio_tls::TlsStream<tokio::net::TcpStream>>,
    sock_r:
        tokio::sync::mpsc::Receiver<tokio_tls::Accept<tokio::net::TcpStream>>,
    sock_w: tokio::sync::mpsc::Sender<
        tokio_tls::TlsStream<tokio::net::TcpStream>,
    >,
    accepting_sockets: Vec<tokio_tls::Accept<tokio::net::TcpStream>>,
}

impl Server {
    pub fn new(
        buffer_size: usize,
        read_timeout: std::time::Duration,
        sock_r: tokio::sync::mpsc::Receiver<
            tokio_tls::Accept<tokio::net::TcpStream>,
        >,
    ) -> Self {
        let (tls_sock_w, tls_sock_r) = tokio::sync::mpsc::channel(100);
        Self {
            server: super::Server::new(buffer_size, read_timeout, tls_sock_r),
            sock_r,
            sock_w: tls_sock_w,
            accepting_sockets: vec![],
        }
    }
}

impl Server {
    const POLL_FNS: &'static [&'static dyn for<'a> Fn(
        &'a mut Self,
    ) -> Result<
        crate::component_future::Poll<()>,
    >] = &[
        &Self::poll_new_connections,
        &Self::poll_handshake_connections,
        &Self::poll_server,
    ];

    fn poll_new_connections(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        match self
            .sock_r
            .poll()
            .context(crate::error::SocketChannelReceive)?
        {
            futures::Async::Ready(Some(sock)) => {
                self.accepting_sockets.push(sock);
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::Ready(None) => Err(Error::SocketChannelClosed),
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }

    fn poll_handshake_connections(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
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
            Ok(crate::component_future::Poll::DidWork)
        } else if not_ready {
            Ok(crate::component_future::Poll::NotReady)
        } else {
            Ok(crate::component_future::Poll::NothingToDo)
        }
    }

    fn poll_server(&mut self) -> Result<crate::component_future::Poll<()>> {
        match self.server.poll()? {
            futures::Async::Ready(()) => {
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }
}

#[must_use = "futures do nothing unless polled"]
impl futures::future::Future for Server {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        crate::component_future::poll_future(self, Self::POLL_FNS)
    }
}
