use crate::prelude::*;

const LIST_URL: &str = "http://127.0.0.1:4145/list";
const WATCH_URL: &str = "ws://127.0.0.1:4145/watch";

struct WatchConn {
    ws: WebSocket,
    term: vt100::Parser,
}

impl Drop for WatchConn {
    fn drop(&mut self) {
        self.ws.close().unwrap();
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Session {
    pub id: String,
    pub username: String,
}

#[derive(Default)]
pub struct Model {
    sessions: Vec<Session>,
    watch_conn: Option<WatchConn>,
}

impl Model {
    pub(crate) fn list(
        &self,
    ) -> impl futures::Future<Item = crate::Msg, Error = crate::Msg> {
        seed::Request::new(LIST_URL).fetch_json_data(crate::Msg::List)
    }

    pub(crate) fn watch(
        &mut self,
        id: &str,
        orders: &mut impl Orders<crate::Msg>,
    ) {
        let ws = crate::ws::connect(
            &format!("{}?id={}", WATCH_URL, id),
            id,
            crate::Msg::Watch,
            orders,
        );
        let term = vt100::Parser::default();
        self.watch_conn = Some(WatchConn { ws, term })
    }

    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    pub fn update_sessions(&mut self, sessions: Vec<Session>) {
        self.sessions = sessions;
    }

    pub fn watching(&self) -> bool {
        self.watch_conn.is_some()
    }

    pub fn disconnect_watch(&mut self) {
        self.watch_conn = None;
    }

    pub fn process(&mut self, bytes: &[u8]) {
        if let Some(conn) = &mut self.watch_conn {
            conn.term.process(bytes);
        }
    }

    pub fn screen(&self) -> String {
        if let Some(conn) = &self.watch_conn {
            conn.term.screen().contents()
        } else {
            "".to_string()
        }
    }
}
