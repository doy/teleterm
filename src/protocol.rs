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
    pub watchers: u32,
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

pub const PROTO_VERSION: u8 = 1;

#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum AuthType {
    Plain = 0,
    RecurseCenter,
}

impl AuthType {
    pub fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::RecurseCenter => "recurse_center",
        }
    }

    pub fn is_oauth(self) -> bool {
        match self {
            Self::Plain => false,
            Self::RecurseCenter => true,
        }
    }

    pub fn iter() -> impl Iterator<Item = Self> {
        (0..=255)
            .map(Self::try_from)
            .take_while(std::result::Result::is_ok)
            .map(std::result::Result::unwrap)
    }
}

impl std::convert::TryFrom<u8> for AuthType {
    type Error = Error;

    fn try_from(n: u8) -> Result<Self> {
        Ok(match n {
            0 => Self::Plain,
            1 => Self::RecurseCenter,
            _ => return Err(Error::InvalidAuthType { ty: n }),
        })
    }
}

impl std::convert::TryFrom<&str> for AuthType {
    type Error = Error;

    fn try_from(s: &str) -> Result<Self> {
        Ok(match s {
            s if Self::Plain.name() == s => Self::Plain,
            s if Self::RecurseCenter.name() == s => Self::RecurseCenter,
            _ => return Err(Error::InvalidAuthTypeStr { ty: s.to_string() }),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    Plain { username: String },
    RecurseCenter { id: Option<String> },
}

impl Auth {
    pub fn plain(username: &str) -> Self {
        Self::Plain {
            username: username.to_string(),
        }
    }

    pub fn recurse_center(id: Option<&str>) -> Self {
        Self::RecurseCenter {
            id: id.map(std::string::ToString::to_string),
        }
    }

    pub fn is_oauth(&self) -> bool {
        self.auth_type().is_oauth()
    }

    pub fn name(&self) -> &'static str {
        self.auth_type().name()
    }

    pub fn auth_type(&self) -> AuthType {
        match self {
            Self::Plain { .. } => AuthType::Plain,
            Self::RecurseCenter { .. } => AuthType::RecurseCenter,
        }
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum MessageType {
    Login = 0,
    StartStreaming,
    StartWatching,
    Heartbeat,
    TerminalOutput,
    ListSessions,
    Sessions,
    Disconnected,
    Error,
    Resize,
    LoggedIn,
    OauthRequest,
    OauthResponse,
}

impl std::convert::TryFrom<u8> for MessageType {
    type Error = Error;

    fn try_from(n: u8) -> Result<Self> {
        Ok(match n {
            0 => Self::Login,
            1 => Self::StartStreaming,
            2 => Self::StartWatching,
            3 => Self::Heartbeat,
            4 => Self::TerminalOutput,
            5 => Self::ListSessions,
            6 => Self::Sessions,
            7 => Self::Disconnected,
            8 => Self::Error,
            9 => Self::Resize,
            10 => Self::LoggedIn,
            11 => Self::OauthRequest,
            12 => Self::OauthResponse,
            _ => return Err(Error::InvalidMessageType { ty: n }),
        })
    }
}

// XXX https://github.com/rust-lang/rust/issues/64362
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Login {
        proto_version: u8,
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
    LoggedIn {
        username: String,
    },
    OauthRequest {
        url: String,
        id: String,
    },
    OauthResponse {
        code: String,
    },
}

impl Message {
    pub fn login(
        auth: &Auth,
        term_type: &str,
        size: &crate::term::Size,
    ) -> Self {
        Self::Login {
            proto_version: PROTO_VERSION,
            auth: auth.clone(),
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

    pub fn logged_in(username: &str) -> Self {
        Self::LoggedIn {
            username: username.to_string(),
        }
    }

    pub fn oauth_request(url: &str, id: &str) -> Self {
        Self::OauthRequest {
            url: url.to_string(),
            id: id.to_string(),
        }
    }

    pub fn oauth_response(code: &str) -> Self {
        Self::OauthResponse {
            code: code.to_string(),
        }
    }

    pub fn message_type(&self) -> MessageType {
        match self {
            Self::Login { .. } => MessageType::Login,
            Self::StartStreaming { .. } => MessageType::StartStreaming,
            Self::StartWatching { .. } => MessageType::StartWatching,
            Self::Heartbeat { .. } => MessageType::Heartbeat,
            Self::TerminalOutput { .. } => MessageType::TerminalOutput,
            Self::ListSessions { .. } => MessageType::ListSessions,
            Self::Sessions { .. } => MessageType::Sessions,
            Self::Disconnected { .. } => MessageType::Disconnected,
            Self::Error { .. } => MessageType::Error,
            Self::Resize { .. } => MessageType::Resize,
            Self::LoggedIn { .. } => MessageType::LoggedIn,
            Self::OauthRequest { .. } => MessageType::OauthRequest,
            Self::OauthResponse { .. } => MessageType::OauthResponse,
        }
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
            // these are security-sensitive, keep them out of logs
            Self::OauthRequest { .. } => {
                log::debug!("{}: message(OauthRequest {{ .. }})", id);
            }
            Self::OauthResponse { .. } => {
                log::debug!("{}: message(OauthResponse {{ .. }})", id);
            }
            message => {
                log::debug!("{}: message({:?})", id, message);
            }
        }
    }
}

struct Packet {
    ty: u8,
    data: Vec<u8>,
}

impl Packet {
    fn read<R: std::io::Read>(mut r: R) -> Result<Self> {
        let mut len_buf = [0_u8; std::mem::size_of::<u32>()];
        r.read_exact(&mut len_buf)
            .context(crate::error::ReadPacket)?;
        let len = u32::from_be_bytes(len_buf.try_into().unwrap());
        if (len as usize) < std::mem::size_of::<u8>() {
            return Err(Error::LenTooSmall {
                len,
                expected: std::mem::size_of::<u8>(),
            });
        }

        let mut data = vec![0_u8; len as usize];
        r.read_exact(&mut data).context(crate::error::ReadPacket)?;
        let (ty_buf, rest) = data.split_at(std::mem::size_of::<u8>());
        let ty = u8::from_be_bytes(ty_buf.try_into().unwrap());

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
                if buf.len() < std::mem::size_of::<u8>() {
                    return Err(Error::LenTooSmall {
                        len: buf.len().try_into().unwrap(),
                        expected: std::mem::size_of::<u8>(),
                    });
                }
                let (ty_buf, data_buf) =
                    buf.split_at(std::mem::size_of::<u8>());
                let ty = u8::from_be_bytes(ty_buf.try_into().unwrap());
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
        fn write_u8(val: u8, data: &mut Vec<u8>) {
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
            write_u32(val.watchers, data);
        }
        fn write_sessions(val: &[Session], data: &mut Vec<u8>) {
            write_u32(u32_from_usize(val.len()), data);
            for s in val {
                write_session(s, data);
            }
        }
        fn write_auth(val: &Auth, data: &mut Vec<u8>) {
            write_u8(val.auth_type() as u8, data);
            match val {
                Auth::Plain { username } => {
                    write_str(username, data);
                }
                Auth::RecurseCenter { id } => {
                    let id = id.as_ref().map_or("", |s| s.as_str());
                    write_str(id, data);
                }
            }
        }

        let ty = msg.message_type() as u8;
        let mut data = vec![];

        match msg {
            Message::Login {
                proto_version,
                auth,
                term_type,
                size,
            } => {
                write_u8(*proto_version, &mut data);
                write_auth(auth, &mut data);
                write_str(term_type, &mut data);
                write_size(size, &mut data);
            }
            Message::StartStreaming => {}
            Message::StartWatching { id } => {
                write_str(id, &mut data);
            }
            Message::Heartbeat => {}
            Message::TerminalOutput { data: output } => {
                write_bytes(output, &mut data);
            }
            Message::ListSessions => {}
            Message::Sessions { sessions } => {
                write_sessions(sessions, &mut data);
            }
            Message::Disconnected => {}
            Message::Error { msg } => {
                write_str(msg, &mut data);
            }
            Message::Resize { size } => {
                write_size(size, &mut data);
            }
            Message::LoggedIn { username } => {
                write_str(username, &mut data);
            }
            Message::OauthRequest { url, id } => {
                write_str(url, &mut data);
                write_str(id, &mut data);
            }
            Message::OauthResponse { code } => {
                write_str(code, &mut data);
            }
        }

        Self { ty, data }
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
        fn read_u8(data: &[u8]) -> Result<(u8, &[u8])> {
            if std::mem::size_of::<u8>() > data.len() {
                return Err(Error::LenTooBig {
                    len: std::mem::size_of::<u8>().try_into().unwrap(),
                    expected: data.len(),
                });
            }
            let (buf, rest) = data.split_at(std::mem::size_of::<u8>());
            let val = u8::from_be_bytes(
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
            let (watchers, data) = read_u32(data)?;
            Ok((
                Session {
                    id,
                    username,
                    term_type,
                    size,
                    idle_time,
                    title,
                    watchers,
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
            let (ty, data) = read_u8(data)?;
            let ty = AuthType::try_from(ty)?;
            let (auth, data) = match ty {
                AuthType::Plain => {
                    let (username, data) = read_str(data)?;
                    let auth = Auth::Plain { username };
                    (auth, data)
                }
                AuthType::RecurseCenter => {
                    let (id, data) = read_str(data)?;
                    let id = if id == "" { None } else { Some(id) };
                    let auth = Auth::RecurseCenter { id };
                    (auth, data)
                }
            };
            Ok((auth, data))
        }

        let ty = MessageType::try_from(packet.ty)?;
        let data: &[u8] = packet.data.as_ref();
        let (msg, rest) = match ty {
            MessageType::Login => {
                let (proto_version, data) = read_u8(data)?;
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
            MessageType::StartStreaming => (Self::StartStreaming, data),
            MessageType::StartWatching => {
                let (id, data) = read_str(data)?;

                (Self::StartWatching { id }, data)
            }
            MessageType::Heartbeat => (Self::Heartbeat, data),
            MessageType::TerminalOutput => {
                let (output, data) = read_bytes(data)?;

                (Self::TerminalOutput { data: output }, data)
            }
            MessageType::ListSessions => (Self::ListSessions, data),
            MessageType::Sessions => {
                let (sessions, data) = read_sessions(data)?;

                (Self::Sessions { sessions }, data)
            }
            MessageType::Disconnected => (Self::Disconnected, data),
            MessageType::Error => {
                let (msg, data) = read_str(data)?;

                (Self::Error { msg }, data)
            }
            MessageType::Resize => {
                let (size, data) = read_size(data)?;

                (Self::Resize { size }, data)
            }
            MessageType::LoggedIn => {
                let (username, data) = read_str(data)?;

                (Self::LoggedIn { username }, data)
            }
            MessageType::OauthRequest => {
                let (url, data) = read_str(data)?;
                let (id, data) = read_str(data)?;

                (Self::OauthRequest { url, id }, data)
            }
            MessageType::OauthResponse => {
                let (code, data) = read_str(data)?;

                (Self::OauthResponse { code }, data)
            }
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

    #[test]
    fn test_auth_values() {
        let mut set = std::collections::HashSet::new();
        let mut seen_err = false;
        for i in 0..=255 {
            if seen_err {
                assert!(AuthType::try_from(i).is_err());
            } else {
                match AuthType::try_from(i) {
                    Ok(ty) => {
                        assert!(!set.contains(&ty));
                        set.insert(ty);
                    }
                    Err(_) => {
                        seen_err = true;
                    }
                }
            }
        }
    }

    #[test]
    fn test_message_values() {
        let mut set = std::collections::HashSet::new();
        let mut seen_err = false;
        for i in 0..=255 {
            if seen_err {
                assert!(MessageType::try_from(i).is_err());
            } else {
                match MessageType::try_from(i) {
                    Ok(ty) => {
                        assert!(!set.contains(&ty));
                        set.insert(ty);
                    }
                    Err(_) => {
                        seen_err = true;
                    }
                }
            }
        }
    }

    fn valid_messages() -> Vec<Message> {
        vec![
            Message::login(
                &Auth::Plain {
                    username: "doy".to_string(),
                },
                "screen",
                &crate::term::Size { rows: 24, cols: 80 },
            ),
            Message::login(
                &Auth::RecurseCenter {
                    id: Some("some-random-id".to_string()),
                },
                "screen",
                &crate::term::Size { rows: 24, cols: 80 },
            ),
            Message::login(
                &Auth::RecurseCenter { id: None },
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
                watchers: 0,
            }]),
            Message::sessions(&[
                Session {
                    id: "some-session-id".to_string(),
                    username: "doy".to_string(),
                    term_type: "screen".to_string(),
                    size: crate::term::Size { rows: 24, cols: 80 },
                    idle_time: 123,
                    title: "it's my terminal title".to_string(),
                    watchers: 0,
                },
                Session {
                    id: "some-other-session-id".to_string(),
                    username: "sartak".to_string(),
                    term_type: "screen".to_string(),
                    size: crate::term::Size { rows: 24, cols: 80 },
                    idle_time: 68,
                    title: "some other terminal title".to_string(),
                    watchers: 0,
                },
            ]),
            Message::disconnected(),
            Message::error("error message"),
            Message::resize(&crate::term::Size { rows: 25, cols: 81 }),
            Message::logged_in("doy"),
        ]
    }

    fn invalid_messages() -> Vec<Vec<u8>> {
        vec![
            b"".to_vec(),
            b"\x04".to_vec(),
            b"\x00\x00\x00\x00".to_vec(),
            b"\x00\x00\x00\x01\x00".to_vec(),
            b"\x00\x00\x00\x01\xff".to_vec(),
            b"\x00\x00\x00\x00\x01".to_vec(),
            b"\x00\x00\x00\x02\x01".to_vec(),
            b"\xee\xee\xee\xee\x01".to_vec(),
            b"\x00\x00\x00\x06\x08\x00\x00\x00\x01\xff".to_vec(),
        ]
    }
}
