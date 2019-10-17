#[derive(Debug, snafu::Snafu)]
#[snafu(visibility = "pub")]
pub enum Error {
    #[snafu(display("failed to accept: {}", source))]
    Acceptor { source: tokio::io::Error },

    #[snafu(display("auth type not allowed: {:?}", ty))]
    AuthTypeNotAllowed { ty: crate::protocol::AuthType },

    #[snafu(display("failed to bind: {}", source))]
    Bind { source: tokio::io::Error },

    #[snafu(display("failed to connect: {}", source))]
    Connect { source: std::io::Error },

    #[snafu(display("failed to connect: {}", source))]
    ConnectTls { source: native_tls::Error },

    #[snafu(display("couldn't find username"))]
    CouldntFindUsername,

    #[snafu(display("failed to create tls acceptor: {}", source))]
    CreateAcceptor { source: native_tls::Error },

    #[snafu(display("failed to create tls connector: {}", source))]
    CreateConnector { source: native_tls::Error },

    #[snafu(display("failed to create directory: {}", source))]
    CreateDir { source: std::io::Error },

    #[snafu(display("failed to create file: {}", source))]
    CreateFile { source: tokio::io::Error },

    #[snafu(display("eof"))]
    EOF,

    #[snafu(display("failed to retrieve access token: {:?}", msg))]
    ExchangeCode {
        msg: String,
        // XXX RequestTokenError doesn't implement the right traits
        // source: oauth2::RequestTokenError<
        //     oauth2::reqwest::Error,
        //     oauth2::StandardErrorResponse<
        //         oauth2::basic::BasicErrorResponseType,
        //     >,
        // >
    },

    #[snafu(display("failed to parse string: {:?}", data))]
    ExtraMessageData { data: Vec<u8> },

    #[snafu(display("failed to write to stdout: {}", source))]
    FlushTerminal { source: tokio::io::Error },

    #[snafu(display("failed to flush writes to terminal: {}", source))]
    FlushTerminalSync { source: std::io::Error },

