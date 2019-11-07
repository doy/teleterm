use crate::prelude::*;
use std::io::Write as _;

const PLAYBACK_RATIO_INCR: f32 = 1.5;

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
            self.play.play_at_start,
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

struct Frame {
    dur: std::time::Duration,
    full: Vec<u8>,
    diff: Vec<u8>,
}

impl Frame {
    fn adjusted_dur(
        &self,
        scale: f32,
        clamp: Option<std::time::Duration>,
    ) -> std::time::Duration {
        let scaled = self.dur.div_f32(scale);
        clamp.map_or(scaled, |clamp| scaled.min(clamp))
    }
}

#[derive(Default)]
struct Ttyrec {
    frames: Vec<Frame>,
}

impl Ttyrec {
    fn new() -> Self {
        Self::default()
    }

    fn add_frame(&mut self, frame: Frame) {
        self.frames.push(frame);
    }

    fn frame(&self, idx: usize) -> Option<&Frame> {
        self.frames.get(idx)
    }

    fn frames(
        &self,
    ) -> impl DoubleEndedIterator<Item = &Frame> + ExactSizeIterator<Item = &Frame>
    {
        self.frames.iter()
    }

    fn matches_from<'a>(
        &'a self,
        idx: usize,
        re: &'a regex::bytes::Regex,
    ) -> impl Iterator<Item = (usize, &Frame)> + 'a {
        self.frames()
            .enumerate()
            .skip(idx)
            .filter(move |(_, frame)| re.is_match(&frame.full))
    }

    fn rmatches_from<'a>(
        &'a self,
        idx: usize,
        re: &'a regex::bytes::Regex,
    ) -> impl Iterator<Item = (usize, &Frame)> + 'a {
        self.frames()
            .enumerate()
            .rev()
            .skip(self.frames.len() - idx)
            .filter(move |(_, frame)| re.is_match(&frame.full))
    }

    fn count_matches_from(
        &self,
        idx: usize,
        re: &regex::bytes::Regex,
    ) -> usize {
        self.matches_from(idx, re).count()
    }

    fn len(&self) -> usize {
        self.frames.len()
    }
}

struct SearchState {
    query: regex::bytes::Regex,
    count: usize,
    idx: Option<usize>,
}

struct Player {
    playback_ratio: f32,
    max_frame_length: Option<std::time::Duration>,
    ttyrec: Ttyrec,
    idx: usize,
    timer: Option<tokio::timer::Delay>,
    base_time: std::time::Instant,
    played_amount: std::time::Duration,
    paused: Option<std::time::Instant>,
    search_state: Option<SearchState>,
}

impl Player {
    fn new(
        play_at_start: bool,
        playback_ratio: f32,
        max_frame_length: Option<std::time::Duration>,
    ) -> Self {
        let now = std::time::Instant::now();
        Self {
            playback_ratio,
            max_frame_length,
            ttyrec: Ttyrec::new(),
            idx: 0,
            timer: None,
            base_time: now,
            played_amount: std::time::Duration::default(),
            paused: if play_at_start { None } else { Some(now) },
            search_state: None,
        }
    }

    fn current_frame_idx(&self) -> usize {
        self.idx
    }

    fn current_frame(&self) -> Option<&Frame> {
        self.ttyrec.frame(self.idx)
    }

    fn num_frames(&self) -> usize {
        self.ttyrec.len()
    }

    fn base_time_incr(&mut self, incr: std::time::Duration) {
        self.base_time += incr;
        self.set_timer();
    }

    fn add_frame(&mut self, frame: Frame) {
        self.ttyrec.add_frame(frame);
        if self.timer.is_none() {
            self.set_timer();
        }
    }

    fn playback_ratio_incr(&mut self) {
        self.playback_ratio *= PLAYBACK_RATIO_INCR;
        self.set_timer();
    }

    fn playback_ratio_decr(&mut self) {
        self.playback_ratio /= PLAYBACK_RATIO_INCR;
        self.set_timer();
    }

    fn playback_ratio_reset(&mut self) {
        self.playback_ratio = 1.0;
        self.set_timer();
    }

