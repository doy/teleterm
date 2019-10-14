use crate::prelude::*;
use std::convert::TryFrom as _;
use tokio::io::{AsyncRead as _, AsyncWrite as _};

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to read from event channel: {}", source))]
    ReadChannel {
        source: tokio::sync::mpsc::error::UnboundedRecvError,
    },

    #[snafu(display("failed to write to event channel: {}", source))]
    WriteChannel {
        source: tokio::sync::mpsc::error::UnboundedSendError,
    },

    #[snafu(display("failed to write to file: {}", source))]
    WriteFile { source: tokio::io::Error },

    #[snafu(display("failed to read from file: {}", source))]
    ReadFile { source: tokio::io::Error },
}

#[allow(dead_code)]
pub type Result<T> = std::result::Result<T, Error>;

pub struct Frame {
    pub time: std::time::Instant,
    pub data: Vec<u8>,
}

impl Frame {
    fn as_bytes(&self, base: std::time::Instant) -> Vec<u8> {
        let dur = self.time - base;
        let secs = u32::try_from(dur.as_secs()).unwrap();
        let micros = dur.subsec_micros();
        let len = u32::try_from(self.data.len()).unwrap();
        let mut bytes = vec![];
        bytes.extend(secs.to_le_bytes().iter());
        bytes.extend(micros.to_le_bytes().iter());
        bytes.extend(len.to_le_bytes().iter());
        bytes.extend(self.data.iter());
        bytes
    }
}

pub struct File {
    file: tokio::fs::File,
    base_time: Option<std::time::Instant>,
    waiting: usize,
    wframe: futures::sink::Wait<tokio::sync::mpsc::UnboundedSender<Frame>>,
    rframe: tokio::sync::mpsc::UnboundedReceiver<Frame>,
    writing: std::collections::VecDeque<u8>,
    read_buf: [u8; 4096],
    reading: std::collections::VecDeque<u8>,
    read_state: Option<(u32, u32, u32)>,
    read: std::collections::VecDeque<Frame>,
}

impl File {
    pub fn new(file: tokio::fs::File) -> Self {
        let (wframe, rframe) = tokio::sync::mpsc::unbounded_channel();
        Self {
            file,
            base_time: None,
            waiting: 0,
            wframe: wframe.wait(),
            rframe,
            writing: std::collections::VecDeque::new(),
            read_buf: [0; 4096],
            reading: std::collections::VecDeque::new(),
            read_state: None,
            read: std::collections::VecDeque::new(),
        }
    }

    pub fn write_frame(&mut self, data: &[u8]) -> Result<()> {
        let now = std::time::Instant::now();
        if self.base_time.is_none() {
            self.base_time = Some(now);
        }

        self.waiting += 1;
        self.wframe
            .send(Frame {
                time: now,
                data: data.to_vec(),
            })
            .context(WriteChannel)
    }

    pub fn poll_write(
        &mut self,
    ) -> std::result::Result<futures::Async<()>, Error> {
        loop {
            if self.writing.is_empty() {
                match self.rframe.poll().context(ReadChannel)? {
                    futures::Async::Ready(Some(frame)) => {
                        self.writing.extend(
                            frame.as_bytes(self.base_time.unwrap()).iter(),
                        );
                        self.waiting -= 1;
                    }
                    futures::Async::Ready(None) => unreachable!(),
                    futures::Async::NotReady => {
                        return Ok(futures::Async::NotReady);
                    }
                }
            }

            let (a, b) = self.writing.as_slices();
            let buf = if a.is_empty() { b } else { a };
            match self.file.poll_write(buf).context(WriteFile)? {
                futures::Async::Ready(n) => {
                    for _ in 0..n {
                        self.writing.pop_front();
                    }
                }
                futures::Async::NotReady => {
                    return Ok(futures::Async::NotReady);
                }
            }
        }
    }

    pub fn poll_read(
        &mut self,
    ) -> std::result::Result<futures::Async<Option<Frame>>, Error> {
        loop {
            if let Some(frame) = self.read.pop_front() {
                return Ok(futures::Async::Ready(Some(frame)));
            }

            match self.file.poll_read(&mut self.read_buf).context(ReadFile)? {
                futures::Async::Ready(n) => {
                    if n > 0 {
                        self.reading.extend(self.read_buf[..n].iter());
                        self.parse_frames();
                    } else {
                        return Ok(futures::Async::Ready(None));
                    }
                }
                futures::Async::NotReady => {
                    return Ok(futures::Async::NotReady);
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.writing.is_empty() && self.waiting == 0
    }

    fn parse_frames(&mut self) {
        loop {
            match self.read_state {
                Some((secs, usecs, len)) => {
                    if self.reading.len() < len as usize {
                        break;
                    }

                    let mut data = vec![];
                    for _ in 0..len {
                        data.push(self.reading.pop_front().unwrap());
                    }

                    if self.base_time.is_none() {
                        self.base_time = Some(std::time::Instant::now());
                    }
                    let dur = std::time::Duration::from_micros(u64::from(
                        secs * 1_000_000 + usecs,
                    ));
                    let time = self.base_time.unwrap() + dur;

                    self.read.push_back(Frame { time, data });

                    self.read_state = None;
                }
                None => {
                    if self.reading.len() < 12 {
                        break;
                    }

                    let secs1 = self.reading.pop_front().unwrap();
                    let secs2 = self.reading.pop_front().unwrap();
                    let secs3 = self.reading.pop_front().unwrap();
                    let secs4 = self.reading.pop_front().unwrap();
                    let secs =
                        u32::from_le_bytes([secs1, secs2, secs3, secs4]);
                    let usecs1 = self.reading.pop_front().unwrap();
                    let usecs2 = self.reading.pop_front().unwrap();
                    let usecs3 = self.reading.pop_front().unwrap();
                    let usecs4 = self.reading.pop_front().unwrap();
                    let usecs =
                        u32::from_le_bytes([usecs1, usecs2, usecs3, usecs4]);
                    let len1 = self.reading.pop_front().unwrap();
                    let len2 = self.reading.pop_front().unwrap();
                    let len3 = self.reading.pop_front().unwrap();
                    let len4 = self.reading.pop_front().unwrap();
                    let len = u32::from_le_bytes([len1, len2, len3, len4]);

                    self.read_state = Some((secs, usecs, len));
                }
            }
        }
    }
}
