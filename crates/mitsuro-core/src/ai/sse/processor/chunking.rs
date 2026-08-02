use bytes::Bytes;
use tracing::{debug, warn};

use super::super::events::SseParser;
use super::{SseStreamProcessor, MAX_PARTIAL_LINE_SIZE};

impl SseStreamProcessor {
    fn set_partial_line(&mut self, line: &str) {
        self.partial_line.clear();

        if line.len() <= MAX_PARTIAL_LINE_SIZE {
            self.partial_line.push_str(line);
            return;
        }

        warn!(
            "Partial line exceeds {} bytes, truncating to prevent OOM",
            MAX_PARTIAL_LINE_SIZE
        );

        let mut boundary = MAX_PARTIAL_LINE_SIZE;
        while boundary > 0 && !line.is_char_boundary(boundary) {
            boundary -= 1;
        }
        self.partial_line.push_str(&line[..boundary]);
    }

    /// Process a chunk of bytes from the SSE stream.
    pub async fn process_chunk<P: SseParser>(
        &mut self,
        bytes: Bytes,
        parser: &P,
    ) -> anyhow::Result<()> {
        self.bytes_received += bytes.len();
        let text = String::from_utf8_lossy(&bytes);

        let combined = if self.partial_line.is_empty() {
            text.into_owned()
        } else {
            let mut combined = std::mem::take(&mut self.partial_line);
            combined.push_str(&text);
            combined
        };

        debug!(
            "SSE chunk received: {} bytes (total: {} bytes)",
            bytes.len(),
            self.bytes_received
        );

        let has_trailing_newline = combined.ends_with('\n');
        let mut lines_iter = combined.lines().peekable();

        while let Some(line) = lines_iter.next() {
            if lines_iter.peek().is_none() && !has_trailing_newline {
                self.set_partial_line(line);
                break;
            }

            if line.is_empty() || line.starts_with(':') {
                continue;
            }

            let data = line
                .strip_prefix("data: ")
                .or_else(|| line.strip_prefix("data:"));
            if let Some(data) = data {
                self.process_sse_data(data, parser).await?;
            }
        }

        Ok(())
    }
}
