use crate::prelude::*;
use std::convert::{TryFrom as _, TryInto as _};

pub type FramedReadHalf<S> = FramedReader<tokio::io::ReadHalf<S>>;
pub type FramedWriteHalf<S> = FramedWriter<tokio::io::WriteHalf<S>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub username: String,
    pub term_type: String,
    pub size: crate::term::Size,
    pub idle_time: u32,
    pub title: String,
}

pub struct FramedReader<T: tokio::io::AsyncRead>(
    tokio::codec::FramedRead<
        T,
        tokio::codec::length_delimited::LengthDelimitedCodec,
    >,
);

impl<T: tokio::io::AsyncRead> FramedReader<T> {
    pub fn new(rs: T, buffer_size: usize) -> Self {
        Self(
            tokio::codec::length_delimited::Builder::new()
                .length_field_length(4)
                .max_frame_length(buffer_size + 1024 * 1024)
                .new_read(rs),
        )
    }
}

pub struct FramedWriter<T: tokio::io::AsyncWrite>(
    tokio::codec::FramedWrite<
        T,
        tokio::codec::length_delimited::LengthDelimitedCodec,
    >,
);

impl<T: tokio::io::AsyncWrite> FramedWriter<T> {
    pub fn new(ws: T, buffer_size: usize) -> Self {
        Self(
            tokio::codec::length_delimited::Builder::new()
                .length_field_length(4)
                .max_frame_length(buffer_size + 1024 * 1024)
                .new_write(ws),
        )
    }
}

pub const PROTO_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    Plain { username: String },
}

const AUTH_PLAIN: u32 = 0;

// XXX https://github.com/rust-lang/rust/issues/64362
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Login {
        proto_version: u32,
        auth: Auth,
        term_type: String,
        size: crate::term::Size,
    },
    StartStreaming,
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
        size: crate::term::Size,
    },
}

const MSG_LOGIN: u32 = 0;
const MSG_START_STREAMING: u32 = 1;
const MSG_START_WATCHING: u32 = 2;
const MSG_HEARTBEAT: u32 = 3;
const MSG_TERMINAL_OUTPUT: u32 = 4;
const MSG_LIST_SESSIONS: u32 = 5;
const MSG_SESSIONS: u32 = 6;
const MSG_DISCONNECTED: u32 = 7;
const MSG_ERROR: u32 = 8;
const MSG_RESIZE: u32 = 9;

impl Message {
    pub fn login_plain(
        username: &str,
        term_type: &str,
        size: &crate::term::Size,
    ) -> Self {
        Self::Login {
            proto_version: PROTO_VERSION,
            auth: Auth::Plain {
                username: username.to_string(),
            },
            term_type: term_type.to_string(),
            size: size.clone(),
        }
    }

    pub fn start_streaming() -> Self {
        Self::StartStreaming
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

    pub fn resize(size: &crate::term::Size) -> Self {
        Self::Resize { size: size.clone() }
    }

    #[allow(dead_code)]
    pub fn read<R: std::io::Read>(r: R) -> Result<Self> {
        Packet::read(r).and_then(Self::try_from)
    }

    pub fn read_async<T: tokio::io::AsyncRead>(
        r: FramedReader<T>,
    ) -> impl futures::future::Future<Item = (Self, FramedReader<T>), Error = Error>
    {
        Packet::read_async(r).and_then(|(packet, r)| {
            Self::try_from(packet).map(|msg| (msg, r))
        })
    }

    #[allow(dead_code)]
    pub fn write<W: std::io::Write>(&self, w: W) -> Result<()> {
        Packet::from(self).write(w)
    }

    pub fn write_async<T: tokio::io::AsyncWrite>(
        &self,
        w: FramedWriter<T>,
    ) -> impl futures::future::Future<Item = FramedWriter<T>, Error = Error>
    {
        Packet::from(self).write_async(w)
    }

    // it'd be nice if i could just override the Debug implementation for
    // specific enum variants, but writing the whole impl Debug by hand just
    // to make this one change would be super obnoxious
    pub fn log(&self, id: &str) {
        match self {
            Self::TerminalOutput { data } => {
                log::debug!(
                    "{}: message(TerminalOutput {{ data: ({} bytes) }})",
                    id,
                    data.len()
                );
            }
            message => {
                log::debug!("{}: message({:?})", id, message);
            }
        }
    }
}

struct Packet {
    ty: u32,
    data: Vec<u8>,
}

impl Packet {
    fn read<R: std::io::Read>(mut r: R) -> Result<Self> {
        let mut len_buf = [0_u8; std::mem::size_of::<u32>()];
        r.read_exact(&mut len_buf)
            .context(crate::error::ReadPacket)?;
        let len = u32::from_be_bytes(len_buf.try_into().unwrap());
        if (len as usize) < std::mem::size_of::<u32>() {
            return Err(Error::LenTooSmall {
                len,
                expected: std::mem::size_of::<u32>(),
            });
        }

        let mut data = vec![0_u8; len as usize];
        r.read_exact(&mut data).context(crate::error::ReadPacket)?;
        let (ty_buf, rest) = data.split_at(std::mem::size_of::<u32>());
        let ty = u32::from_be_bytes(ty_buf.try_into().unwrap());

        Ok(Self {
            ty,
            data: rest.to_vec(),
        })
    }

