use handlebars::handlebars_helper;
use lazy_static::lazy_static;
use lazy_static_include::*;

lazy_static_include::lazy_static_include_bytes!(
    pub INDEX_HTML_TMPL,
    "static/index.html.tmpl"
);
lazy_static_include::lazy_static_include_bytes!(
    pub TELETERM_WEB_JS,
    "static/teleterm_web.js"
);
lazy_static_include::lazy_static_include_bytes!(
    pub TELETERM_WEB_WASM,
    "static/teleterm_web_bg.wasm"
);
lazy_static_include::lazy_static_include_bytes!(
    pub TELETERM_CSS,
    "static/teleterm.css"
);

handlebars_helper!(json: |x: object| serde_json::to_string(x).unwrap());

pub const INDEX_HTML_TMPL_NAME: &str = "index";
lazy_static::lazy_static! {
    pub static ref HANDLEBARS: handlebars::Handlebars = {
        let mut handlebars = handlebars::Handlebars::new();
        handlebars.register_helper("json", Box::new(json));
        handlebars
            .register_template_string(
                INDEX_HTML_TMPL_NAME,
                String::from_utf8(INDEX_HTML_TMPL.to_vec()).unwrap(),
            )
            .unwrap();
        handlebars
    };
}
