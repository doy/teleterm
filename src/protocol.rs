use futures::future::Future as _;
use futures::sink::Sink as _;
use futures::stream::Stream as _;
use snafu::futures01::FutureExt as _;
use snafu::ResultExt as _;
use std::convert::{TryFrom as _, TryInto as _};

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to read packet: {}", source))]
    Read { source: std::io::Error },

    #[snafu(display("failed to read packet: {}", source))]
    ReadAsync { source: tokio::io::Error },

    #[snafu(display("failed to write packet: {}", source))]
    Write { source: std::io::Error },

    #[snafu(display("failed to write packet: {}", source))]
    WriteAsync { source: tokio::io::Error },

    #[snafu(display("failed to parse string: {}", source))]
    ParseString { source: std::string::FromUtf8Error },

    #[snafu(display("failed to parse int: {}", source))]
    ParseInt {
        source: std::array::TryFromSliceError,
    },

    #[snafu(display("failed to parse string: {:?}", data))]
    ExtraMessageData { data: Vec<u8> },

    #[snafu(display("invalid message type: {}", ty))]
    InvalidMessageType { ty: u32 },

    #[snafu(display("eof"))]
    EOF,
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub username: String,
    pub term_type: String,
    pub size: (u32, u32),
    pub idle_time: u32,
    pub title: String,
}

pub struct FramedReader(
    tokio::codec::FramedRead<
        tokio::io::ReadHalf<tokio::net::tcp::TcpStream>,
        tokio::codec::length_delimited::LengthDelimitedCodec,
    >,
);

impl FramedReader {
    pub fn new(rs: tokio::io::ReadHalf<tokio::net::tcp::TcpStream>) -> Self {
        Self(
            tokio::codec::length_delimited::Builder::new()
                .length_field_length(4)
                .new_read(rs),
        )
    }
}

pub struct FramedWriter(
    tokio::codec::FramedWrite<
        tokio::io::WriteHalf<tokio::net::tcp::TcpStream>,
        tokio::codec::length_delimited::LengthDelimitedCodec,
    >,
);

impl FramedWriter {
    pub fn new(ws: tokio::io::WriteHalf<tokio::net::tcp::TcpStream>) -> Self {
        Self(
            tokio::codec::length_delimited::Builder::new()
                .length_field_length(4)
                .new_write(ws),
        )
    }
}

pub const PROTO_VERSION: u32 = 1;

// XXX https://github.com/rust-lang/rust/issues/64362
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Login {
        proto_version: u32,
        username: String,
        term_type: String,
        size: (u32, u32),
    },
    StartCasting,
    StartWatching {
        id: String,
    },
    Heartbeat,
    TerminalOutput {
        data: Vec<u8>,
    },
    ListSessions,
    Sessions {
        sessions: Vec<Session>,
    },
    Disconnected,
    Error {
        msg: String,
    },
    Resize {
        size: (u32, u32),
    },
}

const MSG_LOGIN: u32 = 0;
const MSG_START_CASTING: u32 = 1;
const MSG_START_WATCHING: u32 = 2;
const MSG_HEARTBEAT: u32 = 3;
const MSG_TERMINAL_OUTPUT: u32 = 4;
const MSG_LIST_SESSIONS: u32 = 5;
const MSG_SESSIONS: u32 = 6;
const MSG_DISCONNECTED: u32 = 7;
const MSG_ERROR: u32 = 8;
const MSG_RESIZE: u32 = 9;

impl Message {
    pub fn login(username: &str, term_type: &str, size: (u32, u32)) -> Self {
        Self::Login {
            proto_version: PROTO_VERSION,
            username: username.to_string(),
            term_type: term_type.to_string(),
            size,
        }
    }

    pub fn start_casting() -> Self {
        Self::StartCasting
    }

    pub fn start_watching(id: &str) -> Self {
        Self::StartWatching { id: id.to_string() }
    }

    pub fn heartbeat() -> Self {
        Self::Heartbeat
    }

    pub fn terminal_output(data: &[u8]) -> Self {
        Self::TerminalOutput {
            data: data.to_vec(),
        }
    }

