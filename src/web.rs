use gotham::router::builder::{DefineSingleRoute as _, DrawRoutes as _};

pub fn router() -> gotham::router::Router {
    gotham::router::builder::build_simple_router(|route| {
        route.get("/").to(root);
    })
}

fn root(state: gotham::state::State) -> (gotham::state::State, String) {
    (state, "hello world".to_string())
}
