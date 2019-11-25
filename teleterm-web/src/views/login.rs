use crate::prelude::*;

pub(crate) fn render(_: &crate::model::Model) -> Vec<Node<crate::Msg>> {
    vec![seed::form![
        seed::label![seed::attrs! { At::For => "username" }, "username"],
        seed::input![seed::attrs! {
            At::Id => "username",
            At::Type => "text",
            At::AutoFocus => true.as_at_value(),
        }],
        seed::input![
            seed::attrs! { At::Type => "submit", At::Value => "login" }
        ],
        raw_ev(Ev::Submit, |event| {
            event.prevent_default();
            let username = seed::to_input(
                &seed::document().get_element_by_id("username").unwrap(),
            )
            .value();
            crate::Msg::Login(username)
        }),
    ]]
}
