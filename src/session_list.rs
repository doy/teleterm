use snafu::ResultExt as _;
use std::io::Write as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("{}", source))]
    Resize { source: crate::term::Error },

    #[snafu(display("failed to write to terminal: {}", source))]
    WriteTerminalCrossterm { source: crossterm::ErrorKind },

    #[snafu(display("failed to flush writes to terminal: {}", source))]
    FlushTerminal { source: std::io::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct SessionList {
    sessions: Vec<crate::protocol::Session>,
    offset: usize,
    size: crate::term::Size,
}

impl SessionList {
    pub fn new(sessions: Vec<crate::protocol::Session>) -> Result<Self> {
        let mut by_name = std::collections::HashMap::new();
        for session in sessions {
            if !by_name.contains_key(&session.username) {
                by_name.insert(session.username.clone(), vec![]);
            }
            by_name.get_mut(&session.username).unwrap().push(session);
        }
        let mut names: Vec<_> = by_name.keys().cloned().collect();
        names.sort_by(|a: &String, b: &String| {
            let a_idle =
                by_name[a].iter().min_by_key(|session| session.idle_time);
            let b_idle =
                by_name[b].iter().min_by_key(|session| session.idle_time);
            // these unwraps are safe because we know that none of the vecs in
            // the map can be empty
            a_idle.unwrap().idle_time.cmp(&b_idle.unwrap().idle_time)
        });
        for name in &names {
            if let Some(sessions) = by_name.get_mut(name) {
                sessions.sort_by_key(|s| s.idle_time);
            }
        }

        let mut sorted = vec![];
        for name in names {
            let sessions = by_name.remove(&name).unwrap();
            for session in sessions {
                sorted.push(session);
            }
        }

        Ok(Self {
            sessions: sorted,
            offset: 0,
            size: crate::term::Size::get().context(Resize)?,
        })
    }

    pub fn print(&self) -> Result<()> {
        let char_width = 2;

        let max_name_width = (self.size.cols / 3) as usize;
        let name_width = self
            .sessions
            .iter()
            .map(|s| s.username.len())
            .max()
            .unwrap_or(4);
        // XXX unstable
        // let name_width = name_width.clamp(4, max_name_width);
        let name_width = if name_width < 4 {
            4
        } else if name_width > max_name_width {
            max_name_width
        } else {
            name_width
        };

        let size_width = 7;

        let max_idle_time =
            self.sessions.iter().map(|s| s.idle_time).max().unwrap_or(4);
        let idle_width = format_time(max_idle_time).len();
        let idle_width = if idle_width < 4 { 4 } else { idle_width };

        let max_title_width = (self.size.cols as usize)
            - char_width
            - 3
            - name_width
            - 3
            - size_width
            - 3
            - idle_width
            - 3;

        crossterm::terminal()
            .clear(crossterm::ClearType::All)
            .context(WriteTerminalCrossterm)?;
        println!("welcome to shellshare\r");
        println!("available sessions:\r");
        println!("\r");
        println!(
            "{:4$} | {:5$} | {:6$} | {:7$} | title\r",
            "",
            "name",
            "size",
            "idle",
            char_width,
            name_width,
            size_width,
            idle_width
        );
        println!("{}\r", "-".repeat(self.size.cols as usize));

        let mut prev_name: Option<&str> = None;
        for (i, session) in self.sessions.iter().skip(self.offset).enumerate()
        {
            if let Some(c) = self.idx_to_char(i) {
                let first = if let Some(name) = prev_name {
                    name != session.username
                } else {
                    true
                };

                let display_char = format!("{})", c);
                let display_name = if first {
                    truncate(&session.username, max_name_width)
                } else {
                    "".to_string()
                };
                let display_size_plain = format!("{}", &session.size);
                let display_size_full = if session.size == self.size {
                    // XXX i should be able to use crossterm::style here, but
                    // it has bugs
                    format!("\x1b[32m{}\x1b[m", display_size_plain)
                } else if session.size.fits_in(&self.size) {
                    display_size_plain.clone()
                } else {
                    // XXX i should be able to use crossterm::style here, but
                    // it has bugs
                    format!("\x1b[31m{}\x1b[m", display_size_plain)
                };
                let display_idle = format_time(session.idle_time);
                let display_title = truncate(&session.title, max_title_width);

                println!(
                    "{:5$} | {:6$} | {:7$} | {:8$} | {}\r",
                    display_char,
                    display_name,
                    display_size_full,
                    display_idle,
                    display_title,
                    char_width,
                    name_width,
                    size_width
                        + (display_size_full.len()
                            - display_size_plain.len()),
                    idle_width
                );

                prev_name = Some(&session.username);
            } else {
                break;
            }
        }
        print!(
            "({}/{}) space: refresh, q: quit, <: prev page, >: next page --> ",
            self.current_page(),
            self.total_pages(),
        );
        std::io::stdout().flush().context(FlushTerminal)?;

        Ok(())
    }

    pub fn resize(&mut self, size: crate::term::Size) {
        self.size = size;
    }

    pub fn id_for(&self, c: char) -> Option<&str> {
        self.char_to_idx(c).and_then(|i| {
            self.sessions.get(i + self.offset).map(|s| s.id.as_ref())
        })
    }

    pub fn next_page(&mut self) {
        let inc = self.limit();
        if self.offset + inc < self.sessions.len() {
            self.offset += inc;
        }
    }

    pub fn prev_page(&mut self) {
        let dec = self.limit();
        if self.offset >= dec {
            self.offset -= dec;
        }
    }

    fn idx_to_char(&self, mut i: usize) -> Option<char> {
        if i >= self.limit() {
            return None;
        }

        // 'q' shouldn't be a list option, since it is bound to quit
        if i >= 16 {
            i += 1;
        }

        #[allow(clippy::cast_possible_truncation)]
        Some(std::char::from_u32(('a' as u32) + (i as u32)).unwrap())
    }

    fn char_to_idx(&self, c: char) -> Option<usize> {
        if c == 'q' {
            return None;
        }

        let i = ((c as i32) - ('a' as i32)) as isize;
        if i < 0 {
            return None;
        }
        #[allow(clippy::cast_sign_loss)]
        let mut i = i as usize;

        // 'q' shouldn't be a list option, since it is bound to quit
        if i > 16 {
            i -= 1;
        }

        if i > self.limit() {
            return None;
        }

        Some(i)
    }

    fn current_page(&self) -> usize {
        self.offset / self.limit() + 1
    }

    fn total_pages(&self) -> usize {
        if self.sessions.is_empty() {
            1
        } else {
            (self.sessions.len() - 1) / self.limit() + 1
        }
    }

    fn limit(&self) -> usize {
        let limit = self.size.rows as usize - 6;

        // enough for a-z except q - if we want to allow more than this, we'll
        // need to come up with a better way of choosing streams
        if limit > 25 {
            25
        } else {
            limit
        }
    }
}

fn format_time(dur: u32) -> String {
    let secs = dur % 60;
    let dur = dur / 60;
    if dur == 0 {
        return format!("{}s", secs);
    }

    let mins = dur % 60;
    let dur = dur / 60;
    if dur == 0 {
        return format!("{}m{:02}s", mins, secs);
    }

    let hours = dur % 24;
    let dur = dur / 24;
    if dur == 0 {
        return format!("{}h{:02}m{:02}s", hours, mins, secs);
    }

    let days = dur;
    format!("{}d{:02}h{:02}m{:02}s", days, hours, mins, secs)
}

fn truncate(s: &str, len: usize) -> String {
    if s.len() <= len {
        s.to_string()
    } else {
        format!("{}...", &s[..(len - 3)])
    }
}
