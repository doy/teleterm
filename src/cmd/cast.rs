use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::ResultExt as _;
use tokio::io::AsyncWrite as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to run process: {}", source))]
    Spawn { source: crate::process::Error },

    #[snafu(display("failed to write to stdout: {}", source))]
    WriteTerminal { source: tokio::io::Error },

    #[snafu(display("failed to write to stdout: {}", source))]
    FlushTerminal { source: tokio::io::Error },

    #[snafu(display("process failed: {}", source))]
    Process { source: crate::process::Error },

    #[snafu(display("communication with server failed: {}", source))]
    Client { source: crate::client::Error },

    #[snafu(display("unexpected message: {:?}", message))]
    UnexpectedMessage { message: crate::protocol::Message },
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Stream your terminal")
}

pub fn run<'a>(_matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    run_impl().context(super::Cast)
}

fn run_impl() -> Result<()> {
    tokio::run(
        CastSession::new("zsh", &[], std::time::Duration::from_secs(5))?
            .map_err(|e| {
                eprintln!("{}", e);
            }),
    );

    Ok(())
}

struct CastSession {
    client: crate::client::Client,
    process: crate::process::Process,
    stdout: tokio::io::Stdout,
    buffer: crate::term::Buffer,
    sent_local: usize,
    sent_remote: usize,
    needs_flush: bool,
    done: bool,
}

impl CastSession {
    fn new(
        cmd: &str,
        args: &[String],
        heartbeat_duration: std::time::Duration,
    ) -> Result<Self> {
        let client = crate::client::Client::new(heartbeat_duration);
        let process =
            crate::process::Process::new(cmd, args).context(Spawn)?;
        Ok(Self {
            client,
            process,
            stdout: tokio::io::stdout(),
            buffer: crate::term::Buffer::new(),
            sent_local: 0,
            sent_remote: 0,
            needs_flush: false,
            done: false,
        })
    }

    fn poll_read_client(&mut self) -> Result<bool> {
        match self.client.poll().context(Client)? {
            futures::Async::Ready(Some(e)) => match e {
                crate::client::Event::Reconnect => {
                    self.sent_remote = 0;
                    Ok(true)
                }
                crate::client::Event::ServerMessage(msg) => {
                    Err(Error::UnexpectedMessage { message: msg })
                }
            },
            futures::Async::Ready(None) => {
                if !self.done {
                    unreachable!()
                }
                Ok(true)
            }
            futures::Async::NotReady => Ok(false),
        }
    }

    fn poll_read_process(&mut self) -> Result<bool> {
        match self.process.poll().context(Process)? {
            futures::Async::Ready(Some(e)) => match e {
                crate::process::Event::CommandStart(..) => {}
                crate::process::Event::CommandExit(..) => {
                    self.done = true;
                }
                crate::process::Event::Output(output) => {
                    self.record_bytes(output);
                }
            },
            futures::Async::Ready(None) => {
                if !self.done {
                    unreachable!()
                }
            }
            futures::Async::NotReady => return Ok(false),
        }
        Ok(true)
    }

    fn poll_write_terminal(&mut self) -> Result<bool> {
        if self.sent_local == self.buffer.len() {
            return Ok(false);
        }

        match self
            .stdout
            .poll_write(&self.buffer.contents()[self.sent_local..])
            .context(WriteTerminal)?
        {
            futures::Async::Ready(n) => {
                self.sent_local += n;
                self.needs_flush = true;
                Ok(true)
            }
            futures::Async::NotReady => Ok(false),
        }
    }

    fn poll_flush_terminal(&mut self) -> Result<bool> {
        if !self.needs_flush {
            return Ok(false);
        }

        match self.stdout.poll_flush().context(FlushTerminal)? {
            futures::Async::Ready(()) => {
                self.needs_flush = false;
                Ok(true)
            }
            futures::Async::NotReady => Ok(false),
        }
    }

    fn poll_write_server(&mut self) -> Result<bool> {
        if self.sent_remote == self.buffer.len() {
            return Ok(false);
        }

        let buf = &self.buffer.contents()[self.sent_remote..];
        self.client
            .send_message(crate::protocol::Message::terminal_output(&buf));
        self.sent_remote = self.buffer.len();

        Ok(true)
    }

    fn record_bytes(&mut self, buf: Vec<u8>) {
        if self.buffer.append(buf) {
            self.sent_local = 0;
            self.sent_remote = 0;
        }
    }
}

impl futures::future::Future for CastSession {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        loop {
            let mut did_work = false;

            did_work |= self.poll_read_client()?;
            did_work |= self.poll_read_process()?;
            did_work |= self.poll_write_terminal()?;
            did_work |= self.poll_flush_terminal()?;
            did_work |= self.poll_write_server()?;

            if self.done {
                return Ok(futures::Async::Ready(()));
            }

            if !did_work {
                break;
            }
        }

        Ok(futures::Async::NotReady)
    }
}
