use crate::prelude::*;
use std::io::Write as _;

#[derive(serde::Deserialize, Debug, Default)]
pub struct Config {
    #[serde(default)]
    ttyrec: crate::config::Ttyrec,
}

impl crate::config::Config for Config {
    fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        self.ttyrec.merge_args(matches)
    }

    fn run(&self) -> Result<()> {
        let fut = PlaySession::new(&self.ttyrec.filename);
        tokio::run(fut.map_err(|e| {
            log::error!("{}", e);
        }));
        Ok(())
    }
}

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    crate::config::Ttyrec::cmd(app.about("Play recorded terminal sessions"))
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
        fut: tokio::fs::file::OpenFuture<String>,
    },
    Open {
        file: crate::ttyrec::File,
    },
    Eof,
}

struct PlaySession {
    file: FileState,
    to_write: DumbDelayQueue<Vec<u8>>,
    // to_write: tokio::timer::delay_queue::DelayQueue<Vec<u8>>,
}

impl PlaySession {
    fn new(filename: &str) -> Self {
        Self {
            file: FileState::Closed {
                filename: filename.to_string(),
            },
            to_write: DumbDelayQueue::new(),
            // to_write: tokio::timer::delay_queue::DelayQueue::new(),
        }
    }
}

impl PlaySession {
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> crate::component_future::Poll<
            (),
            Error,
        >] = &[
        &Self::poll_open_file,
        &Self::poll_read_file,
        &Self::poll_write_terminal,
    ];

    fn poll_open_file(&mut self) -> crate::component_future::Poll<(), Error> {
        match &mut self.file {
            FileState::Closed { filename } => {
                self.file = FileState::Opening {
                    filename: filename.to_string(),
                    fut: tokio::fs::File::open(filename.to_string()),
                };
                Ok(crate::component_future::Async::DidWork)
            }
            FileState::Opening { filename, fut } => {
                let file = try_ready!(fut.poll().with_context(|| {
                    crate::error::OpenFile {
                        filename: filename.to_string(),
                    }
                }));
                let file = crate::ttyrec::File::new(file);
                self.file = FileState::Open { file };
                Ok(crate::component_future::Async::DidWork)
            }
            _ => Ok(crate::component_future::Async::NothingToDo),
        }
    }

    fn poll_read_file(&mut self) -> crate::component_future::Poll<(), Error> {
        if let FileState::Open { file } = &mut self.file {
            if let Some(frame) = try_ready!(file.poll_read()) {
                self.to_write.insert_at(frame.data, frame.time);
            } else {
                self.file = FileState::Eof;
            }
            Ok(crate::component_future::Async::DidWork)
        } else {
            Ok(crate::component_future::Async::NothingToDo)
        }
    }

    fn poll_write_terminal(
        &mut self,
    ) -> crate::component_future::Poll<(), Error> {
        if let Some(data) =
            try_ready!(self.to_write.poll().context(crate::error::Sleep))
        {
            // TODO async
            let stdout = std::io::stdout();
            let mut stdout = stdout.lock();
            stdout.write(&data).context(crate::error::WriteTerminal)?;
            stdout.flush().context(crate::error::FlushTerminal)?;
            Ok(crate::component_future::Async::DidWork)
        } else if let FileState::Eof = self.file {
            Ok(crate::component_future::Async::Ready(()))
        } else {
            Ok(crate::component_future::Async::NothingToDo)
        }
    }
}

#[must_use = "futures do nothing unless polled"]
impl futures::future::Future for PlaySession {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        crate::component_future::poll_future(self, Self::POLL_FNS)
    }
}

// XXX tokio's delay_queue implementation seems to have a bug when
// interleaving inserts and polls - if you insert some entries, then poll
// successfully, then insert some more entries, the task won't get notified to
// wake up until the first entry after the successful poll is ready, instead
// of when the next entry of the original set is ready
// NOTE: this implementation is, as its name indicates, pretty dumb - it
// requires the entries to be inserted in order or else it won't work. this is
// fine for reading ttyrecs, but is probably not great for a general purpose
// thing.
struct DumbDelayQueueEntry<T> {
    timer: tokio::timer::Delay,
    data: T,
}

struct DumbDelayQueue<T> {
    queue: std::collections::VecDeque<DumbDelayQueueEntry<T>>,
}

impl<T> DumbDelayQueue<T> {
    fn new() -> Self {
        Self {
            queue: std::collections::VecDeque::new(),
        }
    }

    fn insert_at(&mut self, data: T, time: std::time::Instant) {
        self.queue.push_back(DumbDelayQueueEntry {
            data,
            timer: tokio::timer::Delay::new(time),
        })
    }
}

#[must_use = "streams do nothing unless polled"]
impl<T> futures::stream::Stream for DumbDelayQueue<T> {
    type Item = T;
    type Error = tokio::timer::Error;

    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        if let Some(mut entry) = self.queue.pop_front() {
            match entry.timer.poll() {
                Ok(futures::Async::Ready(_)) => {
                    Ok(futures::Async::Ready(Some(entry.data)))
                }
                Ok(futures::Async::NotReady) => {
                    self.queue.push_front(entry);
                    Ok(futures::Async::NotReady)
                }
                Err(e) => {
                    self.queue.push_front(entry);
                    Err(e)
                }
            }
        } else {
            Ok(futures::Async::Ready(None))
        }
    }
}
