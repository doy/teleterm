#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionType {
    Casting,
    Watching(String),
}
