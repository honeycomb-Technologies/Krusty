//! Selected-row-following windows for compact lists.

use std::ops::Range;

pub fn visible_range(total: usize, selected: usize, capacity: usize) -> Range<usize> {
    let capacity = capacity.min(total);
    if capacity == 0 {
        return 0..0;
    }
    let selected = selected.min(total.saturating_sub(1));
    let start = selected
        .saturating_sub(capacity / 2)
        .min(total.saturating_sub(capacity));
    start..start.saturating_add(capacity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_row_remains_visible_across_the_entire_list() {
        assert_eq!(visible_range(14, 0, 5), 0..5);
        assert_eq!(visible_range(14, 7, 5), 5..10);
        assert_eq!(visible_range(14, 13, 5), 9..14);
        assert_eq!(visible_range(2, 1, 5), 0..2);
    }
}
