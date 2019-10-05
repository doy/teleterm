#[derive(Debug, snafu::Snafu)]
#[snafu(visibility = "pub")]
pub enum Error {
    #[snafu(display("failed to connect: {}", source))]
    Connect { source: std::io::Error },

    #[snafu(display("couldn't find username"))]
    CouldntFindUsername,

    #[snafu(display("failed to get terminal size: {}", source))]
    GetTerminalSize { source: crossterm::ErrorKind },

    #[snafu(display(
        "failed to put the terminal into raw mode: {}",
        source
    ))]
    IntoRawMode { source: crossterm::ErrorKind },

    #[snafu(display("failed to parse address: {}", source))]
    ParseAddr { source: std::net::AddrParseError },

    #[snafu(display("failed to parse buffer size '{}': {}", input, source))]
    ParseBufferSize {
        input: String,
        source: std::num::ParseIntError,
    },

    #[snafu(display("unexpected message: {:?}", message))]
    UnexpectedMessage { message: crate::protocol::Message },
}
