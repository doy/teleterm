mod client;
mod cmd;
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
