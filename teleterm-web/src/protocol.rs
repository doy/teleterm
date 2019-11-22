// it's possible we should just consider pulling the real protocol out into a
// crate or something? but ideally in a way that doesn't require pulling in
// tokio
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub enum Message {
    TerminalOutput { data: Vec<u8> },
    Disconnected,
}
