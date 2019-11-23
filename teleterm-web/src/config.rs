use crate::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen]
    static TELETERM_CONFIG: JsValue;
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct Config {
    pub(crate) title: String,
    pub(crate) public_address: String,
}

impl Config {
    pub(crate) fn load() -> Self {
        TELETERM_CONFIG.into_serde().unwrap()
    }
}
