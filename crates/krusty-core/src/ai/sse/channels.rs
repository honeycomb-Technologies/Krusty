use tokio::sync::mpsc;

use crate::ai::streaming::StreamPart;

/// Create standard streaming channels with buffer processing
pub fn create_streaming_channels() -> (
    mpsc::UnboundedSender<StreamPart>,
    mpsc::UnboundedReceiver<StreamPart>,
    mpsc::UnboundedSender<String>,
    mpsc::UnboundedReceiver<String>,
) {
    let (tx, rx) = mpsc::unbounded_channel::<StreamPart>();
    let (buffer_tx, buffer_rx) = mpsc::unbounded_channel::<String>();
    (tx, rx, buffer_tx, buffer_rx)
}

/// Spawn a task to convert buffered text into StreamParts
pub fn spawn_buffer_processor(
    mut buffer_rx: mpsc::UnboundedReceiver<String>,
    tx: mpsc::UnboundedSender<StreamPart>,
) {
    tokio::spawn(async move {
        while let Some(text) = buffer_rx.recv().await {
            let _ = tx.send(StreamPart::TextDelta { delta: text });
        }
    });
}
