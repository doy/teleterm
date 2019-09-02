use futures::future::Future as _;
use snafu::futures01::FutureExt as _;
use snafu::ResultExt as _;
use std::convert::{TryFrom as _, TryInto as _};

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to read packet: {}", source))]
    ReadAsync { source: tokio::io::Error },

    #[snafu(display("failed to write packet: {}", source))]
    Write { source: std::io::Error },

    #[snafu(display("invalid StartCasting message: {}", source))]
    ParseStartCastingMessage { source: std::string::FromUtf8Error },

    #[snafu(display("invalid StartWatching message: {}", source))]
    ParseStartWatchingMessage { source: std::string::FromUtf8Error },

    #[snafu(display("invalid message type: {}", ty))]
    InvalidMessageType { ty: u32 },
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Message {
    StartCasting { username: String },
    StartWatching { username: String },
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
}

struct Packet {
    ty: u32,
    data: Vec<u8>,
}

impl Packet {
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
                futures::future::ok((r, len, ty))
            })
            .and_then(|(r, len, ty)| {
                let body_buf = vec![0u8; len as usize];
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

    fn as_bytes(&self) -> Vec<u8> {
        let len = (self.data.len() as u32).to_le_bytes();
        let ty = self.ty.to_le_bytes();
        len.iter()
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
        }
    }
}

impl std::convert::TryFrom<Packet> for Message {
    type Error = Error;

    fn try_from(packet: Packet) -> Result<Self> {
        match packet.ty {
            0 => Ok(Message::StartCasting {
                username: std::string::String::from_utf8(packet.data)
                    .context(ParseStartCastingMessage)?,
            }),
            1 => Ok(Message::StartWatching {
                username: std::string::String::from_utf8(packet.data)
                    .context(ParseStartWatchingMessage)?,
            }),
            _ => Err(Error::InvalidMessageType { ty: packet.ty }),
        }
    }
}
