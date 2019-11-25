use crate::prelude::*;

pub(crate) fn render(_: &crate::model::Model) -> Vec<Node<crate::Msg>> {
    vec![seed::form![
        seed::attrs! { At::Action => "#" },
        seed::label![seed::attrs! { At::For => "username" }, "username"],
        seed::input![seed::attrs! {
            At::Id => "username",
            At::Type => "text",
            At::AutoFocus => true.as_at_value(),
        }],
        seed::input![
            seed::attrs! { At::Type => "submit", At::Value => "login" }
        ],
        simple_ev(Ev::Submit, crate::Msg::Login),
    ]]
}
