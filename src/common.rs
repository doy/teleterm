#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub username: String,
    pub term_type: String,
    pub size: (u32, u32),
}
