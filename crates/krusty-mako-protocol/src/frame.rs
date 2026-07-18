use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{FrameError, MAX_FRAME_BYTES};

/// Read one length-prefixed JSON frame. A clean EOF before any header byte is
/// represented as `Ok(None)`; every partial frame is an explicit error.
pub async fn read_frame<R, T>(reader: &mut R) -> Result<Option<T>, FrameError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut header = [0_u8; 4];
    let header_read = read_into(reader, &mut header).await?;
    if header_read == 0 {
        return Ok(None);
    }
    if header_read != header.len() {
        return Err(FrameError::TruncatedHeader {
            received: header_read,
        });
    }

    let payload_len = u32::from_be_bytes(header) as usize;
    if payload_len == 0 {
        return Err(FrameError::ZeroLength);
    }
    if payload_len > MAX_FRAME_BYTES {
        return Err(FrameError::Oversized {
            actual: payload_len,
            maximum: MAX_FRAME_BYTES,
        });
    }

    let mut payload = vec![0_u8; payload_len];
    let payload_read = read_into(reader, &mut payload).await?;
    if payload_read != payload_len {
        return Err(FrameError::TruncatedPayload {
            expected: payload_len,
            received: payload_read,
        });
    }

    serde_json::from_slice(&payload)
        .map(Some)
        .map_err(FrameError::Decode)
}

/// Serialize and write one bounded length-prefixed JSON frame.
pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(value).map_err(FrameError::Encode)?;
    if payload.is_empty() {
        return Err(FrameError::ZeroLength);
    }
    if payload.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Oversized {
            actual: payload.len(),
            maximum: MAX_FRAME_BYTES,
        });
    }

    let payload_len = u32::try_from(payload.len()).map_err(|_| FrameError::Oversized {
        actual: payload.len(),
        maximum: MAX_FRAME_BYTES,
    })?;
    writer.write_all(&payload_len.to_be_bytes()).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_into<R>(reader: &mut R, buffer: &mut [u8]) -> Result<usize, std::io::Error>
where
    R: AsyncRead + Unpin,
{
    let mut offset = 0;
    while offset < buffer.len() {
        let read = reader.read(&mut buffer[offset..]).await?;
        if read == 0 {
            break;
        }
        offset += read;
    }
    Ok(offset)
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use tokio::io::{duplex, AsyncWriteExt};

    use super::*;

    #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
    struct Example {
        value: String,
    }

    #[tokio::test]
    async fn round_trips_a_json_frame() {
        let (mut writer, mut reader) = duplex(1024);
        let expected = Example {
            value: "hello".to_string(),
        };

        write_frame(&mut writer, &expected).await.unwrap();
        let actual: Example = read_frame(&mut reader).await.unwrap().unwrap();
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn clean_eof_is_not_a_truncated_frame() {
        let (writer, mut reader) = duplex(16);
        drop(writer);
        let frame: Option<Example> = read_frame(&mut reader).await.unwrap();
        assert!(frame.is_none());
    }

    #[tokio::test]
    async fn rejects_an_oversized_declared_frame_before_allocating() {
        let (mut writer, mut reader) = duplex(16);
        writer
            .write_all(&((MAX_FRAME_BYTES as u32) + 1).to_be_bytes())
            .await
            .unwrap();

        let error = read_frame::<_, Example>(&mut reader).await.unwrap_err();
        assert!(matches!(error, FrameError::Oversized { .. }));
    }

    #[tokio::test]
    async fn rejects_an_oversized_outbound_frame() {
        let (mut writer, _reader) = duplex(16);
        let oversized = Example {
            value: "x".repeat(MAX_FRAME_BYTES),
        };
        let error = write_frame(&mut writer, &oversized).await.unwrap_err();
        assert!(matches!(error, FrameError::Oversized { .. }));
    }

    #[tokio::test]
    async fn reports_a_truncated_header() {
        let (mut writer, mut reader) = duplex(16);
        writer.write_all(&[0, 0]).await.unwrap();
        writer.shutdown().await.unwrap();

        let error = read_frame::<_, Example>(&mut reader).await.unwrap_err();
        assert!(matches!(error, FrameError::TruncatedHeader { received: 2 }));
    }

    #[tokio::test]
    async fn reports_a_truncated_payload() {
        let (mut writer, mut reader) = duplex(32);
        writer.write_all(&12_u32.to_be_bytes()).await.unwrap();
        writer.write_all(b"short").await.unwrap();
        writer.shutdown().await.unwrap();

        let error = read_frame::<_, Example>(&mut reader).await.unwrap_err();
        assert!(matches!(
            error,
            FrameError::TruncatedPayload {
                expected: 12,
                received: 5
            }
        ));
    }

    #[tokio::test]
    async fn rejects_invalid_json() {
        let (mut writer, mut reader) = duplex(32);
        writer.write_all(&4_u32.to_be_bytes()).await.unwrap();
        writer.write_all(b"nope").await.unwrap();

        let error = read_frame::<_, Example>(&mut reader).await.unwrap_err();
        assert!(matches!(error, FrameError::Decode(_)));
    }
}
