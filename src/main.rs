// XXX this is broken with ale
// #![warn(clippy::cargo)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::multiple_crate_versions)]
#![allow(clippy::similar_names)]
#![allow(clippy::single_match)]
#![allow(clippy::single_match_else)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

const _DUMMY_DEPENDENCY: &str = include_str!("../Cargo.toml");

mod prelude;

mod async_stdin;
mod client;
mod cmd;
mod config;
mod dirs;
mod error;
mod key_reader;
mod oauth;
mod protocol;
mod resize;
mod server;
mod session_list;
mod term;
mod ttyrec;

fn main() {
    dirs::Dirs::new().create_all().unwrap();
    match crate::cmd::parse().and_then(|m| crate::cmd::run(&m)) {
        Ok(_) => {}
        Err(err) => {
            // we don't know if the log crate has been initialized yet
            eprintln!("{}", err);
            std::process::exit(1);
        }
    }
}