    fn back(&mut self) {
        self.idx = self.idx.saturating_sub(1);
        self.recalculate_times();
        self.set_timer();
        self.clear_match_idx();
    }

    fn forward(&mut self) {
        self.idx = self.idx.saturating_add(1);
        if self.idx > self.ttyrec.len() - 1 {
            self.idx = self.ttyrec.len() - 1;
        }
        self.recalculate_times();
        self.set_timer();
        self.clear_match_idx();
    }

    fn first(&mut self) {
        self.idx = 0;
        self.recalculate_times();
        self.set_timer();
        self.clear_match_idx();
    }

    fn last(&mut self) {
        self.idx = self.ttyrec.len() - 1;
        self.recalculate_times();
        self.set_timer();
        self.clear_match_idx();
    }

    fn next_match(&mut self) {
        let idx = if let Some(state) = &self.search_state {
            self.ttyrec
                .matches_from(self.idx + 1, &state.query)
                .next()
                .map(|(idx, _)| idx)
        } else {
            return;
        };
        let idx = if let Some(idx) = idx {
            idx
        } else {
            return;
        };

        self.idx = idx;
        self.recalculate_times();
        self.set_timer();

        if let Some(state) = &mut self.search_state {
            if let Some(idx) = &mut state.idx {
                state.idx = Some(*idx + 1);
            } else {
                state.count = self.ttyrec.count_matches_from(0, &state.query);
                state.idx = Some(
                    state.count
                        - self
                            .ttyrec
                            .count_matches_from(self.idx, &state.query),
                );
            }
        }
    }

    fn prev_match(&mut self) {
        let idx = if let Some(state) = &self.search_state {
            self.ttyrec
                .rmatches_from(self.idx, &state.query)
                .next()
                .map(|(idx, _)| idx)
        } else {
            return;
        };
        let idx = if let Some(idx) = idx {
            idx
        } else {
            return;
        };

        self.idx = idx;
        self.recalculate_times();
        self.set_timer();

        if let Some(state) = &mut self.search_state {
            if let Some(idx) = &mut state.idx {
                state.idx = Some(*idx - 1);
            } else {
                state.count = self.ttyrec.count_matches_from(0, &state.query);
                state.idx = Some(
                    state.count
                        - self
                            .ttyrec
                            .count_matches_from(self.idx, &state.query),
                );
            }
        }
    }

    fn toggle_pause(&mut self) {
        let now = std::time::Instant::now();
        if let Some(time) = self.paused.take() {
            self.base_time_incr(now - time);
        } else {
            self.paused = Some(now);
        }
    }

    fn paused(&self) -> bool {
        self.paused.is_some()
    }

    fn recalculate_times(&mut self) {
        let now = std::time::Instant::now();
        self.played_amount = self
            .ttyrec
            .frames
            .iter()
            .map(|f| f.dur)
            .take(self.idx)
            .sum();
        self.base_time = now - self.played_amount;
        if let Some(paused) = &mut self.paused {
            *paused = now;
        }
    }

    fn set_timer(&mut self) {
        if let Some(frame) = self.ttyrec.frame(self.idx) {
            self.timer = Some(tokio::timer::Delay::new(
                self.base_time
                    + self.played_amount
                    + frame.adjusted_dur(
                        self.playback_ratio,
                        self.max_frame_length,
                    ),
            ));
        } else {
            self.timer = None;
        }
    }

    fn set_search_query(&mut self, re: regex::bytes::Regex) {
        let count = self.ttyrec.count_matches_from(0, &re);
        self.search_state = Some(SearchState {
            query: re,
            count,
            idx: None,
        });
        self.next_match();
    }

    fn clear_match_idx(&mut self) {
        if let Some(SearchState { idx, .. }) = &mut self.search_state {
            *idx = None;
        }
    }

    fn poll(&mut self) -> futures::Poll<Option<Vec<u8>>, Error> {
        let frame = if let Some(frame) = self.ttyrec.frame(self.idx) {
            frame
        } else {
            return Ok(futures::Async::Ready(None));
        };
        let timer = if let Some(timer) = &mut self.timer {
            timer
        } else {
            return Ok(futures::Async::Ready(None));
        };

        futures::try_ready!(timer.poll().context(crate::error::Sleep));
        let ret = frame.diff.clone();

        self.idx += 1;
        self.played_amount +=
            frame.adjusted_dur(self.playback_ratio, self.max_frame_length);
        self.set_timer();
        self.clear_match_idx();

        Ok(futures::Async::Ready(Some(ret)))
    }
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
        parser: vt100::Parser,
    },
    Eof,
}

