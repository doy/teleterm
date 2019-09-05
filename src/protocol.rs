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
}

pub type Result<T> = std::result::Result<T, Error>;

pub const PROTO_VERSION: u32 = 1;

#[derive(Debug)]
pub enum Message {
    StartCasting {
        proto_version: u32,
        username: String,
        term_type: String,
    },
    StartWatching {
        proto_version: u32,
        username: String,
        term_type: String,
    },
    Heartbeat,
    TerminalOutput {
        data: Vec<u8>,
    },
    ListSessions,
    Sessions {
        ids: Vec<String>,
    },
    WatchSession {
        id: String,
    },
}

impl Message {
    pub fn start_casting(username: &str, term_type: &str) -> Message {
        Message::StartCasting {
            proto_version: PROTO_VERSION,
            username: username.to_string(),
            term_type: term_type.to_string(),
        }
    }

    pub fn start_watching(username: &str, term_type: &str) -> Message {
        Message::StartWatching {
            proto_version: PROTO_VERSION,
            username: username.to_string(),
            term_type: term_type.to_string(),
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
        fn u32_from_usize(n: usize) -> u32 {
            // XXX this can actually panic
            n.try_into().unwrap()
        }
        fn write_u32(val: u32, data: &mut Vec<u8>) {
            data.extend_from_slice(&val.to_le_bytes());
        }
        fn write_bytes(val: &[u8], data: &mut Vec<u8>) {
            write_u32(u32_from_usize(val.len()), data);
            data.extend_from_slice(val);
        }
        fn write_str(val: &str, data: &mut Vec<u8>) {
            write_bytes(val.as_bytes(), data);
        }
        fn write_strvec(val: &[String], data: &mut Vec<u8>) {
            write_u32(u32_from_usize(val.len()), data);
            for s in val {
                write_str(&s, data);
            }
        }

        match msg {
            Message::StartCasting {
                proto_version,
                username,
                term_type,
            } => {
                let mut data = vec![];

                write_u32(*proto_version, &mut data);
                write_str(username, &mut data);
                write_str(term_type, &mut data);

                Packet { ty: 0, data }
            }
            Message::StartWatching {
                proto_version,
                username,
                term_type,
            } => {
                let mut data = vec![];

                write_u32(*proto_version, &mut data);
                write_str(username, &mut data);
                write_str(term_type, &mut data);

                Packet { ty: 1, data }
            }
            Message::Heartbeat => Packet {
                ty: 2,
                data: vec![],
            },
            Message::TerminalOutput { data: output } => {
                let mut data = vec![];

                write_bytes(output, &mut data);

                Packet {
                    ty: 3,
                    data: data.to_vec(),
                }
            }
            Message::ListSessions => Packet {
                ty: 4,
                data: vec![],
            },
            Message::Sessions { ids } => {
                let mut data = vec![];

                write_strvec(ids, &mut data);

                Packet { ty: 5, data }
            }
            Message::WatchSession { id } => {
                let mut data = vec![];

                write_str(id, &mut data);

                Packet { ty: 6, data }
            }
        }
    }
}

impl std::convert::TryFrom<Packet> for Message {
    type Error = Error;

    fn try_from(packet: Packet) -> Result<Self> {
        fn read_u32(data: &[u8]) -> Result<(u32, &[u8])> {
            let (buf, rest) = data.split_at(std::mem::size_of::<u32>());
            let val = u32::from_le_bytes(buf.try_into().context(ParseInt)?);
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
        fn read_strvec(data: &[u8]) -> Result<(Vec<String>, &[u8])> {
            let mut val = vec![];
            let (len, mut data) = read_u32(data)?;
            for _ in 0..len {
                let (subval, subdata) = read_str(data)?;
                val.push(subval);
                data = subdata;
            }
            Ok((val, data))
        }

        let data: &[u8] = packet.data.as_ref();
        let (msg, rest) = match packet.ty {
            0 => {
                let (proto_version, data) = read_u32(data)?;
                let (username, data) = read_str(data)?;
                let (term_type, data) = read_str(data)?;

                (
                    Message::StartCasting {
                        proto_version,
                        username,
                        term_type,
                    },
                    data,
                )
            }
            1 => {
                let (proto_version, data) = read_u32(data)?;
                let (username, data) = read_str(data)?;
                let (term_type, data) = read_str(data)?;

                (
                    Message::StartWatching {
                        proto_version,
                        username,
                        term_type,
                    },
                    data,
                )
            }
            2 => (Message::Heartbeat, data),
            3 => {
                let (output, data) = read_bytes(data)?;

                (Message::TerminalOutput { data: output }, data)
            }
            4 => (Message::ListSessions, data),
            5 => {
                let (ids, data) = read_strvec(data)?;

                (Message::Sessions { ids }, data)
            }
            6 => {
                let (id, data) = read_str(data)?;
                (Message::WatchSession { id }, data)
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
