use crate::prelude::*;

pub(crate) fn render(model: &crate::model::Model) -> Vec<Node<crate::Msg>> {
    let mut view = vec![seed::h1![model.title()]];

    if model.watching() {
        view.extend(super::watch::render(model))
    } else {
        view.extend(super::list::render(model))
    }

    view
}
