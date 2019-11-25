use crate::prelude::*;

pub(crate) fn render(model: &crate::model::Model) -> Vec<Node<crate::Msg>> {
    let mut view = vec![seed::h1![model.title()]];

    if let Some(username) = model.username() {
        view.push(seed::p!["logged in as ", username]);
    } else {
        view.push(seed::p!["not logged in"]);
    }

    if model.logging_in() {
        if model.username().is_some() {
            view.push(seed::p!["loading..."]);
        } else {
            view.extend(super::login::render(model))
        }
    } else if model.choosing() {
        view.extend(super::list::render(model))
    } else if model.watching() {
        view.extend(super::watch::render(model))
    } else {
        unreachable!()
    }

    view
}
