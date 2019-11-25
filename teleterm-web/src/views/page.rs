use crate::prelude::*;

pub(crate) fn render(model: &crate::model::Model) -> Vec<Node<crate::Msg>> {
    let mut view = vec![seed::h1![model.title()]];

    if model.logging_in() {
        view.extend(super::login::render(model))
    } else if model.choosing() {
        view.extend(super::list::render(model))
    } else if model.watching() {
        view.extend(super::watch::render(model))
    } else {
        unreachable!()
    }

    view
}
