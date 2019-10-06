use snafu::ResultExt as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to get terminal size: {}", source))]
    GetTerminalSize { source: crossterm::ErrorKind },

    #[snafu(display("failed to resize pty: {}", source))]
    ResizePty { source: std::io::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

const RESET: &[&[u8]] = &[
    b"\x1b[3J\x1b[H\x1b[2J",
    b"\x1b[H\x1b[J",
    b"\x1b[H\x1b[2J",
    b"\x1bc",
];
const WINDOW_TITLE: &[(&[u8], &[u8])] =
    &[(b"\x1b]0;", b"\x07"), (b"\x1b]2;", b"\x07")];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}

impl Size {
    pub fn get() -> Result<Self> {
        let (cols, rows) =
            crossterm::terminal().size().context(GetTerminalSize)?;
        Ok(Self { rows, cols })
    }

    pub fn resize_pty<T: tokio_pty_process::PtyMaster>(
        &self,
        pty: &T,
    ) -> futures::Poll<(), Error> {
        pty.resize(self.rows, self.cols).context(ResizePty)
    }
}

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Display::fmt(&format!("{}x{}", self.cols, self.rows), f)
    }
}

#[derive(Debug, Default)]
pub struct Buffer {
    max_size: usize,
    contents: Vec<u8>,
    title: String,
}

impl Buffer {
    pub fn new(max_size: usize) -> Self {
        let mut self_ = Self::default();
        self_.max_size = max_size;
        self_
    }

    pub fn append(&mut self, mut buf: &[u8]) -> usize {
        let mut truncated = 0;

        let mut found = None;
        for window_title in WINDOW_TITLE {
            if let Some(i) = twoway::rfind_bytes(buf, window_title.0) {
                if let Some(j) = twoway::find_bytes(&buf[i..], window_title.1)
                {
                    let start = i + window_title.0.len();
                    let end = j + i;
                    if let Ok(title) = std::str::from_utf8(&buf[start..end]) {
                        if let Some((i2, _)) = found {
                            if i > i2 {
                                found = Some((i, title));
                            }
                        } else {
                            found = Some((i, title));
                        }
                    }
                }
            }
        }
        if let Some((_, title)) = found {
            self.title = title.to_string();
        }

        let mut found = None;
        for reset in RESET {
            if let Some(i) = twoway::rfind_bytes(buf, reset) {
                let len = reset.len();
                if let Some((i2, len2)) = found {
                    if i + len > i2 + len2 || (i + len == i2 + len2 && i < i2)
                    {
                        found = Some((i, len));
                    }
                } else {
                    found = Some((i, len));
                }
            }
        }
        if let Some((i, _)) = found {
            truncated = self.contents.len();
            self.contents.clear();
            buf = &buf[i..];
        }

        let prev_len = self.contents.len();
        self.contents.extend_from_slice(buf);
        if self.contents.len() > self.max_size {
            let new_contents = self
                .contents
                .split_off(self.contents.len() - self.max_size / 2);
            truncated = std::cmp::min(self.contents.len(), prev_len);
            self.contents = new_contents;
        }

        truncated
    }

    pub fn contents(&self) -> &[u8] {
        &self.contents
    }

    pub fn len(&self) -> usize {
        self.contents.len()
    }

    pub fn title(&self) -> &str {
        &self.title
    }
}

#[cfg(test)]
#[allow(clippy::cognitive_complexity)]
#[allow(clippy::shadow_unrelated)]
mod test {
    use super::*;

    #[test]
    fn test_basic() {
        let mut buffer = Buffer::new(100);
        assert_eq!(buffer.contents(), b"");
        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.title(), "");