enum InputState {
    Normal,
    Search { query: String },
}

struct PlaySession {
    file: FileState,
    player: Player,
    raw_screen: Option<crossterm::screen::RawScreen>,
    alternate_screen: Option<crossterm::screen::AlternateScreen>,
    key_reader: crate::key_reader::KeyReader,
    last_frame_time: std::time::Duration,
    last_frame_screen: Option<vt100::Screen>,
    input_state: InputState,
}

impl PlaySession {
    fn new(
        filename: &str,
        play_at_start: bool,
        playback_ratio: f32,
        max_frame_length: Option<std::time::Duration>,
    ) -> Self {
        Self {
            file: FileState::Closed {
                filename: filename.to_string(),
            },
            player: Player::new(
                play_at_start,
                playback_ratio,
                max_frame_length,
            ),
            raw_screen: None,
            alternate_screen: None,
            key_reader: crate::key_reader::KeyReader::new(),
            last_frame_time: std::time::Duration::default(),
            last_frame_screen: None,
            input_state: InputState::Normal,
        }
    }

    fn normal_keypress(
        &mut self,
        e: &crossterm::input::InputEvent,
    ) -> Result<bool> {
        match e {
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('q'),
            ) => return Ok(true),
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char(' '),
            ) => {
                self.player.toggle_pause();
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('+'),
            ) => {
                self.player.playback_ratio_incr();
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('-'),
            ) => {
                self.player.playback_ratio_decr();
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('='),
            ) => {
                self.player.playback_ratio_reset();
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('<'),
            ) => {
                self.player.back();
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('>'),
            ) => {
                self.player.forward();
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('0'),
            ) => {
                self.player.first();
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('$'),
            ) => {
                self.player.last();
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('/'),
            ) => {
                self.input_state = InputState::Search {
                    query: String::new(),
                };
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('n'),
            ) => {
                self.player.next_match();
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char('p'),
            ) => {
                self.player.prev_match();
            }
            _ => {}
        }
        Ok(false)
    }

    fn search_keypress(
        &mut self,
        e: &crossterm::input::InputEvent,
    ) -> Result<bool> {
        match e {
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Esc,
            ) => {
                self.input_state = InputState::Normal;
            }
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Char(c),
            ) => match &mut self.input_state {
                InputState::Search { query } => {
                    query.push(*c);
                }
                _ => unreachable!(),
            },
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Backspace,
            ) => match &mut self.input_state {
                InputState::Search { query } => {
                    query.pop();
                }
                _ => unreachable!(),
            },
            crossterm::input::InputEvent::Keyboard(
                crossterm::input::KeyEvent::Enter,
            ) => {
                let query =
                    if let InputState::Search { query } = &self.input_state {
                        query.to_string()
                    } else {
                        unreachable!()
                    };
                if let Ok(re) = regex::bytes::Regex::new(&query) {
                    self.input_state = InputState::Normal;
                    self.player.set_search_query(re);
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn keypress(&mut self, e: &crossterm::input::InputEvent) -> Result<bool> {
        match &mut self.input_state {
            InputState::Normal => self.normal_keypress(e),
            InputState::Search { .. } => self.search_keypress(e),
        }
    }

    fn redraw(&self) -> Result<()> {
        let frame = if let Some(frame) = self.player.current_frame() {
            frame
        } else {
            return Ok(());
        };
        self.write(&frame.full)?;
        self.draw_ui()?;
        Ok(())
    }

    fn write(&self, data: &[u8]) -> Result<()> {
        // TODO async
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        stdout.write(data).context(crate::error::WriteTerminal)?;
        stdout.flush().context(crate::error::FlushTerminal)?;
        Ok(())
    }

    fn draw_ui(&self) -> Result<()> {
        let size = crate::term::Size::get()?;

        if self.player.paused() {
            self.write(b"\x1b7\x1b[37;44m\x1b[?25l")?;

            self.draw_status()?;
            self.draw_help(size)?;

            self.write(b"\x1b8")?;
        }

        self.draw_search(size)?;

        Ok(())
    }

    fn draw_status(&self) -> Result<()> {
        let msg = format!(
            "paused (frame {}/{})",
            self.player.current_frame_idx() + 1,
            self.player.num_frames()
        );

        self.write(b"\x1b[2;2H")?;
        self.write("╭".as_bytes())?;
        self.write("─".repeat(2 + msg.len()).as_bytes())?;
        self.write("╮".as_bytes())?;

        self.write(b"\x1b[3;2H")?;
        self.write(format!("│ {} │", msg).as_bytes())?;

        self.write(b"\x1b[4;2H")?;
        self.write("╰".as_bytes())?;
        self.write("─".repeat(2 + msg.len()).as_bytes())?;
        self.write("╯".as_bytes())?;

        Ok(())
    }

    fn draw_help(&self, size: crate::term::Size) -> Result<()> {
        self.write(
            format!("\x1b[{};{}H", size.rows - 11, size.cols - 32).as_bytes(),
        )?;
        self.write("╭".as_bytes())?;
        self.write("─".repeat(30).as_bytes())?;
        self.write("╮".as_bytes())?;

        self.write(
            format!("\x1b[{};{}H", size.rows - 10, size.cols - 32).as_bytes(),
        )?;
        self.write("│             Keys             │".as_bytes())?;
        self.write(
            format!("\x1b[{};{}H", size.rows - 9, size.cols - 32).as_bytes(),
        )?;
        self.write("│ q: quit                      │".as_bytes())?;
        self.write(
            format!("\x1b[{};{}H", size.rows - 8, size.cols - 32).as_bytes(),
        )?;
        self.write("│ Space: pause/unpause         │".as_bytes())?;
        self.write(
            format!("\x1b[{};{}H", size.rows - 7, size.cols - 32).as_bytes(),
        )?;
        self.write("│ </>: previous/next frame     │".as_bytes())?;
        self.write(
            format!("\x1b[{};{}H", size.rows - 6, size.cols - 32).as_bytes(),
        )?;
        self.write("│ 0/$: first/last frame        │".as_bytes())?;
        self.write(
            format!("\x1b[{};{}H", size.rows - 5, size.cols - 32).as_bytes(),
        )?;
        self.write("│ +/-: increase/decrease speed │".as_bytes())?;
        self.write(
            format!("\x1b[{};{}H", size.rows - 4, size.cols - 32).as_bytes(),
        )?;
        self.write("│ =: normal speed              │".as_bytes())?;
        self.write(
            format!("\x1b[{};{}H", size.rows - 3, size.cols - 32).as_bytes(),
        )?;
        self.write("│ /: search                    │".as_bytes())?;
        self.write(
            format!("\x1b[{};{}H", size.rows - 2, size.cols - 32).as_bytes(),
        )?;
        self.write("│ n/p: next/previous match     │".as_bytes())?;

        self.write(
            format!("\x1b[{};{}H", size.rows - 1, size.cols - 32).as_bytes(),
        )?;
        self.write("╰".as_bytes())?;
        self.write("─".repeat(30).as_bytes())?;
        self.write("╯".as_bytes())?;

        Ok(())
    }

    fn draw_search(&self, size: crate::term::Size) -> Result<()> {
        match &self.input_state {
            InputState::Normal => {
                if !self.player.paused() {
                    return Ok(());
                }

                if let Some(state) = &self.player.search_state {
                    self.write(b"\x1b7\x1b[37;44m")?;
                    self.write(
                        format!("\x1b[{};{}H", 2, size.cols - 32).as_bytes(),
                    )?;
                    self.write("╭".as_bytes())?;
                    self.write("─".repeat(30).as_bytes())?;
                    self.write("╮".as_bytes())?;

                    let msg = if let Some(idx) = state.idx {
                        format!("match ({}/{})", idx + 1, state.count)
                    } else {
                        format!("match (-/{})", state.count)
                    };
                    self.write(
                        format!("\x1b[{};{}H", 3, size.cols - 32).as_bytes(),
                    )?;
                    self.write(
                        format!("│ {}:{} │", msg, " ".repeat(27 - msg.len()))
                            .as_bytes(),
                    )?;

                    self.write(
                        format!("\x1b[{};{}H", 4, size.cols - 32).as_bytes(),
                    )?;
                    self.write(
                        format!("│ {:28} │", state.query.as_str()).as_bytes(),
                    )?;

                    self.write(
                        format!("\x1b[{};{}H", 5, size.cols - 32).as_bytes(),
                    )?;
                    self.write("╰".as_bytes())?;
                    self.write("─".repeat(30).as_bytes())?;
                    self.write("╯".as_bytes())?;

                    self.write(b"\x1b8")?;
                }
            }
            InputState::Search { query } => {
                self.write(b"\x1b7\x1b[37;44m")?;
                self.write(
                    format!("\x1b[{};{}H", 2, size.cols - 32).as_bytes(),
                )?;
                self.write("╭".as_bytes())?;
                self.write("─".repeat(30).as_bytes())?;
                self.write("╮".as_bytes())?;

                self.write(
                    format!("\x1b[{};{}H", 3, size.cols - 32).as_bytes(),
                )?;
                self.write("│ search:                      │".as_bytes())?;

                self.write(
                    format!("\x1b[{};{}H", 4, size.cols - 32).as_bytes(),
                )?;
                self.write(format!("│ {:28} │", query).as_bytes())?;

                self.write(
                    format!("\x1b[{};{}H", 5, size.cols - 32).as_bytes(),
                )?;
                self.write("╰".as_bytes())?;
                self.write("─".repeat(30).as_bytes())?;
                self.write("╯".as_bytes())?;

                self.write(
                    format!(
                        "\x1b8\x1b[{};{}H\x1b[?25h",
                        4,
                        size.cols as usize - 32 + 2 + query.len()
                    )
                    .as_bytes(),
                )?;
            }
        }
        Ok(())
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
                let size = crate::term::Size::get()?;
                let reader = ttyrec::Reader::new(file);
                let parser = vt100::Parser::new(size.rows, size.cols);
                self.file = FileState::Open { reader, parser };
                Ok(component_future::Async::DidWork)
            }
            _ => Ok(component_future::Async::NothingToDo),
        }
    }

    fn poll_read_file(&mut self) -> component_future::Poll<(), Error> {
        if let FileState::Open { reader, parser } = &mut self.file {
            if let Some(frame) = component_future::try_ready!(reader
                .poll_read()
                .context(crate::error::ReadTtyrec))
            {
                parser.process(&frame.data);

                let frame_time = frame.time - reader.offset().unwrap();
                let frame_dur = frame_time - self.last_frame_time;
                self.last_frame_time = frame_time;

                let full = parser.screen().contents_formatted();
                let diff = if let Some(last_frame_screen) =
                    &self.last_frame_screen
                {
                    parser.screen().contents_diff(last_frame_screen)
                } else {
                    full.clone()
                };

                self.last_frame_screen = Some(parser.screen().clone());
                self.player.add_frame(Frame {
                    dur: frame_dur,
                    full,
                    diff,
                });
                if self.player.paused() {
                    self.draw_ui()?;
                }
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
                crossterm::screen::RawScreen::into_raw_mode()
                    .context(crate::error::ToRawMode)?,
            );
        }
        if self.alternate_screen.is_none() {
            self.alternate_screen = Some(
                crossterm::screen::AlternateScreen::to_alternate(false)
                    .context(crate::error::ToAlternateScreen)?,
            );
        }

        let e = component_future::try_ready!(self.key_reader.poll()).unwrap();
        let quit = self.keypress(&e)?;
        if quit {
            self.write(b"\x1b[?25h")?;
            Ok(component_future::Async::Ready(()))
        } else {
            self.redraw()?;
            Ok(component_future::Async::DidWork)
        }
    }

    fn poll_write_terminal(&mut self) -> component_future::Poll<(), Error> {
        if self.player.paused() {
            return Ok(component_future::Async::NothingToDo);
        }

        if let Some(data) = component_future::try_ready!(self.player.poll()) {
            self.write(&data)?;
            self.draw_ui()?;
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
