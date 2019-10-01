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
#[derive(Debug, Clone)]
pub enum Message {
    Login {
        proto_version: u32,
        username: String,
        term_type: String,
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
        ids: Vec<String>,
    },
    Disconnected,
    Error {
        msg: String,
    },
}

impl Message {
    pub fn login(username: &str, term_type: &str) -> Self {
        Self::Login {
            proto_version: PROTO_VERSION,
            username: username.to_string(),
            term_type: term_type.to_string(),
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

    pub fn sessions(ids: &[String]) -> Self {
        Self::Sessions { ids: ids.to_vec() }
    }

    pub fn disconnected() -> Self {
        Self::Disconnected
    }

    pub fn error(msg: &str) -> Self {
        Self::Error {
            msg: msg.to_string(),
        }
    }

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
        fn write_strvec(val: &[String], data: &mut Vec<u8>) {
            write_u32(u32_from_usize(val.len()), data);
            for s in val {
                write_str(s, data);
            }
        }

        match msg {
            Message::Login {
                proto_version,
                username,
                term_type,
            } => {
                let mut data = vec![];

                write_u32(*proto_version, &mut data);
                write_str(username, &mut data);
                write_str(term_type, &mut data);

                Self { ty: 0, data }
            }
            Message::StartCasting => Self {
                ty: 1,
                data: vec![],
            },
            Message::StartWatching { id } => {
                let mut data = vec![];

                write_str(id, &mut data);

                Self { ty: 2, data }
            }
            Message::Heartbeat => Self {
                ty: 3,
                data: vec![],
            },
            Message::TerminalOutput { data: output } => {
                let mut data = vec![];

                write_bytes(output, &mut data);

                Self {
                    ty: 4,
                    data: data.to_vec(),
                }
            }
            Message::ListSessions => Self {
                ty: 5,
                data: vec![],
            },
            Message::Sessions { ids } => {
                let mut data = vec![];

                write_strvec(ids, &mut data);

                Self { ty: 6, data }
            }
            Message::Disconnected => Self {
                ty: 7,
                data: vec![],
            },
            Message::Error { msg } => {
                let mut data = vec![];

                write_str(msg, &mut data);

                Self { ty: 8, data }
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
                    Self::Login {
                        proto_version,
                        username,
                        term_type,
                    },
                    data,
                )
            }
            1 => (Self::StartCasting, data),
            2 => {
                let (id, data) = read_str(data)?;

                (Self::StartWatching { id }, data)
            }
            3 => (Self::Heartbeat, data),
            4 => {
                let (output, data) = read_bytes(data)?;

                (Self::TerminalOutput { data: output }, data)
            }
            5 => (Self::ListSessions, data),
            6 => {
                let (ids, data) = read_strvec(data)?;

                (Self::Sessions { ids }, data)
            }
            7 => (Self::Disconnected, data),
            8 => {
                let (msg, data) = read_str(data)?;
                (Self::Error { msg }, data)
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
