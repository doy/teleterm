use crate::prelude::*;
use tokio::io::AsyncWrite as _;

#[derive(serde::Deserialize, Debug, Default)]
pub struct Config {
    #[serde(default)]
    command: crate::config::Command,

    #[serde(default)]
    ttyrec: crate::config::Ttyrec,
}

impl crate::config::Config for Config {
    fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        self.command.merge_args(matches)?;
        self.ttyrec.merge_args(matches)?;
        Ok(())
    }

    fn run(&self) -> Result<()> {
        let fut = RecordSession::new(
            &self.ttyrec.filename,
            self.command.buffer_size,
            &self.command.command,
            &self.command.args,
        );
        tokio::run(fut.map_err(|e| {
            eprintln!("{}", e);
        }));
        Ok(())
    }
}

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    crate::config::Command::cmd(crate::config::Ttyrec::cmd(
        app.about("Record a terminal session to a file"),
    ))
}

pub fn config(
    config: config::Config,
) -> Result<Box<dyn crate::config::Config>> {
    let config: Config = config
        .try_into()
        .context(crate::error::CouldntParseConfig)?;
    Ok(Box::new(config))
}

#[allow(clippy::large_enum_variant)]
enum FileState {
    Closed {
        filename: String,
    },
    Opening {
        filename: String,
        fut: tokio::fs::file::CreateFuture<String>,
    },
    Open {
        file: crate::ttyrec::File,
    },
}

struct RecordSession {
    file: FileState,
    process: crate::resize::ResizingProcess<crate::async_stdin::Stdin>,
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
        let process = crate::resize::ResizingProcess::new(
            crate::process::Process::new(cmd, args, input),
        );

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
        self.sent_local -= self.buffer.append_client(buf, self.sent_local);
    }
}

impl RecordSession {
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> crate::component_future::Poll<
            (),
            Error,
        >] = &[
        &Self::poll_open_file,
        &Self::poll_read_process,
        &Self::poll_write_terminal,
        &Self::poll_flush_terminal,
        &Self::poll_write_file,
    ];

    fn poll_open_file(&mut self) -> crate::component_future::Poll<(), Error> {
        match &mut self.file {
            FileState::Closed { filename } => {
                self.file = FileState::Opening {
                    filename: filename.to_string(),
                    fut: tokio::fs::File::create(filename.to_string()),
                };
                Ok(crate::component_future::Async::DidWork)
            }
            FileState::Opening { filename, fut } => {
                let file = try_ready!(fut.poll().with_context(|| {
                    crate::error::OpenFile {
                        filename: filename.clone(),
                    }
                }));
                let mut file = crate::ttyrec::File::new(file);
                file.write_frame(self.buffer.contents())?;
                self.file = FileState::Open { file };
                Ok(crate::component_future::Async::DidWork)
            }
            FileState::Open { .. } => {
                Ok(crate::component_future::Async::NothingToDo)
            }
        }
    }

    fn poll_read_process(
        &mut self,
    ) -> crate::component_future::Poll<(), Error> {
        match try_ready!(self.process.poll()) {
            Some(crate::resize::Event::Process(e)) => {
                match e {
                    crate::process::Event::CommandStart(..) => {
                        if self.raw_screen.is_none() {
                            self.raw_screen = Some(
                                crossterm::RawScreen::into_raw_mode()
                                    .context(crate::error::ToRawMode)?,
                            );
                        }
                    }
                    crate::process::Event::CommandExit(..) => {
                        self.done = true;
                    }
                    crate::process::Event::Output(output) => {
                        self.record_bytes(&output);
                        if let FileState::Open { file } = &mut self.file {
                            file.write_frame(&output)?;
                        }
                    }
                }
                Ok(crate::component_future::Async::DidWork)
            }
            Some(crate::resize::Event::Resize(_)) => {
                Ok(crate::component_future::Async::DidWork)
            }
            None => {
                if !self.done {
                    unreachable!()
                }
                // don't return final event here - wait until we are done
                // writing all data to the file (see poll_write_file)
                Ok(crate::component_future::Async::DidWork)
            }
        }
    }

    fn poll_write_terminal(
        &mut self,
    ) -> crate::component_future::Poll<(), Error> {
        if self.sent_local == self.buffer.len() {
            return Ok(crate::component_future::Async::NothingToDo);
        }

        let n = try_ready!(self
            .stdout
            .poll_write(&self.buffer.contents()[self.sent_local..])
            .context(crate::error::WriteTerminal));
        self.sent_local += n;
        self.needs_flush = true;
        Ok(crate::component_future::Async::DidWork)
    }

    fn poll_flush_terminal(
        &mut self,
    ) -> crate::component_future::Poll<(), Error> {
        if !self.needs_flush {
            return Ok(crate::component_future::Async::NothingToDo);
        }

        try_ready!(self
            .stdout
            .poll_flush()
            .context(crate::error::FlushTerminal));
        self.needs_flush = false;
        Ok(crate::component_future::Async::DidWork)
    }

    fn poll_write_file(
        &mut self,
    ) -> crate::component_future::Poll<(), Error> {
        let file = match &mut self.file {
            FileState::Open { file } => file,
            _ => {
                return Ok(crate::component_future::Async::NothingToDo);
            }
        };

        match file.poll_write()? {
            futures::Async::Ready(()) => {
                Ok(crate::component_future::Async::DidWork)
            }
            futures::Async::NotReady => {
                // ship all data to the server before actually ending
                if self.done && file.is_empty() {
                    Ok(crate::component_future::Async::Ready(()))
                } else {
                    Ok(crate::component_future::Async::NotReady)
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
        crate::component_future::poll_future(self, Self::POLL_FNS)
    }
}
