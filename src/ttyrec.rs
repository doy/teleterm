use crate::prelude::*;
use std::convert::TryFrom as _;

pub struct Frame {
    pub time: std::time::Duration,
    pub data: Vec<u8>,
}

impl std::convert::TryFrom<Frame> for Vec<u8> {
    type Error = Error;

    fn try_from(frame: Frame) -> Result<Self> {
        let secs = u32::try_from(frame.time.as_secs()).map_err(|_| {
            Error::FrameTooLong {
                input: frame.time.as_secs(),
            }
        })?;
        let micros = frame.time.subsec_micros();
        let len = u32::try_from(frame.data.len()).map_err(|_| {
            Error::FrameTooBig {
                input: frame.data.len(),
            }
        })?;
        let mut bytes = vec![];
        bytes.extend(secs.to_le_bytes().iter());
        bytes.extend(micros.to_le_bytes().iter());
        bytes.extend(len.to_le_bytes().iter());
        bytes.extend(frame.data.iter());
        Ok(bytes)
    }
}

#[repr(packed)]
struct Header {
    secs: u32,
    micros: u32,
    len: u32,
}

impl Header {
    fn time(&self) -> std::time::Duration {
        std::time::Duration::from_micros(u64::from(
            self.secs * 1_000_000 + self.micros,
        ))
    }

    fn len(&self) -> usize {
        self.len as usize
    }
}

pub struct Parser {
    reading: std::collections::VecDeque<u8>,
    read_state: Option<Header>,
}

impl Parser {
    pub fn new() -> Self {
        Self {
            reading: std::collections::VecDeque::new(),
            read_state: None,
        }
    }

    pub fn add_bytes(&mut self, bytes: &[u8]) {
        self.reading.extend(bytes.iter());
    }

    pub fn next_frame(&mut self) -> Option<Frame> {
        let header = if let Some(header) = &self.read_state {
            header
        } else {
            if self.reading.len() < std::mem::size_of::<Header>() {
                return None;
            }

            let secs1 = self.reading.pop_front().unwrap();
            let secs2 = self.reading.pop_front().unwrap();
            let secs3 = self.reading.pop_front().unwrap();
            let secs4 = self.reading.pop_front().unwrap();
            let secs = u32::from_le_bytes([secs1, secs2, secs3, secs4]);

            let usecs1 = self.reading.pop_front().unwrap();
            let usecs2 = self.reading.pop_front().unwrap();
            let usecs3 = self.reading.pop_front().unwrap();
            let usecs4 = self.reading.pop_front().unwrap();
            let micros = u32::from_le_bytes([usecs1, usecs2, usecs3, usecs4]);

            let len1 = self.reading.pop_front().unwrap();
            let len2 = self.reading.pop_front().unwrap();
            let len3 = self.reading.pop_front().unwrap();
            let len4 = self.reading.pop_front().unwrap();
            let len = u32::from_le_bytes([len1, len2, len3, len4]);

            let header = Header { secs, micros, len };
            self.read_state = Some(header);
            self.read_state.as_ref().unwrap()
        };

        if self.reading.len() < header.len() {
            return None;
        }

        let mut data = vec![];
        for _ in 0..header.len() {
            data.push(self.reading.pop_front().unwrap());
        }

        let time = header.time();

        self.read_state = None;
        Some(Frame { time, data })
    }
}

pub struct Creator {
    base_time: Option<std::time::Instant>,
}

impl Creator {
    pub fn new() -> Self {
        Self { base_time: None }
    }

    pub fn frame(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        let cur_time = std::time::Instant::now();
        let base_time = if let Some(base_time) = &self.base_time {
            base_time
        } else {
            self.base_time = Some(cur_time);
            self.base_time.as_ref().unwrap()
        };
        Vec::<u8>::try_from(Frame {
            time: cur_time - *base_time,
            data: data.to_vec(),
        })
    }
}

pub struct Reader<R> {
    reader: R,
    parser: Parser,
    read_buf: [u8; 4096],
    done_reading: bool,
}

impl<R: tokio::io::AsyncRead> Reader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            parser: Parser::new(),
            read_buf: [0; 4096],
            done_reading: false,
        }
    }

    pub fn poll_read(&mut self) -> futures::Poll<Option<Frame>, Error> {
        loop {
            if let Some(frame) = self.parser.next_frame() {
                return Ok(futures::Async::Ready(Some(frame)));
            } else if self.done_reading {
                return Ok(futures::Async::Ready(None));
            }

            let n = futures::try_ready!(self
                .reader
                .poll_read(&mut self.read_buf)
                .context(crate::error::ReadFile));
            if n > 0 {
                self.parser.add_bytes(&self.read_buf[..n]);
            } else {
                self.done_reading = true;
            }
        }
    }
}

pub struct Writer<W> {
    writer: W,
    creator: Creator,
    to_write: std::collections::VecDeque<u8>,
}

impl<W: tokio::io::AsyncWrite> Writer<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            creator: Creator::new(),
            to_write: std::collections::VecDeque::new(),
        }
    }

    pub fn frame(&mut self, data: &[u8]) -> Result<()> {
        let bytes = self.creator.frame(data)?;
        self.to_write.extend(bytes.iter());
        Ok(())
    }

    pub fn poll_write(&mut self) -> futures::Poll<(), Error> {
        let (a, b) = self.to_write.as_slices();
        let buf = if a.is_empty() { b } else { a };
        let n = futures::try_ready!(self
            .writer
            .poll_write(buf)
            .context(crate::error::WriteFile));
        for _ in 0..n {
            self.to_write.pop_front();
        }
        Ok(futures::Async::Ready(()))
    }

    pub fn is_empty(&self) -> bool {
        self.to_write.is_empty()
    }
}
