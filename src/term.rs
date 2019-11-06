use crate::prelude::*;

const RESET: &[&[u8]] = &[b"\x1bc"];
const CLEAR: &[&[u8]] =
    &[b"\x1b[3J\x1b[H\x1b[2J", b"\x1b[H\x1b[J", b"\x1b[H\x1b[2J"];
const WINDOW_TITLE: &[(&[u8], &[u8])] =
    &[(b"\x1b]0;", b"\x07"), (b"\x1b]2;", b"\x07")];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}

impl Size {
    pub fn get() -> Result<Self> {
        let (cols, rows) = crossterm::terminal::size()
            .context(crate::error::GetTerminalSize)?;
        Ok(Self { rows, cols })
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

    // doesn't parse out window titles, since we don't care about them. also
    // tries to spread out truncations a bit to keep each individual run of
    // this function reasonably fast (trying to avoid most runs being fast but
    // occasionally one being extra slow when it hits a limit). this makes it
    // a bit less accurate (clears don't actually reset the entire terminal
    // state, so dropping everything before them when we see one isn't quite
    // correct), but the client side buffer is only used in the case where
    // someone reconnects after disconnecting from the server, which should be
    // uncommon enough that it shouldn't matter all that much.
    pub fn append_client(&mut self, buf: &[u8], written: usize) -> usize {
        self.contents.extend_from_slice(buf);

        if written == 0 {
            return written;
        }

        if self.find_reset(buf).is_some() {
            self.truncate_at(written);
            return written;
        }

        if self.find_clear(buf).is_some() {
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

    // aim for accuracy over speed - on the client side, we're guaranteed to
    // see every byte because the bytes are coming from the client, but our
    // truncation behavior isn't particularly guaranteed to give correct
    // results in all cases, so try to avoid doing it too much if we can help
    // it. this makes this function slower (and spikier) than append_client,
    // but that's pretty okay because we don't care as much about latency
    // here.
    pub fn append_server(&mut self, buf: &[u8]) {
        self.contents.extend_from_slice(buf);

        if let Some(title) = self.find_window_title(buf) {
            self.title = title;
        }

        // resets are actually safe to truncate at, because they reset ~all
        // terminal state
        if let Some(i) = self.find_reset(buf) {
            self.truncate_at(self.contents.len() - buf.len() + i);
        }

        while self.contents.len() > self.max_size {
            let hard_truncate = self.contents.len() - self.max_size / 2;
            // off by one so that we skip over a clear escape sequence at the
            // start of the buffer
            if let Some(i) = self.find_clear(&self.contents[1..]) {
                self.truncate_at((i + 1).min(hard_truncate));
            } else {
                self.truncate_at(hard_truncate);
            }
        }
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

    fn find_clear(&self, buf: &[u8]) -> Option<usize> {
        for clear in CLEAR {
            if let Some(i) = twoway::find_bytes(buf, clear) {
                return Some(i);
            }
        }
        None
    }

    fn find_reset(&self, buf: &[u8]) -> Option<usize> {
        for reset in RESET {
            // rfind because we only ever care about the most recent reset,
            // unlike clears where we might want to be more conservative
            if let Some(i) = twoway::rfind_bytes(buf, reset) {
                return Some(i);
            }
        }
        None
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

        let n = buffer.append_client(b"foo", 0);
        assert_eq!(buffer.contents(), b"foo");
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append_client(b"bar", 3);
        assert_eq!(buffer.contents(), b"foobar");
        assert_eq!(buffer.len(), 6);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);
    }

    #[test]
    fn test_clear() {
        let mut buffer = Buffer::new(100);

        let n = buffer.append_client(b"foo", 0);
        assert_eq!(buffer.contents(), b"foo");
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append_client(b"\x1b[3J\x1b[H\x1b[2J", 3);
        assert_eq!(buffer.contents(), b"\x1b[3J\x1b[H\x1b[2J");
        assert_eq!(buffer.len(), 11);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 3);

        let n = buffer.append_client(b"bar", 11);
        assert_eq!(buffer.contents(), b"\x1b[3J\x1b[H\x1b[2Jbar");
        assert_eq!(buffer.len(), 14);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append_client(b"baz\x1bcquux", 14);
        assert_eq!(buffer.contents(), b"baz\x1bcquux");
        assert_eq!(buffer.len(), 9);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 14);

        let n = buffer.append_client(b"blorg\x1b[H\x1b[J", 9);
        assert_eq!(buffer.contents(), b"blorg\x1b[H\x1b[J");
        assert_eq!(buffer.len(), 11);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 9);

        let n = buffer.append_client(b"\x1b[H\x1b[2Jabc", 11);
        assert_eq!(buffer.contents(), b"\x1b[H\x1b[2Jabc");
        assert_eq!(buffer.len(), 10);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 11);