    pub fn list_sessions() -> Self {
        Self::ListSessions
    }

    pub fn sessions(sessions: &[Session]) -> Self {
        Self::Sessions {
            sessions: sessions.to_vec(),
        }
    }

    pub fn disconnected() -> Self {
        Self::Disconnected
    }

    pub fn error(msg: &str) -> Self {
        Self::Error {
            msg: msg.to_string(),
        }
    }

    pub fn resize(size: (u32, u32)) -> Self {
        Self::Resize { size }
    }

    #[allow(dead_code)]
    pub fn read<R: std::io::Read>(r: R) -> Result<Self> {
        Packet::read(r).and_then(Self::try_from)
    }

    pub fn read_async(
        r: FramedReader,
    ) -> impl futures::future::Future<Item = (Self, FramedReader), Error = Error>
    {
        Packet::read_async(r).and_then(|(packet, r)| {
            Self::try_from(packet).map(|msg| (msg, r))
        })
    }

    #[allow(dead_code)]
    pub fn write<W: std::io::Write>(&self, w: W) -> Result<()> {
        Packet::from(self).write(w)
    }

    pub fn write_async(
        &self,
        w: FramedWriter,
    ) -> impl futures::future::Future<Item = FramedWriter, Error = Error>
    {
        Packet::from(self).write_async(w)
    }
}

struct Packet {
    ty: u32,
    data: Vec<u8>,
}

impl Packet {
    fn read<R: std::io::Read>(mut r: R) -> Result<Self> {
        let mut len_buf = [0_u8; std::mem::size_of::<u32>()];
        r.read_exact(&mut len_buf).context(Read)?;
        let len = u32::from_be_bytes(len_buf.try_into().unwrap());

        let mut data = vec![0_u8; len.try_into().unwrap()];
        r.read_exact(&mut data).context(Read)?;
        let (ty_buf, rest) = data.split_at(std::mem::size_of::<u32>());
        let ty = u32::from_be_bytes(ty_buf.try_into().unwrap());

        Ok(Self {
            ty,
            data: rest.to_vec(),
        })
    }

    fn read_async(
        r: FramedReader,
    ) -> impl futures::future::Future<Item = (Self, FramedReader), Error = Error>
    {
        r.0.into_future()
            .map_err(|(e, _)| Error::ReadAsync { source: e })
            .and_then(|(data, r)| match data {
                Some(data) => Ok((data, r)),
                None => Err(Error::EOF),
            })
            .map(|(buf, r)| {
                let (ty_buf, data_buf) =
                    buf.split_at(std::mem::size_of::<u32>());
                let ty = u32::from_be_bytes(ty_buf.try_into().unwrap());
                let data = data_buf.to_vec();
                (Self { ty, data }, FramedReader(r))
            })
    }

    fn write<W: std::io::Write>(&self, mut w: W) -> Result<()> {
        let bytes = self.as_bytes();
        let len: u32 = bytes.len().try_into().unwrap();
        let len_buf = len.to_be_bytes();
        let buf: Vec<u8> =
            len_buf.iter().chain(bytes.iter()).copied().collect();
        Ok(w.write_all(&buf).context(Write)?)
    }

    fn write_async(
        &self,
        w: FramedWriter,
    ) -> impl futures::future::Future<Item = FramedWriter, Error = Error>
    {
        w.0.send(bytes::Bytes::from(self.as_bytes()))
            .map(FramedWriter)
            .context(WriteAsync)
    }

    fn as_bytes(&self) -> Vec<u8> {
        self.ty
            .to_be_bytes()
            .iter()
            .chain(self.data.iter())
            .cloned()
            .collect()
    }
}

