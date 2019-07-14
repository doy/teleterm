use snafu::OptionExt as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("couldn't find name in argv"))]
    MissingArgv,

    #[snafu(display(
        "detected argv path was not a valid filename: {}",
        path
    ))]
    NotAFileName { path: String },
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn program_name() -> Result<String> {
    let program = std::env::args().next().context(MissingArgv)?;
    let path = std::path::Path::new(&program);
    let filename = path.file_name();
    Ok(filename
        .ok_or_else(|| Error::NotAFileName {
            path: path.to_string_lossy().to_string(),
        })?
        .to_string_lossy()
        .to_string())
}
