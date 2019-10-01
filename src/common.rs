#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionType {
    Casting,
    Watching(String),
}

#[derive(Debug, Clone)]
pub struct SessionMetadata {
    pub username: String,
    pub term_type: String,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub metadata: Option<SessionMetadata>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            id: format!("{}", uuid::Uuid::new_v4()),
            metadata: None,
        }
    }

    pub fn connect(&mut self, username: &str, term_type: &str) {
        self.metadata = Some(SessionMetadata {
            username: username.to_string(),
            term_type: term_type.to_string(),
        })
    }
}
