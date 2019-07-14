#[derive(Debug, snafu::Snafu)]
pub enum Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Stream your terminal")
}

pub fn run<'a>(matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    unimplemented!()
}