        let n = buffer.append(b"foo");
        assert_eq!(buffer.contents(), b"foo");
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append(b"bar");
        assert_eq!(buffer.contents(), b"foobar");
        assert_eq!(buffer.len(), 6);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);
    }

    #[test]
    fn test_clear() {
        let mut buffer = Buffer::new(100);

        let n = buffer.append(b"foo");
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.contents(), b"foo");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append(b"\x1b[3J\x1b[H\x1b[2J");
        assert_eq!(buffer.len(), 11);
        assert_eq!(buffer.contents(), b"\x1b[3J\x1b[H\x1b[2J");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 3);

        let n = buffer.append(b"bar");
        assert_eq!(buffer.len(), 14);
        assert_eq!(buffer.contents(), b"\x1b[3J\x1b[H\x1b[2Jbar");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append(b"baz\x1bcquux");
        assert_eq!(buffer.len(), 6);
        assert_eq!(buffer.contents(), b"\x1bcquux");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 14);

        let n = buffer.append(b"blorg\x1b[H\x1b[J");
        assert_eq!(buffer.len(), 6);
        assert_eq!(buffer.contents(), b"\x1b[H\x1b[J");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 6);

        let n = buffer.append(b"\x1b[H\x1b[2Jabc");
        assert_eq!(buffer.len(), 10);
        assert_eq!(buffer.contents(), b"\x1b[H\x1b[2Jabc");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 6);

        let n = buffer.append(b"first\x1bcsecond\x1b[H\x1b[2Jthird");
        assert_eq!(buffer.len(), 12);
        assert_eq!(buffer.contents(), b"\x1b[H\x1b[2Jthird");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 10);

        let n = buffer.append(b"first\x1b[H\x1b[2Jsecond\x1bcthird");
        assert_eq!(buffer.len(), 7);
        assert_eq!(buffer.contents(), b"\x1bcthird");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 12);
    }

    #[test]
    fn test_title() {
        let mut buffer = Buffer::new(100);

        let n = buffer.append(b"\x1b]0;this is a title\x07");
        assert_eq!(buffer.len(), 20);
        assert_eq!(buffer.contents(), b"\x1b]0;this is a title\x07");
        assert_eq!(buffer.title(), "this is a title");
        assert_eq!(n, 0);

        let n = buffer.append(b"\x1b]2;this is another title\x07");
        assert_eq!(buffer.len(), 46);
        assert_eq!(
            buffer.contents(),
            &b"\x1b]0;this is a title\x07\x1b]2;this is another title\x07"[..]
        );
        assert_eq!(buffer.title(), "this is another title");
        assert_eq!(n, 0);

        let n = buffer.append(b"\x1bcfoo");
        assert_eq!(buffer.len(), 5);
        assert_eq!(buffer.contents(), b"\x1bcfoo");
        assert_eq!(buffer.title(), "this is another title");
        assert_eq!(n, 46);

        let n = buffer
            .append(b"\x1bcabc\x1b]0;title1\x07def\x1b]2;title2\x07ghi");
        assert_eq!(buffer.len(), 33);
        assert_eq!(
            buffer.contents(),
            &b"\x1bcabc\x1b]0;title1\x07def\x1b]2;title2\x07ghi"[..]
        );
        assert_eq!(buffer.title(), "title2");
        assert_eq!(n, 5);

        let n = buffer
            .append(b"\x1bcabc\x1b]2;title3\x07def\x1b]0;title4\x07ghi");
        assert_eq!(buffer.len(), 33);
        assert_eq!(
            buffer.contents(),
            &b"\x1bcabc\x1b]2;title3\x07def\x1b]0;title4\x07ghi"[..]
        );
        assert_eq!(buffer.title(), "title4");
        assert_eq!(n, 33);
    }

    #[test]
    fn test_size_limit() {
        let mut buffer = Buffer::new(100);

        let n = buffer.append(b"foobarbazq");
        assert_eq!(buffer.len(), 10);
        assert_eq!(buffer.contents(), b"foobarbazq");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append("0123456789".repeat(9).as_ref());
        assert_eq!(buffer.len(), 100);
        assert_eq!(
            buffer.contents(),
            &b"foobarbazq012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789"[..]
        );
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append(b"z");
        assert_eq!(buffer.len(), 50);
        assert_eq!(
            buffer.contents(),
            &b"1234567890123456789012345678901234567890123456789z"[..]
        );
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 51);

        let n = buffer.append("abcdefghij".repeat(15).as_ref());
        assert_eq!(buffer.len(), 50);
        assert_eq!(
            buffer.contents(),
            &b"abcdefghijabcdefghijabcdefghijabcdefghijabcdefghij"[..]
        );
        assert_eq!(buffer.title(), "");
        // the return value of append represents how many characters from the
        // beginning of the previous .contents() value no longer exist, so it
        // should never return a greater value than the previous length of
        // .contents()
        assert_eq!(n, 50);
    }
}