impl From<&Message> for Packet {
    fn from(msg: &Message) -> Self {
        fn u32_from_usize(n: usize) -> u32 {
            // XXX this can actually panic
            n.try_into().unwrap()
        }
        fn write_u32(val: u32, data: &mut Vec<u8>) {
            data.extend_from_slice(&val.to_be_bytes());
        }
        fn write_bytes(val: &[u8], data: &mut Vec<u8>) {
            write_u32(u32_from_usize(val.len()), data);
            data.extend_from_slice(val);
        }
        fn write_str(val: &str, data: &mut Vec<u8>) {
            write_bytes(val.as_bytes(), data);
        }
        fn write_session(val: &Session, data: &mut Vec<u8>) {
            write_str(&val.id, data);
            write_str(&val.username, data);
            write_str(&val.term_type, data);
            write_u32(val.size.0, data);
            write_u32(val.size.1, data);
            write_u32(val.idle_time, data);
            write_str(&val.title, data);
        }
        fn write_sessions(val: &[Session], data: &mut Vec<u8>) {
            write_u32(u32_from_usize(val.len()), data);
            for s in val {
                write_session(s, data);
            }
        }

        match msg {
            Message::Login {
                proto_version,
                username,
                term_type,
                size,
            } => {
                let mut data = vec![];

                write_u32(*proto_version, &mut data);
                write_str(username, &mut data);
                write_str(term_type, &mut data);
                write_u32(size.0, &mut data);
                write_u32(size.1, &mut data);

                Self {
                    ty: MSG_LOGIN,
                    data,
                }
            }
            Message::StartCasting => Self {
                ty: MSG_START_CASTING,
                data: vec![],
            },
            Message::StartWatching { id } => {
                let mut data = vec![];

                write_str(id, &mut data);

                Self {
                    ty: MSG_START_WATCHING,
                    data,
                }
            }
            Message::Heartbeat => Self {
                ty: MSG_HEARTBEAT,
                data: vec![],
            },
            Message::TerminalOutput { data: output } => {
                let mut data = vec![];

                write_bytes(output, &mut data);

                Self {
                    ty: MSG_TERMINAL_OUTPUT,
                    data: data.to_vec(),
                }
            }
            Message::ListSessions => Self {
                ty: MSG_LIST_SESSIONS,
                data: vec![],
            },
            Message::Sessions { sessions } => {
                let mut data = vec![];

                write_sessions(sessions, &mut data);

                Self {
                    ty: MSG_SESSIONS,
                    data,
                }
            }
            Message::Disconnected => Self {
                ty: MSG_DISCONNECTED,
                data: vec![],
            },
            Message::Error { msg } => {
                let mut data = vec![];

                write_str(msg, &mut data);

                Self {
                    ty: MSG_ERROR,
                    data,
                }
            }
            Message::Resize { size } => {
                let mut data = vec![];

                write_u32(size.0, &mut data);
                write_u32(size.1, &mut data);

                Self {
                    ty: MSG_RESIZE,
                    data,
                }
            }
        }
    }
}

impl std::convert::TryFrom<Packet> for Message {
    type Error = Error;

