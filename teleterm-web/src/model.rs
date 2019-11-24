use crate::prelude::*;

struct WatchConn {
    ws: WebSocket,
    term: vt100::Parser,
    received_data: bool,
}

impl WatchConn {
    fn new(ws: WebSocket) -> Self {
        Self {
            ws,
            term: vt100::Parser::default(),
            received_data: false,
        }
    }
}

impl Drop for WatchConn {
    fn drop(&mut self) {
        self.ws.close().unwrap();
    }
}

#[allow(clippy::large_enum_variant)]
enum State {
    Login,
    List(Vec<crate::protocol::Session>),
    Watch(WatchConn),
}

pub(crate) struct Model {
    config: crate::config::Config,
    state: State,
}

impl Model {
    pub(crate) fn new(config: crate::config::Config) -> Self {
        Self {
            config,
            state: State::Login,
        }
    }

    pub(crate) fn update(
        &mut self,
        msg: crate::Msg,
        orders: &mut impl Orders<crate::Msg>,
    ) {
        match msg {
            crate::Msg::Login(username) => {
                log::debug!("login");
                self.login(&username, orders);
            }
            crate::Msg::LoggedIn(..) => {
                log::debug!("logged in");
                orders.send_msg(crate::Msg::Refresh);
            }
            crate::Msg::Refresh => {
                log::debug!("refreshing");
                self.list(orders);
            }
            crate::Msg::List(sessions) => match sessions {
                Ok(sessions) => {
                    log::debug!("got sessions");
                    self.state = State::List(sessions);
                }
                Err(e) => {
                    log::error!("error getting sessions: {:?}", e);
                }
            },
            crate::Msg::StartWatching(id) => {
                log::debug!("watching {}", id);
                self.watch(&id, orders);
            }
            crate::Msg::Watch(id, event) => match event {
                crate::ws::WebSocketEvent::Connected(_) => {
                    log::info!("{}: connected", id);
                }
                crate::ws::WebSocketEvent::Disconnected(_) => {
                    log::info!("{}: disconnected", id);
                }
                crate::ws::WebSocketEvent::Message(msg) => {
                    log::info!("{}: message: {:?}", id, msg);
                    let json = msg.data().as_string().unwrap();
                    let msg: crate::protocol::Message =
                        serde_json::from_str(&json).unwrap();
                    match msg {
                        crate::protocol::Message::TerminalOutput { data } => {
                            self.process(&data);
                        }
                        crate::protocol::Message::Disconnected => {
                            self.list(orders);
                        }
                        crate::protocol::Message::Resize { size } => {
                            self.set_size(size.rows, size.cols);
                        }
                    }
                }
                crate::ws::WebSocketEvent::Error(e) => {
                    log::error!("{}: error: {:?}", id, e);
                }
            },
            crate::Msg::StopWatching => {
                self.list(orders);
            }
        }
    }

    pub(crate) fn logged_in(&self) -> bool {
        if let State::Login = self.state {
            false
        } else {
            true
        }
    }

    pub(crate) fn title(&self) -> &str {
        &self.config.title
    }

    pub(crate) fn screen(&self) -> Option<&vt100::Screen> {
        if let State::Watch(conn) = &self.state {
            Some(conn.term.screen())
        } else {
            None
        }
    }

    pub(crate) fn sessions(&self) -> &[crate::protocol::Session] {
        if let State::List(sessions) = &self.state {
            sessions
        } else {
            &[]
        }
    }

    pub(crate) fn watching(&self) -> bool {
        if let State::Watch(..) = self.state {
            true
        } else {
            false
        }
    }

    pub(crate) fn received_data(&self) -> bool {
        if let State::Watch(conn) = &self.state {
            conn.received_data
        } else {
            false
        }
    }

    fn login(&self, username: &str, orders: &mut impl Orders<crate::Msg>) {
        let url = format!(
            "http://{}/login?username={}",
            self.config.public_address, username
        );
        orders.perform_cmd(
            seed::Request::new(url).fetch_json_data(crate::Msg::LoggedIn),
        );
    }

    fn list(&self, orders: &mut impl Orders<crate::Msg>) {
        let url = format!("http://{}/list", self.config.public_address);
        orders.perform_cmd(
            seed::Request::new(url).fetch_json_data(crate::Msg::List),
        );
    }

    fn watch(&mut self, id: &str, orders: &mut impl Orders<crate::Msg>) {
        let url =
            format!("ws://{}/watch?id={}", self.config.public_address, id);
        let ws = crate::ws::connect(&url, id, crate::Msg::Watch, orders);
        self.state = State::Watch(WatchConn::new(ws));
    }

    fn process(&mut self, bytes: &[u8]) {
        if let State::Watch(conn) = &mut self.state {
            conn.term.process(bytes);
            conn.received_data = true;
        }
    }

    fn set_size(&mut self, rows: u16, cols: u16) {
        if let State::Watch(conn) = &mut self.state {
            conn.term.set_size(rows, cols);
        }
    }
}