        let n =
            buffer.append_client(b"first\x1bcsecond\x1b[H\x1b[2Jthird", 10);
        assert_eq!(buffer.contents(), b"first\x1bcsecond\x1b[H\x1b[2Jthird");
        assert_eq!(buffer.len(), 25);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 10);

        let n =
            buffer.append_client(b"first\x1b[H\x1b[2Jsecond\x1bcthird", 25);
        assert_eq!(buffer.contents(), b"first\x1b[H\x1b[2Jsecond\x1bcthird");
        assert_eq!(buffer.len(), 25);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 25);
    }

    #[test]
    fn test_title() {
        let mut buffer = Buffer::new(50);

        buffer.append_server(b"\x1b]0;this is a title\x07");
        assert_eq!(buffer.contents(), b"\x1b]0;this is a title\x07");
        assert_eq!(buffer.len(), 20);
        assert_eq!(buffer.title(), "this is a title");

        buffer.append_server(b"\x1b]2;this is another title\x07");
        assert_eq!(
            buffer.contents(),
            &b"\x1b]0;this is a title\x07\x1b]2;this is another title\x07"[..]
        );
        assert_eq!(buffer.len(), 46);
        assert_eq!(buffer.title(), "this is another title");

        buffer.append_server(b"\x1bcfoo");
        assert_eq!(buffer.contents(), &b"\x1bcfoo"[..]);
        assert_eq!(buffer.len(), 5);
        assert_eq!(buffer.title(), "this is another title");

        buffer.append_server(
            b"\x1bcabc\x1b]0;title1\x07def\x1b]2;title2\x07ghi",
        );
        assert_eq!(
            buffer.contents(),
            &b"\x1bcabc\x1b]0;title1\x07def\x1b]2;title2\x07ghi"[..]
        );
        assert_eq!(buffer.len(), 33);
        assert_eq!(buffer.title(), "title2");

        buffer.append_server(
            b"\x1bcabc\x1b]2;title3\x07def\x1b]0;title4\x07ghi",
        );
        assert_eq!(
            buffer.contents(),
            &b"\x1bcabc\x1b]2;title3\x07def\x1b]0;title4\x07ghi"[..]
        );
        assert_eq!(buffer.len(), 33);
        assert_eq!(buffer.title(), "title4");
    }

    #[test]
    fn test_size_limit_client() {
        let mut buffer = Buffer::new(100);

        let n = buffer.append_client(b"foobarbazq", 0);
        assert_eq!(buffer.contents(), b"foobarbazq");
        assert_eq!(buffer.len(), 10);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append_client("0123456789".repeat(9).as_ref(), 10);
        assert_eq!(
            buffer.contents(),
            &b"foobarbazq012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789"[..]
        );
        assert_eq!(buffer.len(), 100);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let n = buffer.append_client(b"z", 100);
        assert_eq!(
            buffer.contents(),
            &b"1234567890123456789012345678901234567890123456789z"[..]
        );
        assert_eq!(buffer.len(), 50);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 51);

        let n = buffer.append_client("abcdefghij".repeat(15).as_ref(), 50);
        assert_eq!(
            buffer.contents(),
            &b"abcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghij"[..]
        );
        assert_eq!(buffer.len(), 150);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 50);
    }

    #[test]
    fn test_written() {
        let mut buffer = Buffer::new(100);

        let n = buffer.append_client("abcdefghij".repeat(15).as_ref(), 0);
        assert_eq!(
            buffer.contents(),
            &b"abcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghij"[..]
        );
        assert_eq!(buffer.len(), 150);
        assert_eq!(buffer.title(), "");
        assert_eq!(n, 0);

        let mut buffer2 = buffer.clone();
        let n = buffer2.append_client(b"z", 0);
        assert_eq!(
            buffer2.contents(),
            &b"abcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijz"[..]
        );
        assert_eq!(buffer2.len(), 151);
        assert_eq!(buffer2.title(), "");
        assert_eq!(n, 0);

        let mut buffer2 = buffer.clone();
        let n = buffer2.append_client(b"z", 20);
        assert_eq!(
            buffer2.contents(),
            &b"abcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijz"[..]
        );
        assert_eq!(buffer2.len(), 131);
        assert_eq!(buffer2.title(), "");
        assert_eq!(n, 20);

        let mut buffer2 = buffer.clone();
        let n = buffer2.append_client(b"z", 130);
        assert_eq!(
            buffer2.contents(),
            &b"bcdefghijabcdefghijabcdefghijabcdefghijabcdefghijz"[..]
        );
        assert_eq!(buffer2.len(), 50);
        assert_eq!(buffer2.title(), "");
        assert_eq!(n, 101);
    }

    #[test]
    fn test_server() {
        let mut buffer = Buffer::new(100);

        buffer.append_server(b"foobar");
        assert_eq!(buffer.contents(), b"foobar");
        assert_eq!(buffer.len(), 6);
        assert_eq!(buffer.title(), "");

        buffer.append_server(b"\x1b[3J\x1b[H\x1b[2J");
        assert_eq!(buffer.contents(), b"foobar\x1b[3J\x1b[H\x1b[2J");
        assert_eq!(buffer.len(), 17);
        assert_eq!(buffer.title(), "");

        buffer.append_server("abcdefghij".repeat(8).as_ref());
        assert_eq!(buffer.contents(), &b"foobar\x1b[3J\x1b[H\x1b[2Jabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghij"[..]);
        assert_eq!(buffer.len(), 97);
        assert_eq!(buffer.title(), "");

        buffer.append_server(b"abcd");
        assert_eq!(buffer.contents(), &b"\x1b[3J\x1b[H\x1b[2Jabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijabcd"[..]);
        assert_eq!(buffer.len(), 95);
        assert_eq!(buffer.title(), "");

        buffer.append_server("\x1b[H\x1b[Jfooo".repeat(8).as_ref());
        assert_eq!(buffer.contents(), &b"\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfooo"[..]);
        assert_eq!(buffer.len(), 80);
        assert_eq!(buffer.title(), "");

        buffer.append_server("abcdefghij".repeat(5).as_ref());
        assert_eq!(buffer.contents(), &b"\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfoooabcdefghijabcdefghijabcdefghijabcdefghijabcdefghij"[..]);
        assert_eq!(buffer.len(), 100);
        assert_eq!(buffer.title(), "");

        buffer.append_server(b"z");
        assert_eq!(buffer.contents(), &b"\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfoooabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijz"[..]);
        assert_eq!(buffer.len(), 91);
        assert_eq!(buffer.title(), "");

        buffer.append_server(b"bcdefghijabcdefghij\x1b[H\x1b[Jfooo");
        assert_eq!(buffer.contents(), &b"\x1b[H\x1b[Jfooo\x1b[H\x1b[Jfoooabcdefghijabcdefghijabcdefghijabcdefghijabcdefghijzbcdefghijabcdefghij\x1b[H\x1b[Jfooo"[..]);
        assert_eq!(buffer.len(), 100);
        assert_eq!(buffer.title(), "");

        buffer.append_server(b"abcdefghijz");
        assert_eq!(
            buffer.contents(),
            &b"bcdefghijzbcdefghijabcdefghij\x1b[H\x1b[Jfoooabcdefghijz"[..]
        );
        assert_eq!(buffer.len(), 50);
        assert_eq!(buffer.title(), "");

        buffer.append_server("\x1bcfooobaar".repeat(8).as_ref());
        assert_eq!(buffer.contents(), b"\x1bcfooobaar");
        assert_eq!(buffer.len(), 10);
        assert_eq!(buffer.title(), "");
    }
}