    fn try_from(packet: Packet) -> Result<Self> {
        fn read_u32(data: &[u8]) -> Result<(u32, &[u8])> {
            let (buf, rest) = data.split_at(std::mem::size_of::<u32>());
            let val = u32::from_be_bytes(buf.try_into().context(ParseInt)?);
            Ok((val, rest))
        }
        fn read_bytes(data: &[u8]) -> Result<(Vec<u8>, &[u8])> {
            let (len, data) = read_u32(data)?;
            let (buf, rest) = data.split_at(len.try_into().unwrap());
            let val = buf.to_vec();
            Ok((val, rest))
        }
        fn read_str(data: &[u8]) -> Result<(String, &[u8])> {
            let (bytes, rest) = read_bytes(data)?;
            let val = String::from_utf8(bytes).context(ParseString)?;
            Ok((val, rest))
        }
        fn read_session(data: &[u8]) -> Result<(Session, &[u8])> {
            let (id, data) = read_str(data)?;
            let (username, data) = read_str(data)?;
            let (term_type, data) = read_str(data)?;
            let (cols, data) = read_u32(data)?;
            let (rows, data) = read_u32(data)?;
            let (idle_time, data) = read_u32(data)?;
            let (title, data) = read_str(data)?;
            Ok((
                Session {
                    id,
                    username,
                    term_type,
                    size: (cols, rows),
                    idle_time,
                    title,
                },
                data,
            ))
        }
        fn read_sessions(data: &[u8]) -> Result<(Vec<Session>, &[u8])> {
            let mut val = vec![];
            let (len, mut data) = read_u32(data)?;
            for _ in 0..len {
                let (subval, subdata) = read_session(data)?;
                val.push(subval);
                data = subdata;
            }
            Ok((val, data))
        }

        let data: &[u8] = packet.data.as_ref();
        let (msg, rest) = match packet.ty {
            MSG_LOGIN => {
                let (proto_version, data) = read_u32(data)?;
                let (username, data) = read_str(data)?;
                let (term_type, data) = read_str(data)?;
                let (cols, data) = read_u32(data)?;
                let (rows, data) = read_u32(data)?;

                (
                    Self::Login {
                        proto_version,
                        username,
                        term_type,
                        size: (cols, rows),
                    },
                    data,
                )
            }
            MSG_START_CASTING => (Self::StartCasting, data),
            MSG_START_WATCHING => {
                let (id, data) = read_str(data)?;

                (Self::StartWatching { id }, data)
            }
            MSG_HEARTBEAT => (Self::Heartbeat, data),
            MSG_TERMINAL_OUTPUT => {
                let (output, data) = read_bytes(data)?;

                (Self::TerminalOutput { data: output }, data)
            }
            MSG_LIST_SESSIONS => (Self::ListSessions, data),
            MSG_SESSIONS => {
                let (sessions, data) = read_sessions(data)?;

                (Self::Sessions { sessions }, data)
            }
            MSG_DISCONNECTED => (Self::Disconnected, data),
            MSG_ERROR => {
                let (msg, data) = read_str(data)?;
                (Self::Error { msg }, data)
            }
            MSG_RESIZE => {
                let (cols, data) = read_u32(data)?;
                let (rows, data) = read_u32(data)?;

                (Self::Resize { size: (cols, rows) }, data)
            }
            _ => return Err(Error::InvalidMessageType { ty: packet.ty }),
        };

        if !rest.is_empty() {
            return Err(Error::ExtraMessageData {
                data: rest.to_vec(),
            });
        }

        Ok(msg)
    }
}

#[cfg(test)]
#[allow(clippy::cognitive_complexity)]
#[allow(clippy::shadow_unrelated)]
mod test {
    use super::*;

    #[test]
    fn test_serde() {
        let msg = Message::login("doy", "screen", (80, 24));
        let packet = Packet::from(&msg);
        let msg2 = Message::try_from(packet).unwrap();
        assert_eq!(msg, msg2);

        let msg = Message::start_casting();
        let packet = Packet::from(&msg);
        let msg2 = Message::try_from(packet).unwrap();
        assert_eq!(msg, msg2);

        let msg = Message::start_watching("some-session-id");
        let packet = Packet::from(&msg);
        let msg2 = Message::try_from(packet).unwrap();
        assert_eq!(msg, msg2);

        let msg = Message::heartbeat();
        let packet = Packet::from(&msg);
        let msg2 = Message::try_from(packet).unwrap();
        assert_eq!(msg, msg2);

        let msg = Message::terminal_output(b"foobar");
        let packet = Packet::from(&msg);
        let msg2 = Message::try_from(packet).unwrap();
        assert_eq!(msg, msg2);

        let msg = Message::list_sessions();
        let packet = Packet::from(&msg);
        let msg2 = Message::try_from(packet).unwrap();
        assert_eq!(msg, msg2);

        let msg = Message::sessions(&[]);
        let packet = Packet::from(&msg);
        let msg2 = Message::try_from(packet).unwrap();
        assert_eq!(msg, msg2);

        let msg = Message::disconnected();
        let packet = Packet::from(&msg);
        let msg2 = Message::try_from(packet).unwrap();
        assert_eq!(msg, msg2);

        let msg = Message::error("error message");
        let packet = Packet::from(&msg);
        let msg2 = Message::try_from(packet).unwrap();
        assert_eq!(msg, msg2);

        let msg = Message::resize((81, 25));
        let packet = Packet::from(&msg);
        let msg2 = Message::try_from(packet).unwrap();
        assert_eq!(msg, msg2);
    }
}
