//! Box-drawing `mitsuro` wordmark.

/// Three-row line art (Unicode box drawing).
pub const MARK: &[&str] = &[
    "┌┬┐┬┌┬┐┌─┐┬ ┬┬─┐┌─┐",
    "││││ │ └─┐│ │├┬┘│ │",
    "┴ ┴┴ ┴ └─┘└─┘┴└─└─┘",
];

/// ASCII fallback approximating the same 19×3 geometry (spaces aligned).
pub const MARK_ASCII: &[&str] = &[
    "++++++++-++ ++-++-+",
    "|||| | +-+| |+++| |",
    "+ ++ + +-++-+++-+-+",
];

pub const MARK_WIDTH: u16 = 19;
pub const MARK_HEIGHT: u16 = 3;

/// Total non-space ink cells, used to pace the stroke-in reveal.
pub fn ink_cells(ascii: bool) -> usize {
    lines(ascii)
        .iter()
        .flat_map(|line| line.chars())
        .filter(|ch| *ch != ' ')
        .count()
}

pub fn lines(ascii: bool) -> &'static [&'static str] {
    if ascii {
        MARK_ASCII
    } else {
        MARK
    }
}

pub fn char_at(ascii: bool, y: usize, x: usize) -> char {
    lines(ascii)
        .get(y)
        .and_then(|row| row.chars().nth(x))
        .unwrap_or(' ')
}

/// Stroke order: left-to-right, top-to-bottom (natural reading order).
/// Returns the reveal index of the ink cell at (x,y), or `None` for empty.
pub fn stroke_index(ascii: bool, y: usize, x: usize) -> Option<usize> {
    let mut index = 0usize;
    for (row_y, line) in lines(ascii).iter().enumerate() {
        for (col_x, ch) in line.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            if row_y == y && col_x == x {
                return Some(index);
            }
            index += 1;
        }
    }
    None
}
