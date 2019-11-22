use crate::prelude::*;

pub(crate) fn render(model: &crate::model::Model) -> Vec<Node<crate::Msg>> {
    vec![
        crate::views::terminal::render(model),
        seed::button![simple_ev(Ev::Click, crate::Msg::StopWatching), "back"],
    ]
}
