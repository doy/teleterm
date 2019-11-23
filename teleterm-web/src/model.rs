use crate::prelude::*;

const LIST_URL: &str = "http://127.0.0.1:4145/list";
const WATCH_URL: &str = "ws://127.0.0.1:4145/watch";

struct WatchConn {
    ws: WebSocket,
    term: vt100::Parser,
    received_data: bool,
}

impl Drop for WatchConn {
    fn drop(&mut self) {
        self.ws.close().unwrap();
    }
}

#[derive(Default)]
pub(crate) struct Model {
    sessions: Vec<crate::protocol::Session>,
    watch_conn: Option<WatchConn>,
}

impl Model {
    pub(crate) fn update(
        &mut self,
        msg: crate::Msg,
        orders: &mut impl Orders<crate::Msg>,
    ) {
        match msg {
            crate::Msg::List(sessions) => match sessions {
                Ok(sessions) => {
                    log::debug!("got sessions");
                    self.update_sessions(sessions);
                }
                Err(e) => {
                    log::error!("error getting sessions: {:?}", e);
                }
            },
            crate::Msg::Refresh => {
                log::debug!("refreshing");
                orders.perform_cmd(
                    seed::Request::new(LIST_URL)
                        .fetch_json_data(crate::Msg::List),
                );
            }
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
                            self.disconnect_watch();
                            orders.send_msg(crate::Msg::Refresh);
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
                self.disconnect_watch();
                orders.send_msg(crate::Msg::Refresh);
            }
        }
    }

    pub(crate) fn screen(&self) -> Option<&vt100::Screen> {
        self.watch_conn.as_ref().map(|conn| conn.term.screen())
    }

    pub(crate) fn sessions(&self) -> &[crate::protocol::Session] {
        &self.sessions
    }

    pub(crate) fn watching(&self) -> bool {
        self.watch_conn.is_some()
    }

    pub(crate) fn received_data(&self) -> bool {
        self.watch_conn
            .as_ref()
            .map(|conn| conn.received_data)
            .unwrap_or(false)
    }

    fn watch(&mut self, id: &str, orders: &mut impl Orders<crate::Msg>) {
        let ws = crate::ws::connect(
            &format!("{}?id={}", WATCH_URL, id),
            id,
            crate::Msg::Watch,
            orders,
        );
        let term = vt100::Parser::default();
        self.watch_conn = Some(WatchConn {
            ws,
            term,
            received_data: false,
        })
    }

    fn update_sessions(&mut self, sessions: Vec<crate::protocol::Session>) {
        self.sessions = sessions;
    }

    fn disconnect_watch(&mut self) {
        self.watch_conn = None;
    }

    fn process(&mut self, bytes: &[u8]) {
        if let Some(conn) = &mut self.watch_conn {
            conn.term.process(bytes);
            conn.received_data = true;
        }
    }

    fn set_size(&mut self, rows: u16, cols: u16) {
        if let Some(conn) = &mut self.watch_conn {
            conn.term.set_size(rows, cols);
        }
    }
}
