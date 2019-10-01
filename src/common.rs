#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionType {
    Casting,
    Watching(String),
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub username: String,
    pub term_type: String,
}
