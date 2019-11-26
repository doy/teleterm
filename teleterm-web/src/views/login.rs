use crate::prelude::*;

pub(crate) fn render(model: &crate::model::Model) -> Vec<Node<crate::Msg>> {
    let plain = model.allowed_login_method(crate::protocol::AuthType::Plain);
    let recurse_center_url = if model
        .allowed_login_method(crate::protocol::AuthType::RecurseCenter)
    {
        model.oauth_login_url(crate::protocol::AuthType::RecurseCenter)
    } else {
        None
    };

    let mut view = vec![];

    if plain {
        view.extend(render_plain());
    }
    if plain && recurse_center_url.is_some() {
        view.push(seed::p!["or"])
    }
    if let Some(url) = recurse_center_url {
        view.extend(render_recurse_center(&url));
    }

    view
}

fn render_plain() -> Vec<Node<crate::Msg>> {
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

fn render_recurse_center(url: &str) -> Vec<Node<crate::Msg>> {
    vec![seed::a![
        seed::attrs! {
            At::Href => url,
        },
        "login via oauth"
    ]]
}
