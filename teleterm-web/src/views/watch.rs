use crate::prelude::*;

pub(crate) fn render(model: &crate::model::Model) -> Node<crate::Msg> {
    seed::pre![model.screen()]
}
