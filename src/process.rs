use futures::future::Future as _;
use snafu::ResultExt as _;
use tokio::io::{AsyncRead as _, AsyncWrite as _};
use tokio_pty_process::CommandExt as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to open a pty: {}", source))]
    OpenPty { source: std::io::Error },

    #[snafu(display("failed to spawn process for `{}`: {}", cmd, source))]
    SpawnProcess { cmd: String, source: std::io::Error },

    #[snafu(display("failed to resize pty: {}", source))]
    ResizePty { source: std::io::Error },

    #[snafu(display("failed to read from pty: {}", source))]
    ReadFromPty { source: std::io::Error },

    #[snafu(display("failed to write to pty: {}", source))]
    WriteToPty { source: std::io::Error },

    #[snafu(display("failed to read from terminal: {}", source))]
    ReadFromTerminal { source: std::io::Error },

    #[snafu(display(
        "failed to clear ready state on pty for reading: {}",
        source
    ))]
    PtyClearReadReady { source: std::io::Error },

    #[snafu(display("failed to poll for process exit: {}", source))]
    ProcessExitPoll { source: std::io::Error },

    #[snafu(display(
        "failed to put the terminal into raw mode: {}",
        source
    ))]
    IntoRawMode { source: std::io::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

pub enum Event {
    CommandStart(String, Vec<String>),
    Output(Vec<u8>),
    CommandExit(std::process::ExitStatus),
}

pub struct Process {
    pty: tokio_pty_process::AsyncPtyMaster,
    process: tokio_pty_process::Child,
    // TODO: tokio::io::Stdin is broken
    // input: tokio::io::Stdin,
    input: tokio::reactor::PollEvented2<EventedStdin>,
    input_buf: std::collections::VecDeque<u8>,
    cmd: String,
    args: Vec<String>,
    buf: Vec<u8>,
    started: bool,
    exited: bool,
    manage_screen: bool,
    raw_screen: Option<crossterm::RawScreen>,
}

struct Resizer<'a, T> {
    rows: u16,
    cols: u16,
    pty: &'a T,
}

impl<'a, T: tokio_pty_process::PtyMaster> futures::future::Future
    for Resizer<'a, T>
{
    type Item = ();
    type Error = std::io::Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        self.pty.resize(self.rows, self.cols)
    }
}

