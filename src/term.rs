use crate::prelude::*;

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
        let (cols, rows) = crossterm::terminal()
            .size()
            .context(crate::error::GetTerminalSize)?;
        Ok(Self { rows, cols })
    }

    pub fn resize_pty<T: tokio_pty_process::PtyMaster>(
        &self,
        pty: &T,
    ) -> futures::Poll<(), Error> {
        pty.resize(self.rows, self.cols)
            .context(crate::error::ResizePty)
    }

    pub fn fits_in(&self, other: &Self) -> bool {
        self.rows <= other.rows && self.cols <= other.cols
    }
}

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Display::fmt(&format!("{}x{}", self.cols, self.rows), f)
    }
}

#[derive(Debug, Clone, Default)]
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

    pub fn append(&mut self, buf: &[u8], written: usize) -> usize {
        self.contents.extend_from_slice(buf);

        if let Some(title) = self.find_window_title(buf) {
            self.title = title;
        }

        if written == 0 {
            return written;
        }

        if self.has_reset(buf) {
            self.truncate_at(written);
            return written;
        }

        if self.contents.len() > self.max_size {
            let to_drop = self.contents.len() - self.max_size / 2;
            let to_drop = to_drop.min(written);
            if to_drop > 0 {
                self.truncate_at(to_drop);
                return to_drop;
            }
        }

        0
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

    fn find_window_title(&self, buf: &[u8]) -> Option<String> {
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
        found.map(|(_, title)| title.to_string())
    }

    fn has_reset(&self, buf: &[u8]) -> bool {
        for reset in RESET {
            if twoway::find_bytes(buf, reset).is_some() {
                return true;
            }
        }
        false
    }

    fn truncate_at(&mut self, i: usize) {
        let new_contents = self.contents.split_off(i);
        self.contents = new_contents;
    }
}

#[cfg(test)]
#[allow(clippy::cognitive_complexity)]
#[allow(clippy::redundant_clone)]
#[allow(clippy::shadow_unrelated)]
mod test {
    use super::*;

