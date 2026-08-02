use tokio::sync::mpsc;

use crate::ai::streaming::StreamPart;

/// Create standard streaming channels with buffer processing
pub fn create_streaming_channels() -> (
    mpsc::UnboundedSender<StreamPart>,
    mpsc::UnboundedReceiver<StreamPart>,
) {
    let (tx, rx) = mpsc::unbounded_channel::<StreamPart>();
    (tx, rx)
}
