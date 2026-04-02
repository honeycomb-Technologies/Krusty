//! Text wrapping logic

use unicode_width::UnicodeWidthChar;

use super::MultiLineInput;

impl MultiLineInput {
    pub(super) fn get_wrapped_lines_impl(&self) -> Vec<String> {
        // Check cache first
        if let Some(cached) = self.wrapped_lines_cache.borrow().as_ref() {
            return cached.clone();
        }

        // Compute wrapped lines
        let lines = self.compute_wrapped_lines();

        // Store in cache
        *self.wrapped_lines_cache.borrow_mut() = Some(lines.clone());

        lines
    }

    /// Compute wrapped lines without caching (used by cache population)
    fn compute_wrapped_lines(&self) -> Vec<String> {
        if self.content.is_empty() {
            return vec![String::new()];
        }

        let mut lines = Vec::new();
        let mut current_line = String::new();
        let mut current_width = 0;

        for ch in self.content.chars() {
            if ch == '\n' {
                lines.push(current_line);
                current_line = String::new();
                current_width = 0;
            } else {
                let ch_width = UnicodeWidthChar::width(ch).unwrap_or(1);

                // Break line if adding this character would exceed width
                if current_width + ch_width > self.width as usize && !current_line.is_empty() {
                    lines.push(current_line);
                    current_line = String::new();
                    current_width = 0;
                }

                current_line.push(ch);
                current_width += ch_width;
            }
        }

        // Push the last line
        if !current_line.is_empty() {
            lines.push(current_line);
        }

        if lines.is_empty() {
            lines.push(String::new());
        }

        lines
    }
}