impl Process {
    const POLL_FNS: &'static [&'static dyn for<'a> Fn(
        &'a mut Self,
    ) -> Result<
        crate::component_future::Poll<Event>,
    >] = &[
        // order is important here - checking command_exit first so that we
        // don't try to read from a process that has already exited, which
        // causes an error
        &Self::poll_command_start,
        &Self::poll_command_exit,
        &Self::poll_read_stdin,
        &Self::poll_write_stdin,
        &Self::poll_read_stdout,
    ];

    pub fn new(cmd: &str, args: &[String]) -> Result<Self> {
        let pty =
            tokio_pty_process::AsyncPtyMaster::open().context(OpenPty)?;

        let process = std::process::Command::new(cmd)
            .args(args)
            .spawn_pty_async(&pty)
            .context(SpawnProcess { cmd })?;

        let (cols, rows) = crossterm::terminal().terminal_size();
        Resizer {
            rows,
            cols,
            pty: &pty,
        }
        .wait()
        .context(ResizePty)?;

        // TODO: tokio::io::stdin is broken (it's blocking)
        // let input = tokio::io::stdin();
        let input = tokio::reactor::PollEvented2::new(EventedStdin);

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
            manage_screen: true,
            raw_screen: None,
        })
    }

    #[allow(dead_code)]
    pub fn set_raw(mut self, raw: bool) -> Self {
        self.manage_screen = raw;
        self
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
        if self.exited {
            return Ok(crate::component_future::Poll::NothingToDo);
        }

        // XXX this is why i had to do the EventedFd thing - poll_read on its
        // own will block reading from stdin, so i need a way to explicitly
        // check readiness before doing the read
        let ready = mio::Ready::readable();
        match self.input.poll_read_ready(ready) {
            Ok(futures::Async::Ready(_)) => {
                match self.input.poll_read(&mut self.buf) {
                    Ok(futures::Async::Ready(n)) => {
                        if n > 0 {
                            self.input_buf.extend(self.buf[..n].iter());
                        }
                    }
                    Ok(futures::Async::NotReady) => {
                        return Ok(crate::component_future::Poll::NotReady);
                    }
                    Err(e) => return Err(e).context(ReadFromTerminal),
                }
                // XXX i'm pretty sure this is wrong (if the single poll_read
                // call didn't return all waiting data, clearing read ready
                // state means that we won't get the rest until some more data
                // beyond that appears), but i don't know that there's a way
                // to do it correctly given that poll_read blocks
                if let Err(e) = self
                    .input
                    .clear_read_ready(ready)
                    .context(PtyClearReadReady)
                {
                    return Err(e);
                }

                Ok(crate::component_future::Poll::DidWork)
            }
            Ok(futures::Async::NotReady) => {
                Ok(crate::component_future::Poll::NotReady)
            }
            Err(e) => Err(e).context(ReadFromTerminal),
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
        match self.pty.poll_write(buf) {
            Ok(futures::Async::Ready(n)) => {
                for _ in 0..n {
                    self.input_buf.pop_front();
                }
                Ok(crate::component_future::Poll::DidWork)
            }
            Ok(futures::Async::NotReady) => {
                Ok(crate::component_future::Poll::NotReady)
            }
            Err(e) => Err(e).context(WriteToPty),
        }
    }

    fn poll_read_stdout(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        if self.exited {
            return Ok(crate::component_future::Poll::NothingToDo);
        }

        match self.pty.poll_read(&mut self.buf) {
            Ok(futures::Async::Ready(n)) => {
                let bytes = self.buf[..n].to_vec();
                let bytes: Vec<_> = bytes
                    .iter()
                    // replace \n with \r\n
                    .fold(vec![], |mut acc, &c| {
                        if c == b'\n' {
                            acc.push(b'\r');
                            acc.push(b'\n');
                        } else {
                            acc.push(c);
                        }
                        acc
                    });
                Ok(crate::component_future::Poll::Event(Event::Output(bytes)))
            }
            Ok(futures::Async::NotReady) => {
                Ok(crate::component_future::Poll::NotReady)
            }
            Err(e) => Err(e).context(ReadFromPty),
        }
    }

    fn poll_command_exit(
        &mut self,
    ) -> Result<crate::component_future::Poll<Event>> {
        if self.exited {
            return Ok(crate::component_future::Poll::Done);
        }

        match self.process.poll().context(ProcessExitPoll) {
            Ok(futures::Async::Ready(status)) => {
                self.exited = true;
                Ok(crate::component_future::Poll::Event(Event::CommandExit(
                    status,
                )))
            }
            Ok(futures::Async::NotReady) => {
                Ok(crate::component_future::Poll::NotReady)
            }
            Err(e) => Err(e),
        }
    }
}

#[must_use = "streams do nothing unless polled"]
impl futures::stream::Stream for Process {
    type Item = Event;
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        if self.manage_screen && self.raw_screen.is_none() {
            self.raw_screen = Some(
                crossterm::RawScreen::into_raw_mode().context(IntoRawMode)?,
            );
        }

        crate::component_future::poll_stream(self, Self::POLL_FNS)
    }
}

struct EventedStdin;

impl std::io::Read for EventedStdin {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let stdin = std::io::stdin();
        let mut stdin = stdin.lock();
        stdin.read(buf)
    }
}

impl mio::Evented for EventedStdin {
    fn register(
        &self,
        poll: &mio::Poll,
        token: mio::Token,
        interest: mio::Ready,
        opts: mio::PollOpt,
    ) -> std::io::Result<()> {
        let fd = 0 as std::os::unix::io::RawFd;
        let eventedfd = mio::unix::EventedFd(&fd);
        eventedfd.register(poll, token, interest, opts)
    }

    fn reregister(
        &self,
        poll: &mio::Poll,
        token: mio::Token,
        interest: mio::Ready,
        opts: mio::PollOpt,
    ) -> std::io::Result<()> {
        let fd = 0 as std::os::unix::io::RawFd;
        let eventedfd = mio::unix::EventedFd(&fd);
        eventedfd.reregister(poll, token, interest, opts)
    }

    fn deregister(&self, poll: &mio::Poll) -> std::io::Result<()> {
        let fd = 0 as std::os::unix::io::RawFd;
        let eventedfd = mio::unix::EventedFd(&fd);
        eventedfd.deregister(poll)
    }
}
