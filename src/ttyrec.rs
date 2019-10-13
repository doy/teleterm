use futures::sink::Sink as _;
use futures::stream::Stream as _;
use snafu::ResultExt as _;
use std::convert::TryFrom as _;
use tokio::io::AsyncWrite as _;

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
}

#[allow(dead_code)]
pub type Result<T> = std::result::Result<T, Error>;

pub struct Frame {
    time: std::time::Instant,
    data: Vec<u8>,
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

    pub fn is_empty(&self) -> bool {
        self.writing.is_empty() && self.waiting == 0
    }
}
