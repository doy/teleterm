#![allow(clippy::type_complexity)]

mod client;
mod cmd;
mod component_future;
mod process;
mod protocol;
mod term;
mod util;

fn main() {
    match crate::cmd::parse().and_then(crate::cmd::run) {
        Ok(_) => {}
        Err(err) => {
            eprintln!("{}", err);
            std::process::exit(1);
        }
    }
}
