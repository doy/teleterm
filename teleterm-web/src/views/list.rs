use crate::prelude::*;

pub(crate) fn render(model: &crate::model::Model) -> Vec<Node<crate::Msg>> {
    vec![
        crate::views::sessions::render(model.sessions()),
        seed::button![simple_ev(Ev::Click, crate::Msg::Refresh), "refresh"],
    ]
}
