use futures::future::Future as _;
use snafu::ResultExt as _;
use tokio::io::{AsyncRead as _, AsyncWrite as _};
use tokio_pty_process::CommandExt as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("{}", source))]
    Resize { source: crate::term::Error },

    #[snafu(display("failed to open a pty: {}", source))]
    OpenPty { source: std::io::Error },

    #[snafu(display("failed to spawn process for `{}`: {}", cmd, source))]
    SpawnProcess { cmd: String, source: std::io::Error },

    #[snafu(display("failed to read from pty: {}", source))]
    ReadFromPty { source: std::io::Error },

    #[snafu(display("failed to write to pty: {}", source))]
    WriteToPty { source: std::io::Error },

    #[snafu(display("failed to read from terminal: {}", source))]
    ReadFromTerminal { source: std::io::Error },

    #[snafu(display("failed to poll for process exit: {}", source))]
    ProcessExitPoll { source: std::io::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    CommandStart(String, Vec<String>),
    Output(Vec<u8>),
    CommandExit(std::process::ExitStatus),
}

pub struct Process<R: tokio::io::AsyncRead> {
    pty: tokio_pty_process::AsyncPtyMaster,
    process: tokio_pty_process::Child,
    input: R,
    input_buf: std::collections::VecDeque<u8>,
    cmd: String,
    args: Vec<String>,
    buf: Vec<u8>,
    started: bool,
    exited: bool,
    needs_resize: Option<crate::term::Size>,
    stdin_closed: bool,
    stdout_closed: bool,
}

impl<R: tokio::io::AsyncRead + 'static> Process<R> {
    pub fn new(cmd: &str, args: &[String], input: R) -> Result<Self> {
        let pty =
            tokio_pty_process::AsyncPtyMaster::open().context(OpenPty)?;

        let process = std::process::Command::new(cmd)
            .args(args)
            .spawn_pty_async(&pty)
            .context(SpawnProcess { cmd })?;

        Ok(Self {
            pty,
            process,
            input,
            input_buf: std::collections::VecDeque::with_capacity(4096),
            cmd: cmd.to_string(),
            args: args.to_vec(),
            buf: vec![0; 4096],
            started: false,
            exited: false,
            needs_resize: None,
            stdin_closed: false,
            stdout_closed: false,
        })
    }

    pub fn resize(&mut self, size: crate::term::Size) {
        self.needs_resize = Some(size);
    }
}

impl<R: tokio::io::AsyncRead + 'static> Process<R> {
    const POLL_FNS: &'static [&'static dyn for<'a> Fn(
        &'a mut Self,
    ) -> Result<
        crate::component_future::Poll<Event>,
    >] = &[
        // order is important here - checking command_exit first so that we
        // don't try to read from a process that has already exited, which
        // causes an error
        &Self::poll_resize,
        &Self::poll_command_start,
        &Self::poll_command_exit,
        &Self::poll_read_stdin,
        &Self::poll_write_stdin,
        &Self::poll_read_stdout,
    ];

    fn poll_resize(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        if let Some(size) = &self.needs_resize {
            match size.resize_pty(&self.pty).context(Resize)? {
                futures::Async::Ready(()) => {
                    self.needs_resize = None;
                    Ok(crate::component_future::Poll::DidWork)
                }
                futures::Async::NotReady => {
                    Ok(crate::component_future::Poll::NotReady)
                }
            }
        } else {
            Ok(crate::component_future::Poll::NothingToDo)
        }
    }

    fn poll_command_start(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        if self.started {
            return Ok(crate::component_future::Poll::NothingToDo);
        }

        self.started = true;
        Ok(crate::component_future::Poll::Event(Event::CommandStart(
            self.cmd.clone(),
            self.args.clone(),
        )))
    }

    fn poll_read_stdin(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        if self.exited || self.stdin_closed {
            return Ok(crate::component_future::Poll::NothingToDo);
        }

        match self
            .input
            .poll_read(&mut self.buf)
            .context(ReadFromTerminal)?
        {
            futures::Async::Ready(n) => {
                if n > 0 {
                    self.input_buf.extend(self.buf[..n].iter());
                } else {
                    self.input_buf.push_back(b'\x04');
                    self.stdin_closed = true;
                }
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }

    fn poll_write_stdin(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        if self.exited || self.input_buf.is_empty() {
            return Ok(crate::component_future::Poll::NothingToDo);
        }

        let (a, b) = self.input_buf.as_slices();
        let buf = if a.is_empty() { b } else { a };
        match self.pty.poll_write(buf).context(WriteToPty)? {
            futures::Async::Ready(n) => {
                for _ in 0..n {
                    self.input_buf.pop_front();
                }
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }

    fn poll_read_stdout(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        match self.pty.poll_read(&mut self.buf).context(ReadFromPty) {
            Ok(futures::Async::Ready(n)) => {
                let bytes = self.buf[..n].to_vec();
                Ok(crate::component_future::Poll::Event(Event::Output(bytes)))
            }
            Ok(futures::Async::NotReady) => {
                Ok(crate::component_future::Poll::NotReady)
            }
            Err(e) => {
                // XXX this seems to be how eof is returned, but this seems...
                // wrong? i feel like there has to be a better way to do this
                if let Error::ReadFromPty { source } = &e {
                    if source.kind() == std::io::ErrorKind::Other {
                        self.stdout_closed = true;
                        return Ok(crate::component_future::Poll::DidWork);
                    }
                }
                Err(e)
            }
        }
    }

    fn poll_command_exit(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        if self.exited {
            return Ok(crate::component_future::Poll::Done);
        }
        if !self.stdout_closed {
            return Ok(crate::component_future::Poll::NothingToDo);
        }

        match self.process.poll().context(ProcessExitPoll)? {
            futures::Async::Ready(status) => {
                self.exited = true;
                Ok(crate::component_future::Poll::Event(Event::CommandExit(
                    status,
                )))
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }
}

#[must_use = "streams do nothing unless polled"]
impl<R: tokio::io::AsyncRead + 'static> futures::stream::Stream
    for Process<R>
{
    type Item = Event;
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        crate::component_future::poll_stream(self, Self::POLL_FNS)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use futures::sink::Sink as _;
    use futures::stream::Stream as _;

    #[test]
    fn test_simple() {
        let (wres, rres) = tokio::sync::mpsc::channel(100);
        let wres2 = wres.clone();
        let mut wres = wres.wait();
        let buf = std::io::Cursor::new(b"hello world\n");
        let fut = Process::new("cat", &[], buf)
            .unwrap()
            .for_each(move |e| {
                wres.send(Ok(e)).unwrap();
                Ok(())
            })
            .map_err(|e| {
                wres2.wait().send(Err(e)).unwrap();
            });
        tokio::run(fut);

        let mut rres = rres.wait();

        let event = rres.next();
        let event = event.unwrap();
        let event = event.unwrap();
        let event = event.unwrap();
        assert_eq!(event, Event::CommandStart("cat".to_string(), vec![]));

        let mut output: Vec<u8> = vec![];
        let mut exited = false;
        for event in rres {
            assert!(!exited);
            let event = event.unwrap();
            let event = event.unwrap();
            match event {
                Event::CommandStart(..) => panic!("unexpected CommandStart"),
                Event::Output(buf) => {
                    output.extend(buf.iter());
                }
                Event::CommandExit(status) => {
                    assert!(status.success());
                    exited = true;
                }
            }
        }
        assert!(exited);
        assert_eq!(output, b"hello world\r\nhello world\r\n");
    }
}
