#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::similar_names)]
#![allow(clippy::single_match)]
#![allow(clippy::single_match_else)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

mod prelude;

#[macro_use]
mod component_future;

mod async_stdin;
mod client;
mod cmd;
mod config;
mod dirs;
mod error;
mod key_reader;
mod oauth;
mod process;
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
            eprintln!("{}", err);
            std::process::exit(1);
        }
    }
}
