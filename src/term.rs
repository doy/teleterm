const RESET: &[&[u8]] = &[
    b"\x1b[3J\x1b[H\x1b[2J",
    b"\x1b[H\x1b[J",
    b"\x1b[H\x1b[2J",
    b"\x1bc",
];
const WINDOW_TITLE: &[(&[u8], &[u8])] =
    &[(b"\x1b]0;", b"\x07"), (b"\x1b]2;", b"\x07")];

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
        for window_title in WINDOW_TITLE {
            if let Some(i) = twoway::rfind_bytes(buf, window_title.0) {
                if let Some(j) = twoway::find_bytes(&buf[i..], window_title.1)
                {
                    let start = i + window_title.0.len();
                    let end = j + i;
                    if let Ok(title) = std::str::from_utf8(&buf[start..end]) {
                        self.title = title.to_string();
                    }
                }
            }
        }
        for reset in RESET {
            if let Some(i) = twoway::rfind_bytes(buf, reset) {
                truncated = self.contents.len();
                self.contents.clear();
                buf = &buf[i..];
            }
        }

        self.contents.extend_from_slice(buf);
        if self.contents.len() > self.max_size {
            let new_contents = self.contents.split_off(self.max_size / 2);
            truncated = self.contents.len();
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
