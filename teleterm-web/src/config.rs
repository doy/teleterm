use crate::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen]
    static TELETERM_CONFIG: JsValue;
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct Config {
    pub(crate) username: Option<String>,
    pub(crate) public_address: String,
    pub(crate) allowed_login_methods:
        std::collections::HashSet<crate::protocol::AuthType>,
    pub(crate) oauth_login_urls:
        std::collections::HashMap<crate::protocol::AuthType, String>,
}

impl Config {
    pub(crate) fn load() -> Self {
        TELETERM_CONFIG.into_serde().unwrap()
    }
}
