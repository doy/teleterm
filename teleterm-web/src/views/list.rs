use crate::prelude::*;

pub(crate) fn render(model: &crate::model::Model) -> Vec<Node<crate::Msg>> {
    let mut list = vec![];
    for session in model.sessions() {
        list.push(seed::li![seed::button![
            simple_ev(
                Ev::Click,
                crate::Msg::StartWatching(session.id.clone())
            ),
            format!("{}: {}", session.username, session.id),
        ]]);
    }
    vec![
        seed::ul![list],
        seed::button![simple_ev(Ev::Click, crate::Msg::Refresh), "refresh"],
    ]
}
