use gotham::state::FromState as _;

pub fn run(
    mut state: gotham::state::State,
) -> (gotham::state::State, hyper::Response<hyper::Body>) {
    let session = gotham::middleware::session::SessionData::<
        crate::web::SessionData,
    >::take_from(&mut state);

    session.discard(&mut state).unwrap();

    (state, hyper::Response::new(hyper::Body::empty()))
}
