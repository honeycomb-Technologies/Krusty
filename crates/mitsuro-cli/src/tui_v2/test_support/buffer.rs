//! Exact character and style snapshot values.

use ratatui::{buffer::Buffer, style::Color};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellSnapshot {
    pub symbol: String,
    pub foreground: Color,
    pub background: Color,
    pub modifiers: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BufferSnapshot {
    pub width: u16,
    pub height: u16,
    pub cells: Vec<CellSnapshot>,
    pub cursor: Option<CursorSnapshot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CursorSnapshot {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
}

impl BufferSnapshot {
    pub fn capture(buffer: &Buffer, cursor: Option<CursorSnapshot>) -> Self {
        let cells = buffer
            .content
            .iter()
            .map(|cell| CellSnapshot {
                symbol: cell.symbol().to_owned(),
                foreground: cell.fg,
                background: cell.bg,
                modifiers: cell.modifier.bits(),
            })
            .collect();

        Self {
            width: buffer.area.width,
            height: buffer.area.height,
            cells,
            cursor,
        }
    }

    pub fn text(&self) -> String {
        self.cells
            .chunks(self.width.into())
            .map(|row| {
                row.iter()
                    .map(|cell| cell.symbol.as_str())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end_matches('\n')
            .to_owned()
    }

    pub fn cell(&self, x: u16, y: u16) -> &CellSnapshot {
        assert!(x < self.width, "x coordinate {x} exceeds {}", self.width);
        assert!(y < self.height, "y coordinate {y} exceeds {}", self.height);
        &self.cells[usize::from(y) * usize::from(self.width) + usize::from(x)]
    }

    /// Deterministic whole-buffer fingerprint covering characters, color,
    /// modifiers, dimensions, and cursor state. Text goldens stay readable;
    /// this catches invisible style or stale-cell regressions as well.
    pub fn stable_fingerprint(&self) -> String {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let mut feed = |bytes: &[u8]| {
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        feed(&self.width.to_le_bytes());
        feed(&self.height.to_le_bytes());
        for cell in &self.cells {
            feed(cell.symbol.as_bytes());
            feed(format!("{:?}", cell.foreground).as_bytes());
            feed(format!("{:?}", cell.background).as_bytes());
            feed(&cell.modifiers.to_le_bytes());
        }
        feed(format!("{:?}", self.cursor).as_bytes());
        format!("{hash:016x}")
    }

    #[track_caller]
    pub fn assert_contains(&self, expected: &str) {
        let text = self.text();
        assert!(
            text.contains(expected),
            "buffer did not contain {expected:?}\n\n{text}"
        );
    }

    #[track_caller]
    pub fn assert_text_eq(&self, expected: &str) {
        let actual = self.text();
        let expected = expected.trim_end_matches('\n');
        assert_eq!(actual, expected, "rendered buffer text changed");
    }
}
