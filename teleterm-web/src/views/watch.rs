use crate::prelude::*;

pub(crate) fn render(model: &crate::model::Model) -> Vec<Node<crate::Msg>> {
    vec![
        if let Some(screen) = model.screen() {
            if model.received_data() {
                crate::views::terminal::render(screen)
            } else {
                seed::empty![]
            }
        } else {
            seed::empty![]
        },
        seed::button![simple_ev(Ev::Click, crate::Msg::StopWatching), "back"],
    ]
}
