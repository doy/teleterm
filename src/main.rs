mod cmd;
mod pb;
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