    fn read_async<T: tokio::io::AsyncRead>(
        r: FramedReader<T>,
    ) -> impl futures::future::Future<Item = (Self, FramedReader<T>), Error = Error>
    {
        r.0.into_future()
            .map_err(|(e, _)| Error::ReadPacketAsync { source: e })
            .and_then(|(data, r)| match data {
                Some(data) => Ok((data, r)),
                None => Err(Error::EOF),
            })
            .and_then(|(buf, r)| {
                if buf.len() < std::mem::size_of::<u32>() {
                    return Err(Error::LenTooSmall {
                        len: buf.len().try_into().unwrap(),
                        expected: std::mem::size_of::<u32>(),
                    });
                }
                let (ty_buf, data_buf) =
                    buf.split_at(std::mem::size_of::<u32>());
                let ty = u32::from_be_bytes(ty_buf.try_into().unwrap());
                let data = data_buf.to_vec();
                Ok((Self { ty, data }, FramedReader(r)))
            })
    }

    fn write<W: std::io::Write>(&self, mut w: W) -> Result<()> {
        let bytes = self.as_bytes();
        let len: u32 = bytes.len().try_into().unwrap();
        let len_buf = len.to_be_bytes();
        let buf: Vec<u8> =
            len_buf.iter().chain(bytes.iter()).copied().collect();
        Ok(w.write_all(&buf).context(crate::error::WritePacket)?)
    }

    fn write_async<T: tokio::io::AsyncWrite>(
        &self,
        w: FramedWriter<T>,
    ) -> impl futures::future::Future<Item = FramedWriter<T>, Error = Error>
    {
        w.0.send(bytes::Bytes::from(self.as_bytes()))
            .map(FramedWriter)
            .context(crate::error::WritePacketAsync)
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
            n.try_into().unwrap()
        }
        fn write_u32(val: u32, data: &mut Vec<u8>) {
            data.extend_from_slice(&val.to_be_bytes());
        }
        fn write_u16(val: u16, data: &mut Vec<u8>) {
            data.extend_from_slice(&val.to_be_bytes());
        }
        fn write_bytes(val: &[u8], data: &mut Vec<u8>) {
            write_u32(u32_from_usize(val.len()), data);
            data.extend_from_slice(val);
        }
        fn write_str(val: &str, data: &mut Vec<u8>) {
            write_bytes(val.as_bytes(), data);
        }
        fn write_size(val: &crate::term::Size, data: &mut Vec<u8>) {
            write_u16(val.rows, data);
            write_u16(val.cols, data);
        }
        fn write_session(val: &Session, data: &mut Vec<u8>) {
            write_str(&val.id, data);
            write_str(&val.username, data);
            write_str(&val.term_type, data);
            write_size(&val.size, data);
            write_u32(val.idle_time, data);
            write_str(&val.title, data);
        }
        fn write_sessions(val: &[Session], data: &mut Vec<u8>) {
            write_u32(u32_from_usize(val.len()), data);
            for s in val {
                write_session(s, data);
            }
        }
        fn write_auth(val: &Auth, data: &mut Vec<u8>) {
            match val {
                Auth::Plain { username } => {
                    write_u32(AUTH_PLAIN, data);
                    write_str(username, data);
                }
            }
        }

