use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::ResultExt as _;
use tokio::io::AsyncWrite as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("{}", source))]
    Common { source: crate::error::Error },

    #[snafu(display("process failed: {}", source))]
    Process { source: crate::process::Error },

    #[snafu(display("failed to write to stdout: {}", source))]
    WriteTerminal { source: tokio::io::Error },

    #[snafu(display("failed to write to stdout: {}", source))]
    FlushTerminal { source: tokio::io::Error },

    #[snafu(display("failed to open file: {}", source))]
    OpenFile { source: tokio::io::Error },

    #[snafu(display("failed to write to file: {}", source))]
    WriteFile { source: crate::ttyrec::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Record a terminal session to a file")
        .arg(
            clap::Arg::with_name("filename")
                .long("filename")
                .takes_value(true)
                .required(true),
        )
        .arg(
            clap::Arg::with_name("buffer-size")
                .long("buffer-size")
                .takes_value(true),
        )
        .arg(clap::Arg::with_name("command").index(1))
        .arg(clap::Arg::with_name("args").index(2).multiple(true))
}

pub fn run<'a>(matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    let filename = matches.value_of("filename").unwrap();
    let buffer_size =
        matches
            .value_of("buffer-size")
            .map_or(Ok(4 * 1024 * 1024), |s| {
                s.parse()
                    .context(crate::error::ParseBufferSize { input: s })
                    .context(Common)
                    .context(super::Record)
            })?;
    let command = matches.value_of("command").map_or_else(
        || std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()),
        std::string::ToString::to_string,
    );
    let args = if let Some(args) = matches.values_of("args") {
        args.map(std::string::ToString::to_string).collect()
    } else {
        vec![]
    };
    run_impl(filename, buffer_size, &command, &args).context(super::Record)
}

fn run_impl(
    filename: &str,
    buffer_size: usize,
    command: &str,
    args: &[String],
) -> Result<()> {
    let fut = RecordSession::new(filename, buffer_size, command, args);
    tokio::run(fut.map_err(|e| {
        eprintln!("{}", e);
    }));
    Ok(())
}

#[allow(clippy::large_enum_variant)]
enum FileState {
    Closed {
        filename: String,
    },
    Opening {
        fut: tokio::fs::file::CreateFuture<String>,
    },
    Open {
        file: crate::ttyrec::File,
    },
}

struct RecordSession {
    file: FileState,
    process: crate::process::Process<crate::async_stdin::Stdin>,
    stdout: tokio::io::Stdout,
    buffer: crate::term::Buffer,
    sent_local: usize,
    needs_flush: bool,
    done: bool,
    raw_screen: Option<crossterm::RawScreen>,
}

impl RecordSession {
    fn new(
        filename: &str,
        buffer_size: usize,
        cmd: &str,
        args: &[String],
    ) -> Self {
        let input = crate::async_stdin::Stdin::new();
        let process = crate::process::Process::new(cmd, args, input);

        Self {
            file: FileState::Closed {
                filename: filename.to_string(),
            },
            process,
            stdout: tokio::io::stdout(),
            buffer: crate::term::Buffer::new(buffer_size),
            sent_local: 0,
            needs_flush: false,
            done: false,
            raw_screen: None,
        }
    }

    fn record_bytes(&mut self, buf: &[u8]) {
        let truncated = self.buffer.append(buf);
        if truncated > self.sent_local {
            self.sent_local = 0;
        } else {
            self.sent_local -= truncated;
        }
    }
}

impl RecordSession {
    const POLL_FNS: &'static [&'static dyn for<'a> Fn(
        &'a mut Self,
    ) -> Result<
        crate::component_future::Poll<()>,
    >] = &[
        &Self::poll_open_file,
        &Self::poll_read_process,
        &Self::poll_write_terminal,
        &Self::poll_flush_terminal,
        &Self::poll_write_file,
    ];

    fn poll_open_file(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        match &mut self.file {
            FileState::Closed { filename } => {
                self.file = FileState::Opening {
                    fut: tokio::fs::File::create(filename.to_string()),
                };
                Ok(crate::component_future::Poll::DidWork)
            }
            FileState::Opening { fut } => {
                match fut.poll().context(OpenFile)? {
                    futures::Async::Ready(file) => {
                        let mut file = crate::ttyrec::File::new(file);
                        file.write_frame(self.buffer.contents())
                            .context(WriteFile)?;
                        self.file = FileState::Open { file };
                        Ok(crate::component_future::Poll::DidWork)
                    }
                    futures::Async::NotReady => {
                        Ok(crate::component_future::Poll::NotReady)
                    }
                }
            }
            FileState::Open { .. } => {
                Ok(crate::component_future::Poll::NothingToDo)
            }
        }
    }

    fn poll_read_process(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        match self.process.poll().context(Process)? {
            futures::Async::Ready(Some(e)) => {
                match e {
                    crate::process::Event::CommandStart(..) => {}
                    crate::process::Event::CommandExit(..) => {
                        self.done = true;
                    }
                    crate::process::Event::Output(output) => {
                        self.record_bytes(&output);
                        if let FileState::Open { file } = &mut self.file {
                            file.write_frame(&output).context(WriteFile)?;
                        }
                    }
                }
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::Ready(None) => {
                if !self.done {
                    unreachable!()
                }
                // don't return final event here - wait until we are done
                // writing all data to the file (see poll_write_file)
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }

    fn poll_write_terminal(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        if self.sent_local == self.buffer.len() {
            return Ok(crate::component_future::Poll::NothingToDo);
        }

        match self
            .stdout
            .poll_write(&self.buffer.contents()[self.sent_local..])
            .context(WriteTerminal)?
        {
            futures::Async::Ready(n) => {
                self.sent_local += n;
                self.needs_flush = true;
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }

    fn poll_flush_terminal(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        if !self.needs_flush {
            return Ok(crate::component_future::Poll::NothingToDo);
        }

        match self.stdout.poll_flush().context(FlushTerminal)? {
            futures::Async::Ready(()) => {
                self.needs_flush = false;
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
        }
    }

    fn poll_write_file(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        let file = match &mut self.file {
            FileState::Open { file } => file,
            _ => {
                return Ok(crate::component_future::Poll::NothingToDo);
            }
        };

        match file.poll_write().context(WriteFile)? {
            futures::Async::Ready(()) => {
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::NotReady => {
                // ship all data to the server before actually ending
                if self.done && file.is_empty() {
                    Ok(crate::component_future::Poll::Event(()))
                } else {
                    Ok(crate::component_future::Poll::NotReady)
                }
            }
        }
    }
}

#[must_use = "futures do nothing unless polled"]
impl futures::future::Future for RecordSession {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        if self.raw_screen.is_none() {
            self.raw_screen = Some(
                crossterm::RawScreen::into_raw_mode()
                    .context(crate::error::IntoRawMode)
                    .context(Common)?,
            );
        }
        crate::component_future::poll_future(self, Self::POLL_FNS)
    }
}
