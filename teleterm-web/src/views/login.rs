use crate::prelude::*;

pub(crate) fn render(_: &crate::model::Model) -> Vec<Node<crate::Msg>> {
    vec![seed::p!["logging in..."]]
}