        match msg {
            Message::Login {
                proto_version,
                auth,
                term_type,
                size,
            } => {
                let mut data = vec![];

                write_u32(*proto_version, &mut data);
                write_auth(auth, &mut data);
                write_str(term_type, &mut data);
                write_size(size, &mut data);

                Self {
                    ty: MSG_LOGIN,
                    data,
                }
            }
            Message::StartStreaming => Self {
                ty: MSG_START_STREAMING,
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

                write_size(size, &mut data);

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
            if std::mem::size_of::<u32>() > data.len() {
                return Err(Error::LenTooBig {
                    len: std::mem::size_of::<u32>().try_into().unwrap(),
                    expected: data.len(),
                });
            }
            let (buf, rest) = data.split_at(std::mem::size_of::<u32>());
            let val = u32::from_be_bytes(
                buf.try_into().context(crate::error::ParseInt)?,
            );
            Ok((val, rest))
        }
        fn read_u16(data: &[u8]) -> Result<(u16, &[u8])> {
            if std::mem::size_of::<u16>() > data.len() {
                return Err(Error::LenTooBig {
                    len: std::mem::size_of::<u16>().try_into().unwrap(),
                    expected: data.len(),
                });
            }
            let (buf, rest) = data.split_at(std::mem::size_of::<u16>());
            let val = u16::from_be_bytes(
                buf.try_into().context(crate::error::ParseInt)?,
            );
            Ok((val, rest))
        }
        fn read_bytes(data: &[u8]) -> Result<(Vec<u8>, &[u8])> {
            let (len, data) = read_u32(data)?;
            if len as usize > data.len() {
                return Err(Error::LenTooBig {
                    len,
                    expected: data.len(),
                });
            }
            let (buf, rest) = data.split_at(len as usize);
            let val = buf.to_vec();
            Ok((val, rest))
        }
        fn read_str(data: &[u8]) -> Result<(String, &[u8])> {
            let (bytes, rest) = read_bytes(data)?;
            let val = String::from_utf8(bytes)
                .context(crate::error::ParseString)?;
            Ok((val, rest))
        }
        fn read_size(data: &[u8]) -> Result<(crate::term::Size, &[u8])> {
            let (rows, data) = read_u16(data)?;
            let (cols, data) = read_u16(data)?;
            Ok((crate::term::Size { rows, cols }, data))
        }
        fn read_session(data: &[u8]) -> Result<(Session, &[u8])> {
            let (id, data) = read_str(data)?;
            let (username, data) = read_str(data)?;
            let (term_type, data) = read_str(data)?;
            let (size, data) = read_size(data)?;
            let (idle_time, data) = read_u32(data)?;
            let (title, data) = read_str(data)?;
            Ok((
                Session {
                    id,
                    username,
                    term_type,
                    size,
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
        fn read_auth(data: &[u8]) -> Result<(Auth, &[u8])> {
            let (ty, data) = read_u32(data)?;
            let (auth, data) = match ty {
                AUTH_PLAIN => {
                    let (username, data) = read_str(data)?;
                    let auth = Auth::Plain { username };
                    (auth, data)
                }
                _ => return Err(Error::InvalidAuthType { ty }),
            };
            Ok((auth, data))
        }

        let data: &[u8] = packet.data.as_ref();
        let (msg, rest) = match packet.ty {
            MSG_LOGIN => {
                let (proto_version, data) = read_u32(data)?;
                let (auth, data) = read_auth(data)?;
                let (term_type, data) = read_str(data)?;
                let (size, data) = read_size(data)?;

                (
                    Self::Login {
                        proto_version,
                        auth,
                        term_type,
                        size,
                    },
                    data,
                )
            }
            MSG_START_STREAMING => (Self::StartStreaming, data),
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
                let (size, data) = read_size(data)?;

                (Self::Resize { size }, data)
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
mod test {
    use super::*;

    #[test]
    fn test_serialize_deserialize() {
        for msg in valid_messages() {
            let packet = Packet::from(&msg);
            let msg2 = Message::try_from(packet).unwrap();
            assert_eq!(msg, msg2);
        }
    }

    #[test]
    fn test_read_write() {
        for msg in valid_messages() {
            let mut buf = vec![];
            msg.write(&mut buf).unwrap();
            let msg2 = Message::read(buf.as_slice()).unwrap();
            assert_eq!(msg, msg2);
        }
    }

    #[test]
    fn test_read_write_async() {
        for msg in valid_messages() {
            let (wres, rres) = tokio::sync::mpsc::channel(1);
            let wres2 = wres.clone();
            let buf = std::io::Cursor::new(vec![]);
            let fut = msg
                .write_async(FramedWriter::new(buf, 4_194_304))
                .and_then(|w| {
                    let mut buf = w.0.into_inner();
                    buf.set_position(0);
                    Message::read_async(FramedReader::new(buf, 4_194_304))
                })
                .and_then(move |(msg2, _)| {
                    wres.wait().send(Ok(msg2)).unwrap();
                    futures::future::ok(())
                })
                .map_err(|e| {
                    wres2.wait().send(Err(e)).unwrap();
                });
            tokio::run(fut);
            let msg2 = rres.wait().next();
            let msg2 = msg2.unwrap();
            let msg2 = msg2.unwrap();
            let msg2 = msg2.unwrap();
            assert_eq!(msg, msg2);
        }
    }

    #[test]
    fn test_invalid_sync() {
        for buf in invalid_messages() {
            let res = Message::read(buf.as_slice());
            assert!(res.is_err())
        }
    }

    #[test]
    fn test_invalid_async() {
        for buf in invalid_messages() {
            let (wres, rres) = tokio::sync::mpsc::channel(1);
            let wres2 = wres.clone();
            let buf = std::io::Cursor::new(buf);
            let fut = Message::read_async(FramedReader::new(buf, 4_194_304))
                .and_then(move |(msg2, _)| {
                    wres.wait().send(Ok(msg2)).unwrap();
                    futures::future::ok(())
                })
                .map_err(|e| {
                    wres2.wait().send(Err(e)).unwrap();
                });
            tokio::run(fut);
            let res = rres.wait().next();
            let res = res.unwrap();
            let res = res.unwrap();
            assert!(res.is_err());
        }
    }

    fn valid_messages() -> Vec<Message> {
        vec![
            Message::login_plain(
                "doy",
                "screen",
                &crate::term::Size { rows: 24, cols: 80 },
            ),
            Message::start_streaming(),
            Message::start_watching("some-session-id"),
            Message::heartbeat(),
            Message::terminal_output(b"foobar"),
            Message::terminal_output(b""),
            Message::list_sessions(),
            Message::sessions(&[]),
            Message::sessions(&[Session {
                id: "some-session-id".to_string(),
                username: "doy".to_string(),
                term_type: "screen".to_string(),
                size: crate::term::Size { rows: 24, cols: 80 },
                idle_time: 123,
                title: "it's my terminal title".to_string(),
            }]),
            Message::sessions(&[
                Session {
                    id: "some-session-id".to_string(),
                    username: "doy".to_string(),
                    term_type: "screen".to_string(),
                    size: crate::term::Size { rows: 24, cols: 80 },
                    idle_time: 123,
                    title: "it's my terminal title".to_string(),
                },
                Session {
                    id: "some-other-session-id".to_string(),
                    username: "sartak".to_string(),
                    term_type: "screen".to_string(),
                    size: crate::term::Size { rows: 24, cols: 80 },
                    idle_time: 68,
                    title: "some other terminal title".to_string(),
                },
            ]),
            Message::disconnected(),
            Message::error("error message"),
            Message::resize(&crate::term::Size { rows: 25, cols: 81 }),
        ]
    }

    fn invalid_messages() -> Vec<Vec<u8>> {
        vec![
            b"".to_vec(),
            b"\x04".to_vec(),
            b"\x00\x00\x00\x00".to_vec(),
            b"\x00\x00\x00\x04\x00\x00\x00\x00".to_vec(),
            b"\x00\x00\x00\x04\x00\x00\x00\xff".to_vec(),
            b"\x00\x00\x00\x03\x00\x00\x00\x01".to_vec(),
            b"\x00\x00\x00\x05\x00\x00\x00\x01".to_vec(),
            b"\xee\xee\xee\xee\x00\x00\x00\x01".to_vec(),
            b"\x00\x00\x00\x09\x00\x00\x00\x08\x00\x00\x00\x01\xff".to_vec(),
        ]
    }
}
