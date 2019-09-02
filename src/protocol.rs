use futures::future::Future as _;
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

    #[snafu(display("invalid StartCasting message: {}", source))]
    ParseStartCastingMessage { source: std::string::FromUtf8Error },

    #[snafu(display("invalid StartWatching message: {}", source))]
    ParseStartWatchingMessage { source: std::string::FromUtf8Error },

    #[snafu(display("invalid Sessions message: {}", source))]
    ParseSessionsMessageLen {
        source: std::array::TryFromSliceError,
    },

    #[snafu(display("invalid Sessions message: {}", source))]
    ParseSessionsMessageId { source: std::string::FromUtf8Error },

    #[snafu(display("invalid WatchSession message: {}", source))]
    ParseWatchSessionMessage { source: std::string::FromUtf8Error },

    #[snafu(display("invalid message type: {}", ty))]
    InvalidMessageType { ty: u32 },
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Message {
    StartCasting { username: String },
    StartWatching { username: String },
    Heartbeat,
    TerminalOutput { data: Vec<u8> },
    ListSessions,
    Sessions { ids: Vec<String> },
    WatchSession { id: String },
}

impl Message {
    pub fn start_casting(username: &str) -> Message {
        Message::StartCasting {
            username: username.to_string(),
        }
    }

    pub fn start_watching(username: &str) -> Message {
        Message::StartWatching {
            username: username.to_string(),
        }
    }

    pub fn heartbeat() -> Message {
        Message::Heartbeat
    }

    pub fn terminal_output(data: &[u8]) -> Message {
        Message::TerminalOutput {
            data: data.to_vec(),
        }
    }

    pub fn list_sessions() -> Message {
        Message::ListSessions
    }

    pub fn sessions(ids: &[String]) -> Message {
        Message::Sessions { ids: ids.to_vec() }
    }

    pub fn watch_session(id: &str) -> Message {
        Message::WatchSession { id: id.to_string() }
    }

    pub fn read<R: std::io::Read>(r: R) -> Result<Self> {
        Packet::read(r).and_then(Self::try_from)
    }

    pub fn read_async<R: tokio::io::AsyncRead>(
        r: R,
    ) -> impl futures::future::Future<Item = (Self, R), Error = Error> {
        Packet::read_async(r).and_then(|(packet, r)| {
            Self::try_from(packet).map(|msg| (msg, r))
        })
    }

    pub fn write<W: std::io::Write>(&self, w: W) -> Result<()> {
        Packet::from(self).write(w)
    }

    pub fn write_async<W: tokio::io::AsyncWrite>(
        &self,
        w: W,
    ) -> impl futures::future::Future<Item = W, Error = Error> {
        Packet::from(self).write_async(w)
    }
}

struct Packet {
    ty: u32,
    data: Vec<u8>,
}

impl Packet {
    fn read<R: std::io::Read>(mut r: R) -> Result<Self> {
        let mut header_buf = [0u8; std::mem::size_of::<u32>() * 2];
        r.read_exact(&mut header_buf).context(Read)?;

        let (len_buf, ty_buf) =
            header_buf.split_at(std::mem::size_of::<u32>());
        let len = u32::from_le_bytes(len_buf.try_into().unwrap());
        let ty = u32::from_le_bytes(ty_buf.try_into().unwrap());
        let mut data = vec![0u8; len.try_into().unwrap()];
        r.read_exact(&mut data).context(Read)?;

        Ok(Packet { ty, data })
    }

    fn read_async<R: tokio::io::AsyncRead>(
        r: R,
    ) -> impl futures::future::Future<Item = (Self, R), Error = Error> {
        let header_buf = [0u8; std::mem::size_of::<u32>() * 2];
        tokio::io::read_exact(r, header_buf)
            .and_then(|(r, buf)| {
                let (len_buf, ty_buf) =
                    buf.split_at(std::mem::size_of::<u32>());
                let len = u32::from_le_bytes(len_buf.try_into().unwrap());
                let ty = u32::from_le_bytes(ty_buf.try_into().unwrap());
                let body_buf = vec![0u8; len.try_into().unwrap()];
                tokio::io::read_exact(r, body_buf).map(move |(r, buf)| {
                    (
                        Packet {
                            ty,
                            data: buf.to_vec(),
                        },
                        r,
                    )
                })
            })
            .context(ReadAsync)
    }

