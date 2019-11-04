use crate::prelude::*;
use std::io::Write as _;

#[derive(serde::Deserialize, Debug, Default)]
pub struct Config {
    #[serde(default)]
    ttyrec: crate::config::Ttyrec,

    #[serde(default)]
    play: crate::config::Play,
}

impl crate::config::Config for Config {
    fn merge_args<'a>(
        &mut self,
        matches: &clap::ArgMatches<'a>,
    ) -> Result<()> {
        self.ttyrec.merge_args(matches)?;
        self.play.merge_args(matches)?;
        Ok(())
    }

    fn run(
        &self,
    ) -> Box<dyn futures::future::Future<Item = (), Error = Error> + Send>
    {
        Box::new(PlaySession::new(
            &self.ttyrec.filename,
            self.play.playback_ratio,
            self.play.max_frame_length,
        ))
    }
}

pub fn cmd<'a, 'b>(app: clap::App<'a, 'b>) -> clap::App<'a, 'b> {
    crate::config::Ttyrec::cmd(crate::config::Play::cmd(
        app.about("Play recorded terminal sessions"),
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
        fut: tokio::fs::file::OpenFuture<String>,
    },
    Open {
        reader: ttyrec::Reader<tokio::fs::File>,
    },
    Eof,
}

struct PlaySession {
    file: FileState,
    playback_ratio: f32,
    max_frame_length: Option<std::time::Duration>,

    raw_screen: Option<crossterm::RawScreen>,
    key_reader: crate::key_reader::KeyReader,
    to_write: DumbDelayQueue<Vec<u8>>,
    // to_write: tokio::timer::delay_queue::DelayQueue<Vec<u8>>,
    base_time: std::time::Instant,
    last_frame_time: std::time::Duration,
    total_time_clamped: std::time::Duration,
    paused: Option<std::time::Instant>,
}

impl PlaySession {
    fn new(
        filename: &str,
        playback_ratio: f32,
        max_frame_length: Option<std::time::Duration>,
    ) -> Self {
        Self {
            file: FileState::Closed {
                filename: filename.to_string(),
            },
            playback_ratio,
            max_frame_length,

            raw_screen: None,
            key_reader: crate::key_reader::KeyReader::new(),
            to_write: DumbDelayQueue::new(),
            // to_write: tokio::timer::delay_queue::DelayQueue::new(),
            base_time: std::time::Instant::now(),
            last_frame_time: std::time::Duration::default(),
            total_time_clamped: std::time::Duration::default(),
            paused: None,
        }
    }

    fn keypress(&mut self, e: &crossterm::InputEvent) -> Result<bool> {
        match e {
            crossterm::InputEvent::Keyboard(crossterm::KeyEvent::Char(
                'q',
            )) => return Ok(true),
            crossterm::InputEvent::Keyboard(crossterm::KeyEvent::Char(
                ' ',
            )) => {
                if let Some(time) = self.paused.take() {
                    let diff = std::time::Instant::now() - time;
                    self.to_write.time_incr(diff);
                    self.base_time += diff;
                } else {
                    self.paused = Some(std::time::Instant::now());
                }
            }
            _ => {}
        }
        Ok(false)
    }
}

impl PlaySession {
    const POLL_FNS:
        &'static [&'static dyn for<'a> Fn(
            &'a mut Self,
        )
            -> component_future::Poll<
            (),
            Error,
        >] = &[
        &Self::poll_open_file,
        &Self::poll_read_file,
        &Self::poll_input,
        &Self::poll_write_terminal,
    ];

    fn poll_open_file(&mut self) -> component_future::Poll<(), Error> {
        match &mut self.file {
            FileState::Closed { filename } => {
                self.file = FileState::Opening {
                    filename: filename.to_string(),
                    fut: tokio::fs::File::open(filename.to_string()),
                };
                Ok(component_future::Async::DidWork)
            }
            FileState::Opening { filename, fut } => {
                let file = component_future::try_ready!(fut
                    .poll()
                    .with_context(|| {
                        crate::error::OpenFile {
                            filename: filename.to_string(),
                        }
                    }));
                let reader = ttyrec::Reader::new(file);
                self.file = FileState::Open { reader };
                Ok(component_future::Async::DidWork)
            }
            _ => Ok(component_future::Async::NothingToDo),
        }
    }

    fn poll_read_file(&mut self) -> component_future::Poll<(), Error> {
        if let FileState::Open { reader } = &mut self.file {
            if let Some(frame) = component_future::try_ready!(reader
                .poll_read()
                .context(crate::error::ReadTtyrec))
            {
                let frame_time = frame.time - reader.offset().unwrap();
                let frame_dur = (frame_time - self.last_frame_time)
                    .div_f32(self.playback_ratio);
                self.total_time_clamped += self
                    .max_frame_length
                    .map_or(frame_dur, |max_frame_length| {
                        frame_dur.min(max_frame_length)
                    });

                self.to_write.insert_at(
                    frame.data,
                    self.base_time + self.total_time_clamped,
                );

                self.last_frame_time = frame_time;
            } else {
                self.file = FileState::Eof;
            }
            Ok(component_future::Async::DidWork)
        } else {
            Ok(component_future::Async::NothingToDo)
        }
    }

    fn poll_input(&mut self) -> component_future::Poll<(), Error> {
        if self.raw_screen.is_none() {
            self.raw_screen = Some(
                crossterm::RawScreen::into_raw_mode()
                    .context(crate::error::ToRawMode)?,
            );
        }

        let e = component_future::try_ready!(self.key_reader.poll()).unwrap();
        let quit = self.keypress(&e)?;
        if quit {
            self.raw_screen = None;

            // TODO async
            let stdout = std::io::stdout();
            let mut stdout = stdout.lock();
            stdout
                .write(b"\x1bc")
                .context(crate::error::WriteTerminal)?;
            stdout.flush().context(crate::error::FlushTerminal)?;
            Ok(component_future::Async::Ready(()))
        } else {
            Ok(component_future::Async::DidWork)
        }
    }

    fn poll_write_terminal(&mut self) -> component_future::Poll<(), Error> {
        if self.paused.is_some() {
            return Ok(component_future::Async::NothingToDo);
        }

        if let Some(data) = component_future::try_ready!(self
            .to_write
            .poll()
            .context(crate::error::Sleep))
        {
            // TODO async
            let stdout = std::io::stdout();
            let mut stdout = stdout.lock();
            stdout.write(&data).context(crate::error::WriteTerminal)?;
            stdout.flush().context(crate::error::FlushTerminal)?;
            Ok(component_future::Async::DidWork)
        } else if let FileState::Eof = self.file {
            Ok(component_future::Async::Ready(()))
        } else {
            Ok(component_future::Async::NothingToDo)
        }
    }
}

#[must_use = "futures do nothing unless polled"]
impl futures::future::Future for PlaySession {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        component_future::poll_future(self, Self::POLL_FNS)
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

    fn time_incr(&mut self, dur: std::time::Duration) {
        for entry in &mut self.queue {
            entry.timer.reset(entry.timer.deadline() + dur);
        }
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
