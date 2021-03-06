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

    fn run(
        &self,
    ) -> Box<dyn futures::Future<Item = (), Error = Error> + Send> {
        Box::new(RecordSession::new(
            &self.ttyrec.filename,
            &self.command.command,
            &self.command.args,
        ))
    }
}

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    crate::config::Command::cmd(crate::config::Ttyrec::cmd(
        app.about("Record a terminal session to a file"),
    ))
}

pub fn config(
    config: Option<config::Config>,
) -> Result<Box<dyn crate::config::Config>> {
    let config: Config = if let Some(config) = config {
        config
            .try_into()
            .context(crate::error::CouldntParseConfig)?
    } else {
        Config::default()
    };
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
        writer: ttyrec::Writer<tokio::fs::File>,
    },
}

struct RecordSession {
    file: FileState,
    frame_data: Vec<u8>,

    process:
        tokio_pty_process_stream::ResizingProcess<crate::async_stdin::Stdin>,
    raw_screen: Option<crossterm::screen::RawScreen>,
    done: bool,

    stdout: tokio::io::Stdout,
    to_write_stdout: std::collections::VecDeque<u8>,
    needs_flush: bool,
}

impl RecordSession {
    fn new(filename: &str, cmd: &str, args: &[String]) -> Self {
        let input = crate::async_stdin::Stdin::new();
        let process = tokio_pty_process_stream::ResizingProcess::new(
            tokio_pty_process_stream::Process::new(cmd, args, input),
        );

        Self {
            file: FileState::Closed {
                filename: filename.to_string(),
            },
            frame_data: vec![],

            process,
            raw_screen: None,
            done: false,

            stdout: tokio::io::stdout(),
            to_write_stdout: std::collections::VecDeque::new(),
            needs_flush: false,
        }
    }

    fn record_bytes(&mut self, buf: &[u8]) {
        self.frame_data.extend(buf);
        self.to_write_stdout.extend(buf);
    }
}

impl RecordSession {
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> component_future::Poll<
            (),
            Error,
        >] = &[
        &Self::poll_open_file,
        &Self::poll_read_process,
        &Self::poll_write_terminal,
        &Self::poll_flush_terminal,
        &Self::poll_write_file,
    ];

    fn poll_open_file(&mut self) -> component_future::Poll<(), Error> {
        match &mut self.file {
            FileState::Closed { filename } => {
                self.file = FileState::Opening {
                    filename: filename.to_string(),
                    fut: tokio::fs::File::create(filename.to_string()),
                };
                Ok(component_future::Async::DidWork)
            }
            FileState::Opening { filename, fut } => {
                let file = component_future::try_ready!(fut
                    .poll()
                    .with_context(|| {
                        crate::error::OpenFile {
                            filename: filename.clone(),
                        }
                    }));
                self.file = FileState::Open {
                    writer: ttyrec::Writer::new(file),
                };
                Ok(component_future::Async::DidWork)
            }
            FileState::Open { .. } => {
                Ok(component_future::Async::NothingToDo)
            }
        }
    }

    fn poll_read_process(&mut self) -> component_future::Poll<(), Error> {
        match component_future::try_ready!(self
            .process
            .poll()
            .context(crate::error::Subprocess))
        {
            Some(tokio_pty_process_stream::Event::CommandStart {
                ..
            }) => {
                if self.raw_screen.is_none() {
                    self.raw_screen = Some(
                        crossterm::screen::RawScreen::into_raw_mode()
                            .context(crate::error::ToRawMode)?,
                    );
                }
            }
            Some(tokio_pty_process_stream::Event::CommandExit { .. }) => {
                self.done = true;
            }
            Some(tokio_pty_process_stream::Event::Output { data }) => {
                self.record_bytes(&data);
            }
            Some(tokio_pty_process_stream::Event::Resize { .. }) => {}
            None => {
                if !self.done {
                    unreachable!()
                }
                // don't return final event here - wait until we are done
                // writing all data to the file (see poll_write_file)
            }
        }
        Ok(component_future::Async::DidWork)
    }

    fn poll_write_terminal(&mut self) -> component_future::Poll<(), Error> {
        if self.to_write_stdout.is_empty() {
            return Ok(component_future::Async::NothingToDo);
        }

        let (a, b) = self.to_write_stdout.as_slices();
        let buf = if a.is_empty() { b } else { a };
        let n = component_future::try_ready!(self
            .stdout
            .poll_write(buf)
            .context(crate::error::WriteTerminal));
        for _ in 0..n {
            self.to_write_stdout.pop_front();
        }
        self.needs_flush = true;
        Ok(component_future::Async::DidWork)
    }

    fn poll_flush_terminal(&mut self) -> component_future::Poll<(), Error> {
        if !self.needs_flush {
            return Ok(component_future::Async::NothingToDo);
        }

        component_future::try_ready!(self
            .stdout
            .poll_flush()
            .context(crate::error::FlushTerminal));
        self.needs_flush = false;
        Ok(component_future::Async::DidWork)
    }

    fn poll_write_file(&mut self) -> component_future::Poll<(), Error> {
        let writer = match &mut self.file {
            FileState::Open { writer } => writer,
            _ => {
                return Ok(component_future::Async::NothingToDo);
            }
        };

        if !self.frame_data.is_empty() {
            writer
                .frame(&self.frame_data)
                .context(crate::error::WriteTtyrec)?;
            self.frame_data.clear();
        }

        if writer.needs_write() {
            component_future::try_ready!(writer
                .poll_write()
                .context(crate::error::WriteTtyrec));
            Ok(component_future::Async::DidWork)
        } else {
            // finish writing to the file before actually ending
            if self.done {
                Ok(component_future::Async::Ready(()))
            } else {
                Ok(component_future::Async::NothingToDo)
            }
        }
    }
}

#[must_use = "futures do nothing unless polled"]
impl futures::Future for RecordSession {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        component_future::poll_future(self, Self::POLL_FNS)
    }
}
