#[derive(Debug, snafu::Snafu)]
#[snafu(visibility = "pub")]
pub enum Error {
    #[snafu(display("failed to accept: {}", source))]
    Acceptor { source: tokio::io::Error },

    #[snafu(display(
        "oauth configuration for auth type {:?} not found",
        ty
    ))]
    AuthTypeMissingOauthConfig { ty: crate::protocol::AuthType },

    #[snafu(display("auth type {:?} not allowed", ty))]
    AuthTypeNotAllowed { ty: crate::protocol::AuthType },

    #[snafu(display("auth type {:?} does not use oauth", ty))]
    AuthTypeNotOauth { ty: crate::protocol::AuthType },

    #[snafu(display("failed to bind to {}: {}", address, source))]
    Bind {
        address: std::net::SocketAddr,
        source: tokio::io::Error,
    },

    #[snafu(display("config file {} doesn't exist", name))]
    ConfigFileDoesntExist { name: String },

    #[snafu(display("failed to connect to {}: {}", address, source))]
    Connect {
        address: std::net::SocketAddr,
        source: std::io::Error,
    },

    #[snafu(display(
        "failed to make tls connection to {}: {}",
        host,
        source
    ))]
    ConnectTls {
        host: String,
        source: native_tls::Error,
    },

    #[snafu(display("couldn't determine the current username"))]
    CouldntFindUsername,

    #[snafu(display("failed to parse configuration: {}", source))]
    CouldntParseConfig { source: config::ConfigError },

    #[snafu(display("failed to create tls acceptor: {}", source))]
    CreateAcceptor { source: native_tls::Error },

    #[snafu(display("failed to create tls connector: {}", source))]
    CreateConnector { source: native_tls::Error },

    #[snafu(display("failed to create directory {}: {}", filename, source))]
    CreateDir {
        filename: String,
        source: std::io::Error,
    },

    #[snafu(display("failed to create file {}: {}", filename, source))]
    CreateFile {
        filename: String,
        source: tokio::io::Error,
    },

    #[snafu(display("failed to create file {}: {}", filename, source))]
    CreateFileSync {
        filename: String,
        source: std::io::Error,
    },

    #[snafu(display("received EOF from server"))]
    EOF,

    #[snafu(display(
        "failed to retrieve access token from authorization code: {:?}",
        msg
    ))]
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

    #[snafu(display(
        "failed to retrieve access token from refresh token: {:?}",
        msg
    ))]
    ExchangeRefreshToken {
        msg: String,
        // XXX RequestTokenError doesn't implement the right traits
        // source: oauth2::RequestTokenError<
        //     oauth2::reqwest::Error,
        //     oauth2::StandardErrorResponse<
        //         oauth2::basic::BasicErrorResponseType,
        //     >,
        // >
    },

    #[snafu(display(
        "failed to parse string {:?}: unexpected trailing data",
        data
    ))]
    ExtraMessageData { data: Vec<u8> },

    #[snafu(display("failed to write to stdout: {}", source))]
    FlushTerminal { source: tokio::io::Error },

    #[snafu(display(
        "failed to get recurse center profile data: {}",
        source
    ))]
    GetRecurseCenterProfile { source: reqwest::Error },

    #[snafu(display("failed to get terminal size: {}", source))]
    GetTerminalSize { source: crossterm::ErrorKind },

    #[snafu(display("failed to find any resolvable addresses"))]
    HasResolvedAddr,

    #[snafu(display("invalid auth client {}", ty))]
    InvalidAuthClient { ty: u8 },

    #[snafu(display("invalid auth client {}", ty))]
    InvalidAuthClientStr { ty: String },

    #[snafu(display("invalid auth type {}", ty))]
    InvalidAuthType { ty: u8 },

    #[snafu(display("invalid auth type {}", ty))]
    InvalidAuthTypeStr { ty: String },

    #[snafu(display("invalid message type {}", ty))]
    InvalidMessageType { ty: u8 },

    #[snafu(display("invalid watch id {}", id))]
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
        "detected argv path {} was not a valid filename",
        path
    ))]
    NotAFileName { path: String },

    #[snafu(display(
        "missing oauth configuration item {} for section oauth.{}.{}",
        field,
        auth_type.name(),
        auth_client.name(),
    ))]
    OauthMissingConfiguration {
        field: String,
        auth_type: crate::protocol::AuthType,
        auth_client: crate::protocol::AuthClient,
    },

    #[snafu(display("failed to open file {}: {}", filename, source))]
    OpenFile {
        filename: String,
        source: tokio::io::Error,
    },

    #[snafu(display("failed to open file {}: {}", filename, source))]
    OpenFileSync {
        filename: String,
        source: std::io::Error,
    },

    #[snafu(display("failed to open link in browser: {}", source))]
    OpenLink { source: std::io::Error },

    #[snafu(display("failed to parse address"))]
    ParseAddress,

    #[snafu(display("failed to parse address: {}", source))]
    ParseAddr { source: std::net::AddrParseError },

    #[snafu(display("{}", source))]
    ParseArgs { source: clap::Error },

    #[snafu(display("failed to parse buffer size {}: {}", input, source))]
    ParseBufferSize {
        input: String,
        source: std::num::ParseIntError,
    },

    #[snafu(display("failed to parse config file: {}", source))]
    ParseConfigFile { source: config::ConfigError },

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

    #[snafu(display(
        "failed to parse int from buffer {:?}: {}",
        buf,
        source
    ))]
    ParseInt {
        buf: Vec<u8>,
        source: std::array::TryFromSliceError,
    },

    #[snafu(display("failed to parse float option {}: {}", name, source))]
    ParseFloat {
        name: String,
        source: std::num::ParseFloatError,
    },

    #[snafu(display("failed to parse response json: {}", source))]
    ParseJson { source: reqwest::Error },

    #[snafu(display("failed to parse max frame length: {}", source))]
    ParseMaxFrameLength { source: std::num::ParseIntError },

    #[snafu(display(
        "failed to parse port {} from address: {}",
        string,
        source
    ))]
    ParsePort {
        string: String,
        source: std::num::ParseIntError,
    },

    #[snafu(display("failed to parse read timeout {}: {}", input, source))]
    ParseReadTimeout {
        input: String,
        source: std::num::ParseIntError,
    },

    #[snafu(display("failed to parse string {:?}: {}", string, source))]
    ParseString {
        string: Vec<u8>,
        source: std::string::FromUtf8Error,
    },

    #[snafu(display("rate limit exceeded"))]
    RateLimited,

    #[snafu(display("failed to read from event channel: {}", source))]
    ReadChannel {
        source: tokio::sync::mpsc::error::UnboundedRecvError,
    },

    #[snafu(display("failed to read from file: {}", source))]
    ReadFile { source: tokio::io::Error },

    #[snafu(display("failed to read from file: {}", source))]
    ReadFileSync { source: std::io::Error },

    #[snafu(display("{}", source))]
    ReadMessageWithTimeout {
        #[snafu(source(from(tokio::timer::timeout::Error<Error>, Box::new)))]
        source: Box<tokio::timer::timeout::Error<Error>>,
    },

    #[snafu(display("failed to read packet: {}", source))]
    ReadPacket { source: tokio::io::Error },

    #[snafu(display("failed to read from socket: {}", source))]
    ReadSocket { source: tokio::io::Error },

    #[snafu(display("failed to read from terminal: {}", source))]
    ReadTerminal { source: std::io::Error },

    #[snafu(display("failed to read ttyrec: {}", source))]
    ReadTtyrec { source: ttyrec::Error },

    #[snafu(display("failed to poll for terminal resizing: {}", source))]
    Resize {
        source: tokio_terminal_resize::Error,
    },

    #[snafu(display(
        "failed to resolve address {}:{}: {}",
        host,
        port,
        source
    ))]
    ResolveAddress {
        host: String,
        port: u16,
        source: std::io::Error,
    },

    #[snafu(display("failed to serialize message as json: {}", source))]
    SerializeMessage { source: serde_json::Error },

    #[snafu(display("received error from server: {}", message))]
    Server { message: String },

    #[snafu(display("couldn't connect to server"))]
    ServerDisconnected,

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

    #[snafu(display("poll subprocess failed: {}", source))]
    Subprocess {
        source: tokio_pty_process_stream::Error,
    },

    #[snafu(display("failed to switch gid: {}", source))]
    SwitchGid { source: std::io::Error },

    #[snafu(display("failed to switch uid: {}", source))]
    SwitchUid { source: std::io::Error },

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

    #[snafu(display("failed to find group with gid {}", gid))]
    UnknownGid { gid: users::gid_t },

    #[snafu(display("failed to find group with group name {}", name))]
    UnknownGroup { name: String },

    #[snafu(display("failed to find user with uid {}", uid))]
    UnknownUid { uid: users::uid_t },

    #[snafu(display("failed to find user with username {}", name))]
    UnknownUser { name: String },

    #[snafu(display("failure during websocket stream: {}", source))]
    WebSocket {
        source: tokio_tungstenite::tungstenite::Error,
    },

    #[snafu(display("failed to accept websocket connection: {}", source))]
    WebSocketAccept { source: hyper::Error },

    #[snafu(display("failed to write to file: {}", source))]
    WriteFile { source: tokio::io::Error },

    #[snafu(display("failed to write to file: {}", source))]
    WriteFileSync { source: std::io::Error },

    #[snafu(display("{}", source))]
    WriteMessageWithTimeout {
        #[snafu(source(from(tokio::timer::timeout::Error<Error>, Box::new)))]
        source: Box<tokio::timer::timeout::Error<Error>>,
    },

    #[snafu(display("failed to write packet: {}", source))]
    WritePacket { source: tokio::io::Error },

    #[snafu(display("failed to write to socket: {}", source))]
    WriteSocket { source: tokio::io::Error },

    #[snafu(display("failed to write to stdout: {}", source))]
    WriteTerminal { source: tokio::io::Error },

    #[snafu(display("failed to write to terminal: {}", source))]
    WriteTerminalCrossterm { source: crossterm::ErrorKind },

    #[snafu(display("failed to write ttyrec: {}", source))]
    WriteTtyrec { source: ttyrec::Error },
}

pub type Result<T> = std::result::Result<T, Error>;
