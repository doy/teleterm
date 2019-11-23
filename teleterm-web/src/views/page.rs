use crate::prelude::*;

pub(crate) fn render(model: &crate::model::Model) -> Vec<Node<crate::Msg>> {
    if model.watching() {
        super::watch::render(model)
    } else {
        super::list::render(model)
    }
}