    fn write<W: std::io::Write>(&self, mut w: W) -> Result<()> {
        Ok(w.write_all(&self.as_bytes()).context(Write)?)
    }

    fn write_async<W: tokio::io::AsyncWrite>(
        &self,
        w: W,
    ) -> impl futures::future::Future<Item = W, Error = Error> {
        tokio::io::write_all(w, self.as_bytes())
            .map(|(w, _)| w)
            .context(WriteAsync)
    }

    fn as_bytes(&self) -> Vec<u8> {
        let len: u32 = self.data.len().try_into().unwrap();
        let len_buf = len.to_le_bytes();
        let ty = self.ty.to_le_bytes();
        len_buf
            .iter()
            .chain(ty.iter())
            .chain(self.data.iter())
            .cloned()
            .collect()
    }
}

impl From<&Message> for Packet {
    fn from(msg: &Message) -> Self {
        match msg {
            Message::StartCasting { username } => Packet {
                ty: 0,
                data: username.as_bytes().to_vec(),
            },
            Message::StartWatching { username } => Packet {
                ty: 1,
                data: username.as_bytes().to_vec(),
            },
            Message::Heartbeat => Packet {
                ty: 2,
                data: vec![],
            },
            Message::TerminalOutput { data } => Packet {
                ty: 3,
                data: data.to_vec(),
            },
            Message::ListSessions => Packet {
                ty: 4,
                data: vec![],
            },
            Message::Sessions { ids } => {
                let mut data = vec![];
                let len: u32 = ids.len().try_into().unwrap();
                data.extend_from_slice(&len.to_le_bytes());
                for id in ids {
                    let len: u32 = id.len().try_into().unwrap();
                    data.extend_from_slice(&len.to_le_bytes());
                    data.extend_from_slice(&id.as_bytes());
                }
                Packet { ty: 5, data }
            }
            Message::WatchSession { id } => Packet {
                ty: 6,
                data: id.as_bytes().to_vec(),
            },
        }
    }
}

impl std::convert::TryFrom<Packet> for Message {
    type Error = Error;

    fn try_from(packet: Packet) -> Result<Self> {
        match packet.ty {
            0 => Ok(Message::StartCasting {
                username: String::from_utf8(packet.data)
                    .context(ParseStartCastingMessage)?,
            }),
            1 => Ok(Message::StartWatching {
                username: String::from_utf8(packet.data)
                    .context(ParseStartWatchingMessage)?,
            }),
            2 => Ok(Message::Heartbeat),
            3 => Ok(Message::TerminalOutput { data: packet.data }),
            4 => Ok(Message::ListSessions),
            5 => {
                let mut ids = vec![];
                let mut data: &[u8] = packet.data.as_ref();

                let (num_sessions_buf, rest) =
                    data.split_at(std::mem::size_of::<u32>());
                let num_sessions = u32::from_le_bytes(
                    num_sessions_buf
                        .try_into()
                        .context(ParseSessionsMessageLen)?,
                );
                data = rest;

                for _ in 0..num_sessions {
                    let (len_buf, rest) =
                        data.split_at(std::mem::size_of::<u32>());
                    let len = u32::from_le_bytes(
                        len_buf
                            .try_into()
                            .context(ParseSessionsMessageLen)?,
                    );
                    data = rest;

                    let (id_buf, rest) =
                        data.split_at(len.try_into().unwrap());
                    let id = String::from_utf8(id_buf.to_vec())
                        .context(ParseSessionsMessageId)?;
                    ids.push(id);
                    data = rest;
                }
                Ok(Message::Sessions { ids })
            }
            6 => Ok(Message::WatchSession {
                id: String::from_utf8(packet.data)
                    .context(ParseWatchSessionMessage)?,
            }),
            _ => Err(Error::InvalidMessageType { ty: packet.ty }),
        }
    }
}
