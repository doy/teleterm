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

impl Session {
    pub fn new(username: &str, term_type: &str) -> Self {
        Self {
            id: format!("{}", uuid::Uuid::new_v4()),
            username: username.to_string(),
            term_type: term_type.to_string(),
        }
    }
}
