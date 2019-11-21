use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
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

    pub fn fits_in(self, other: Self) -> bool {
        self.rows <= other.rows && self.cols <= other.cols
    }
}

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Display::fmt(&format!("{}x{}", self.cols, self.rows), f)
    }
}
