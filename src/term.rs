const RESET: &[&[u8]] = &[b"\x1b[H\x1b[J", b"\x1b[2J"];

#[derive(Debug, Default)]
pub struct Buffer(Vec<u8>);

impl Buffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&mut self, mut buf: &[u8]) -> bool {
        let mut cleared = false;
        for reset in RESET {
            if let Some(i) = twoway::find_bytes(&buf, reset) {
                cleared = true;
                self.0.clear();
                buf = &buf[i..];
            }
        }

        self.0.extend_from_slice(buf);

        cleared
    }

    pub fn contents(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}