    #[snafu(display(
        "failed to get recurse center profile data: {}",
        source
    ))]
    GetProfile { source: reqwest::Error },

    #[snafu(display("failed to get terminal size: {}", source))]
    GetTerminalSize { source: crossterm::ErrorKind },

    #[snafu(display("failed to find any resolved addresses"))]
    HasResolvedAddr,

    #[snafu(display("invalid auth type: {}", ty))]
    InvalidAuthType { ty: u8 },

    #[snafu(display("invalid auth type: {}", ty))]
    InvalidAuthTypeStr { ty: String },

    #[snafu(display("invalid message type: {}", ty))]
    InvalidMessageType { ty: u8 },

    #[snafu(display("invalid watch id: {}", id))]
    InvalidWatchId { id: String },

    #[snafu(display(
        "packet length must be at least {} bytes (got {})",
        expected,
        len
    ))]
    LenTooSmall { len: u32, expected: usize },

    #[snafu(display(
        "packet length must be at most {} bytes (got {})",
        expected,
        len
    ))]
    LenTooBig { len: u32, expected: usize },

    #[snafu(display("couldn't find name in argv"))]
    MissingArgv,

    #[snafu(display(
        "detected argv path was not a valid filename: {}",
        path
    ))]
    NotAFileName { path: String },

    #[snafu(display("failed to open file: {}", source))]
    OpenFile { source: tokio::io::Error },

    #[snafu(display("failed to open identity file: {}", source))]
    OpenIdentityFile { source: std::io::Error },

    #[snafu(display("failed to open link in browser: {}", source))]
    OpenLink { source: std::io::Error },

    #[snafu(display("failed to open a pty: {}", source))]
    OpenPty { source: std::io::Error },

    #[snafu(display("failed to parse address"))]
    ParseAddress,

    #[snafu(display("failed to parse address: {}", source))]
    ParseAddr { source: std::net::AddrParseError },

    #[snafu(display("{}", source))]
    ParseArgs { source: clap::Error },

    #[snafu(display("failed to parse buffer size '{}': {}", input, source))]
    ParseBufferSize {
        input: String,
        source: std::num::ParseIntError,
    },

    #[snafu(display("failed to parse incoming http request"))]
    ParseHttpRequest,

    #[snafu(display(
        "failed to validate csrf token on incoming http request"
    ))]
    ParseHttpRequestCsrf,

    #[snafu(display(
        "incoming http request had no code in the query parameters"
    ))]
    ParseHttpRequestMissingCode,

    #[snafu(display(
        "failed to parse path from incoming http request: {}",
        source
    ))]
    ParseHttpRequestPath { source: url::ParseError },

    #[snafu(display("failed to parse identity file: {}", source))]
    ParseIdentity { source: native_tls::Error },

    #[snafu(display("failed to parse int: {}", source))]
    ParseInt {
        source: std::array::TryFromSliceError,
    },

    #[snafu(display("failed to parse response json: {}", source))]
    ParseJson { source: reqwest::Error },

    #[snafu(display("failed to parse address: {}", source))]
    ParsePort { source: std::num::ParseIntError },

    #[snafu(display(
        "failed to parse read timeout '{}': {}",
        input,
        source
    ))]
    ParseReadTimeout {
        input: String,
        source: std::num::ParseIntError,
    },

    #[snafu(display("failed to parse string: {}", source))]
    ParseString { source: std::string::FromUtf8Error },

    #[snafu(display("failed to parse url: {}", source))]
    ParseUrl { source: url::ParseError },

    #[snafu(display("failed to poll for process exit: {}", source))]
    ProcessExitPoll { source: std::io::Error },

    #[snafu(display("rate limit exceeded"))]
    RateLimited,

    #[snafu(display("failed to read from event channel: {}", source))]
    ReadChannel {
        source: tokio::sync::mpsc::error::UnboundedRecvError,
    },

    #[snafu(display("failed to read from channel: {}", source))]
    ReadChannelBounded {
        source: tokio::sync::mpsc::error::RecvError,
    },

    #[snafu(display("failed to read from file: {}", source))]
    ReadFile { source: tokio::io::Error },

    #[snafu(display("failed to read identity file: {}", source))]
    ReadIdentityFile { source: std::io::Error },

    #[snafu(display("{}", source))]
    ReadMessageWithTimeout {
        #[snafu(source(from(tokio::timer::timeout::Error<Error>, Box::new)))]
        source: Box<tokio::timer::timeout::Error<Error>>,
    },

    #[snafu(display("failed to read packet: {}", source))]
    ReadPacket { source: std::io::Error },

    #[snafu(display("failed to read packet: {}", source))]
    ReadPacketAsync { source: tokio::io::Error },

    #[snafu(display("failed to read from pty: {}", source))]
    ReadPty { source: std::io::Error },

    #[snafu(display("failed to read from socket: {}", source))]
    ReadSocket { source: tokio::io::Error },

    #[snafu(display("failed to read from terminal: {}", source))]
    ReadTerminal { source: std::io::Error },

    #[snafu(display("failed to resize pty: {}", source))]
    ResizePty { source: std::io::Error },

    #[snafu(display("failed to resolve address: {}", source))]
    ResolveAddress { source: std::io::Error },

    #[snafu(display(
        "failed to send oauth result back to main thread: {}",
        source
    ))]
    SendResultChannel {
        source: tokio::sync::mpsc::error::SendError,
    },

    #[snafu(display(
        "failed to send accepted socket to server thread: {}",
        source
    ))]
    SendSocketChannel {
        source: tokio::sync::mpsc::error::TrySendError<tokio::net::TcpStream>,
    },

    #[snafu(display("failed to send accepted socket to server thread"))]
    SendSocketChannelTls {
        // XXX tokio_tls::Accept doesn't implement Debug or Display
    // source: tokio::sync::mpsc::error::TrySendError<tokio_tls::Accept<tokio::net::TcpStream>>,
    },

    #[snafu(display("received error from server: {}", message))]
    Server { message: String },

    #[snafu(display("SIGWINCH handler failed: {}", source))]
    SigWinchHandler { source: std::io::Error },

    #[snafu(display("failed to sleep until next frame: {}", source))]
    Sleep { source: tokio::timer::Error },

    #[snafu(display(
        "failed to receive new socket over channel: channel closed"
    ))]
    SocketChannelClosed,

    #[snafu(display(
        "failed to receive new socket over channel: {}",
        source
    ))]
    SocketChannelReceive {
        source: tokio::sync::mpsc::error::RecvError,
    },

    #[snafu(display("failed to spawn process for `{}`: {}", cmd, source))]
    SpawnProcess { cmd: String, source: std::io::Error },

    #[snafu(display(
        "failed to spawn a background thread to read terminal input: {}",
        source
    ))]
    TerminalInputReadingThread { source: std::io::Error },

    #[snafu(display(
        "terminal must be smaller than 1000 rows or columns (got {})",
        size
    ))]
    TermTooBig { size: crate::term::Size },

    #[snafu(display("timeout"))]
    Timeout,

    #[snafu(display("heartbeat timer failed: {}", source))]
    TimerHeartbeat { source: tokio::timer::Error },

    #[snafu(display("read timeout timer failed: {}", source))]
    TimerReadTimeout { source: tokio::timer::Error },

    #[snafu(display("reconnect timer failed: {}", source))]
    TimerReconnect { source: tokio::timer::Error },

    #[snafu(display("failed to switch to alternate screen: {}", source))]
    ToAlternateScreen { source: crossterm::ErrorKind },

    #[snafu(display(
        "failed to put the terminal into raw mode: {}",
        source
    ))]
    ToRawMode { source: crossterm::ErrorKind },

    #[snafu(display("unauthenticated message: {:?}", message))]
    UnauthenticatedMessage { message: crate::protocol::Message },

    #[snafu(display("unexpected message: {:?}", message))]
    UnexpectedMessage { message: crate::protocol::Message },

    #[snafu(display("failed to write to event channel: {}", source))]
    WriteChannel {
        source: tokio::sync::mpsc::error::UnboundedSendError,
    },

    #[snafu(display("failed to write to file: {}", source))]
    WriteFile { source: tokio::io::Error },

    #[snafu(display("{}", source))]
    WriteMessageWithTimeout {
        #[snafu(source(from(tokio::timer::timeout::Error<Error>, Box::new)))]
        source: Box<tokio::timer::timeout::Error<Error>>,
    },

    #[snafu(display("failed to write packet: {}", source))]
    WritePacket { source: std::io::Error },

    #[snafu(display("failed to write packet: {}", source))]
    WritePacketAsync { source: tokio::io::Error },

    #[snafu(display("failed to write to pty: {}", source))]
    WritePty { source: std::io::Error },

    #[snafu(display("failed to write to socket: {}", source))]
    WriteSocket { source: tokio::io::Error },

    #[snafu(display("failed to write to stdout: {}", source))]
    WriteTerminal { source: tokio::io::Error },

    #[snafu(display("failed to write to terminal: {}", source))]
    WriteTerminalCrossterm { source: crossterm::ErrorKind },

    #[snafu(display("failed to write to terminal: {}", source))]
    WriteTerminalSync { source: std::io::Error },
}

pub type Result<T> = std::result::Result<T, Error>;
