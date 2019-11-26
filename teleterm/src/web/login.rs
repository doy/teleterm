use gotham::state::FromState as _;

#[derive(
    serde::Deserialize,
    gotham_derive::StateData,
    gotham_derive::StaticResponseExtender,
)]
pub struct QueryParams {
    username: Option<String>,
}

#[derive(serde::Serialize)]
struct Response {
    username: Option<String>,
}

pub fn run(
    mut state: gotham::state::State,
) -> (gotham::state::State, hyper::Response<hyper::Body>) {
    let username = {
        let query_params = QueryParams::borrow_from(&state);
        query_params.username.clone()
    };
    let session = gotham::middleware::session::SessionData::<
        crate::web::SessionData,
    >::borrow_mut_from(&mut state);

    session.login = username.clone().map(|username| super::LoginState {
        username: username.clone(),
        auth: crate::protocol::Auth::plain(&username),
    });

    (
        state,
        hyper::Response::new(hyper::Body::from(
            serde_json::to_string(&Response { username }).unwrap(),
        )),
    )
}
