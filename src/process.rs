use futures::future::Future as _;
use snafu::ResultExt as _;
use std::io::{Read as _, Write as _};
use tokio::io::AsyncRead as _;
use tokio_pty_process::CommandExt as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to open a pty: {}", source))]
    OpenPty { source: std::io::Error },

    #[snafu(display("failed to spawn process for `{}`: {}", cmd, source))]
    SpawnProcess { cmd: String, source: std::io::Error },

    #[snafu(display("failed to resize pty: {}", source))]
    ResizePty { source: std::io::Error },

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

pub enum CommandEvent {
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
    cmd: String,
    args: Vec<String>,
    buf: Vec<u8>,
    started: bool,
    output_done: bool,
    exit_done: bool,
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
            cmd: cmd.to_string(),
            args: args.to_vec(),
            buf: Vec::with_capacity(4096),
            started: false,
            output_done: false,
            exit_done: false,
            manage_screen: true,
            raw_screen: None,
        })
    }

    #[allow(dead_code)]
    pub fn set_raw(mut self, raw: bool) -> Self {
        self.manage_screen = raw;
        self
    }
}

#[must_use = "streams do nothing unless polled"]
impl futures::stream::Stream for Process {
    type Item = CommandEvent;
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        if self.manage_screen && self.raw_screen.is_none() {
            self.raw_screen = Some(
                crossterm::RawScreen::into_raw_mode().context(IntoRawMode)?,
            );
        }

        if !self.started {
            self.started = true;
            return Ok(futures::Async::Ready(Some(
                CommandEvent::CommandStart(
                    self.cmd.clone(),
                    self.args.clone(),
                ),
            )));
        }

        let ready = mio::Ready::readable();
        let input_poll = self.input.poll_read_ready(ready);
        if let Ok(futures::Async::Ready(_)) = input_poll {
            let stdin = std::io::stdin();
            let mut stdin = stdin.lock();
            let mut buf = vec![0; 4096];
            // TODO: async
            let n = stdin.read(&mut buf).context(ReadFromTerminal)?;
            if n > 0 {
                let bytes = buf[..n].to_vec();

                // TODO: async
                self.pty.write_all(&bytes).context(WriteToPty)?;
            }
        }
        // TODO: this could lose pending bytes if there is stuff to read in
        // the buffer but we don't read it all in the previous read call,
        // since i think we won't get another notification until new bytes
        // actually arrive even if there are bytes in the buffer
        self.input
            .clear_read_ready(ready)
            .context(PtyClearReadReady)?;

        if !self.output_done {
            self.buf.clear();
            let output_poll = self.pty.read_buf(&mut self.buf);
            match output_poll {
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
                    return Ok(futures::Async::Ready(Some(
                        CommandEvent::Output(bytes),
                    )));
                }
                Ok(futures::Async::NotReady) => {
                    return Ok(futures::Async::NotReady);
                }
                Err(_) => {
                    // explicitly ignoring errors (for now?) because we
                    // always read off the end of the pty after the process
                    // is done
                    self.output_done = true;
                }
            }
        }

        if !self.exit_done {
            let exit_poll = self.process.poll().context(ProcessExitPoll);
            match exit_poll {
                Ok(futures::Async::Ready(status)) => {
                    self.exit_done = true;
                    return Ok(futures::Async::Ready(Some(
                        CommandEvent::CommandExit(status),
                    )));
                }
                Ok(futures::Async::NotReady) => {
                    return Ok(futures::Async::NotReady);
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }

        Ok(futures::Async::Ready(None))
    }
}

struct EventedStdin;

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
