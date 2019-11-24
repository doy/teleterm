use crate::prelude::*;

pub(crate) fn render(_: &crate::model::Model) -> Vec<Node<crate::Msg>> {
    vec![seed::form![
        seed::attrs! { At::Action => "#" },
        seed::label![seed::attrs! { At::For => "username" }, "username"],
        seed::input![
            seed::attrs! { At::Type => "text", At::Id => "username" }
        ],
        seed::input![
            seed::attrs! { At::Type => "submit", At::Value => "login" }
        ],
        input_ev(Ev::Submit, crate::Msg::Login),
    ]]
}
