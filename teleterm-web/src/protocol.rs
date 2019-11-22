// it's possible we should just consider pulling the real protocol out into a
// crate or something? but ideally in a way that doesn't require pulling in
// tokio
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub enum Message {
    TerminalOutput { data: Vec<u8> },
    Disconnected,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Session {
    pub id: String,
    pub username: String,
    pub term_type: String,
    pub size: Size,
    pub idle_time: u32,
    pub title: String,
    pub watchers: u32,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}
