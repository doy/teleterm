use gotham::state::FromState as _;
use std::convert::TryFrom as _;

#[derive(
    serde::Deserialize,
    gotham_derive::StateData,
    gotham_derive::StaticResponseExtender,
)]
pub struct PathParts {
    method: String,
}

#[derive(
    serde::Deserialize,
    gotham_derive::StateData,
    gotham_derive::StaticResponseExtender,
)]
pub struct QueryParams {
    code: String,
}

pub fn run(
    mut state: gotham::state::State,
) -> (gotham::state::State, hyper::Response<hyper::Body>) {
    let auth_type = {
        let path_parts = PathParts::borrow_from(&state);
        crate::protocol::AuthType::try_from(path_parts.method.as_str())
    };
    let auth_type = match auth_type {
        Ok(auth_type) => auth_type,
        Err(e) => {
            return (
                state,
                hyper::Response::builder()
                    .status(hyper::StatusCode::BAD_REQUEST)
                    .body(hyper::Body::from(format!("{}", e)))
                    .unwrap(),
            );
        }
    };
    let code = {
        let query_params = QueryParams::borrow_from(&state);
        query_params.code.clone()
    };

    // TODO

    (
        state,
        hyper::Response::builder()
            .status(hyper::StatusCode::FOUND)
            .header(hyper::header::LOCATION, "/")
            .body(hyper::Body::empty())
            .unwrap(),
    )
}