    #[test]
    fn test_basic() {
        let mut buffer = Buffer::new(100);
        assert_eq!(buffer.contents(), b"");
        assert_eq!(buffer.len(), 0);
        assert_eq!(buffer.title(), "");

        let n = buffer.append(b"foo", 0);
        assert_eq!(buffer.contents(), b"foo");
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append(b"bar", 3);
        assert_eq!(buffer.contents(), b"foobar");
        assert_eq!(buffer.len(), 6);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);
    }

    #[test]
    fn test_clear() {
        let mut buffer = Buffer::new(100);

        let n = buffer.append(b"foo", 0);
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.contents(), b"foo");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append(b"\x1b[3J\x1b[H\x1b[2J", 3);
        assert_eq!(buffer.len(), 11);
        assert_eq!(buffer.contents(), b"\x1b[3J\x1b[H\x1b[2J");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 3);

        let n = buffer.append(b"bar", 11);
        assert_eq!(buffer.len(), 14);
        assert_eq!(buffer.contents(), b"\x1b[3J\x1b[H\x1b[2Jbar");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append(b"baz\x1bcquux", 14);
        assert_eq!(buffer.len(), 9);
        assert_eq!(buffer.contents(), b"baz\x1bcquux");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 14);

        let n = buffer.append(b"blorg\x1b[H\x1b[J", 9);
        assert_eq!(buffer.len(), 11);
        assert_eq!(buffer.contents(), b"blorg\x1b[H\x1b[J");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 9);

        let n = buffer.append(b"\x1b[H\x1b[2Jabc", 11);
        assert_eq!(buffer.len(), 10);
        assert_eq!(buffer.contents(), b"\x1b[H\x1b[2Jabc");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 11);

        let n = buffer.append(b"first\x1bcsecond\x1b[H\x1b[2Jthird", 10);
        assert_eq!(buffer.len(), 25);
        assert_eq!(buffer.contents(), b"first\x1bcsecond\x1b[H\x1b[2Jthird");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 10);

        let n = buffer.append(b"first\x1b[H\x1b[2Jsecond\x1bcthird", 25);
        assert_eq!(buffer.len(), 25);
        assert_eq!(buffer.contents(), b"first\x1b[H\x1b[2Jsecond\x1bcthird");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 25);
    }

    #[test]
    fn test_title() {
        let mut buffer = Buffer::new(100);

        let n = buffer.append(b"\x1b]0;this is a title\x07", 0);
        assert_eq!(buffer.len(), 20);
        assert_eq!(buffer.contents(), b"\x1b]0;this is a title\x07");
        assert_eq!(buffer.title(), "this is a title");
        assert_eq!(n, 0);

        let n = buffer.append(b"\x1b]2;this is another title\x07", 20);
        assert_eq!(buffer.len(), 46);
        assert_eq!(
            buffer.contents(),
            &b"\x1b]0;this is a title\x07\x1b]2;this is another title\x07"[..]
        );
        assert_eq!(buffer.title(), "this is another title");
        assert_eq!(n, 0);

        let n = buffer.append(b"\x1bcfoo", 46);
        assert_eq!(buffer.len(), 5);
        assert_eq!(buffer.contents(), b"\x1bcfoo");
        assert_eq!(buffer.title(), "this is another title");
        assert_eq!(n, 46);

        let n = buffer
            .append(b"\x1bcabc\x1b]0;title1\x07def\x1b]2;title2\x07ghi", 5);
        assert_eq!(buffer.len(), 33);
        assert_eq!(
            buffer.contents(),
            &b"\x1bcabc\x1b]0;title1\x07def\x1b]2;title2\x07ghi"[..]
        );
        assert_eq!(buffer.title(), "title2");
        assert_eq!(n, 5);

        let n = buffer
            .append(b"\x1bcabc\x1b]2;title3\x07def\x1b]0;title4\x07ghi", 33);
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

        let n = buffer.append(b"foobarbazq", 0);
        assert_eq!(buffer.len(), 10);
        assert_eq!(buffer.contents(), b"foobarbazq");
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append("0123456789".repeat(9).as_ref(), 10);
        assert_eq!(buffer.len(), 100);
        assert_eq!(
            buffer.contents(),
            &b"foobarbazq012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789"[..]
        );
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append(b"z", 100);
        assert_eq!(buffer.len(), 50);
        assert_eq!(
            buffer.contents(),
            &b"1234567890123456789012345678901234567890123456789z"[..]
        );
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 51);

        let n = buffer.append("abcdefghij".repeat(15).as_ref(), 50);
        assert_eq!(buffer.len(), 150);
        assert_eq!(
            buffer.contents(),
            &b"abcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghij"[..]
        );
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 50);
    }

    #[test]
    fn test_written() {
        let mut buffer = Buffer::new(100);

        let n = buffer.append("abcdefghij".repeat(15).as_ref(), 0);
        assert_eq!(buffer.len(), 150);
        assert_eq!(
            buffer.contents(),
            &b"abcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghij"[..]
        );
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let mut buffer2 = buffer.clone();
        let n = buffer2.append(b"z", 0);
        assert_eq!(buffer2.len(), 151);
        assert_eq!(
            buffer2.contents(),
            &b"abcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijz"[..]
        );
        assert_eq!(buffer2.title(), "");
        assert_eq!(n, 0);

        let mut buffer2 = buffer.clone();
        let n = buffer2.append(b"z", 20);
        assert_eq!(buffer2.len(), 131);
        assert_eq!(
            buffer2.contents(),
            &b"abcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijz"[..]
        );
        assert_eq!(buffer2.title(), "");
        assert_eq!(n, 20);

        let mut buffer2 = buffer.clone();
        let n = buffer2.append(b"z", 130);
        assert_eq!(buffer2.len(), 50);
        assert_eq!(
            buffer2.contents(),
            &b"bcdefghijabcdefghijabcdefghijabcdefghijabcdefghijz"[..]
        );
        assert_eq!(buffer2.title(), "");
        assert_eq!(n, 101);
    }
}
