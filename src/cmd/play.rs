use futures::future::Future as _;
use futures::stream::Stream as _;
use snafu::ResultExt as _;
use std::io::Write as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("{}", source))]
    Common { source: crate::error::Error },

    #[snafu(display("failed to open file: {}", source))]
    OpenFile { source: tokio::io::Error },

    #[snafu(display("failed to sleep until next frame: {}", source))]
    Sleep { source: tokio::timer::Error },

    #[snafu(display("failed to write to stdout: {}", source))]
    WriteTerminal { source: tokio::io::Error },

    #[snafu(display("failed to write to stdout: {}", source))]
    FlushTerminal { source: tokio::io::Error },

    #[snafu(display("failed to read from file: {}", source))]
    ReadFile { source: crate::ttyrec::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    app.about("Play recorded terminal sessions").arg(
        clap::Arg::with_name("filename")
            .long("filename")
            .takes_value(true)
            .required(true),
    )
}

pub fn run<'a>(matches: &clap::ArgMatches<'a>) -> super::Result<()> {
    let filename = matches.value_of("filename").unwrap();
    run_impl(filename).context(super::Play)
}

fn run_impl(filename: &str) -> Result<()> {
    let fut = PlaySession::new(filename);
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
    const POLL_FNS: &'static [&'static dyn for<'a> Fn(
        &'a mut Self,
    ) -> Result<
        crate::component_future::Poll<()>,
    >] = &[
        &Self::poll_open_file,
        &Self::poll_read_file,
        &Self::poll_write_terminal,
    ];

    fn poll_open_file(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        match &mut self.file {
            FileState::Closed { filename } => {
                self.file = FileState::Opening {
                    fut: tokio::fs::File::open(filename.to_string()),
                };
                Ok(crate::component_future::Poll::DidWork)
            }
            FileState::Opening { fut } => {
                match fut.poll().context(OpenFile)? {
                    futures::Async::Ready(file) => {
                        let file = crate::ttyrec::File::new(file);
                        self.file = FileState::Open { file };
                        Ok(crate::component_future::Poll::DidWork)
                    }
                    futures::Async::NotReady => {
                        Ok(crate::component_future::Poll::NotReady)
                    }
                }
            }
            _ => Ok(crate::component_future::Poll::NothingToDo),
        }
    }

    fn poll_read_file(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        if let FileState::Open { file } = &mut self.file {
            match file.poll_read().context(ReadFile)? {
                futures::Async::Ready(Some(frame)) => {
                    self.to_write.insert_at(frame.data, frame.time);
                    Ok(crate::component_future::Poll::DidWork)
                }
                futures::Async::Ready(None) => {
                    self.file = FileState::Eof;
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

    fn poll_write_terminal(
        &mut self,
    ) -> Result<crate::component_future::Poll<()>> {
        match self.to_write.poll().context(Sleep)? {
            futures::Async::Ready(Some(data)) => {
                // TODO async
                let stdout = std::io::stdout();
                let mut stdout = stdout.lock();
                stdout.write(&data).context(WriteTerminal)?;
                stdout.flush().context(FlushTerminal)?;
                Ok(crate::component_future::Poll::DidWork)
            }
            futures::Async::Ready(None) => {
                if let FileState::Eof = self.file {
                    Ok(crate::component_future::Poll::Event(()))
                } else {
                    Ok(crate::component_future::Poll::NothingToDo)
                }
            }
            futures::Async::NotReady => {
                Ok(crate::component_future::Poll::NotReady)
            }
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
