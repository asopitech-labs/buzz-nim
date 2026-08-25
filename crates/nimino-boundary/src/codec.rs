use std::io::ErrorKind;

use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, thiserror::Error)]
pub(crate) enum CodecError {
    #[error("stream closed before the next frame")]
    Eof,
    #[error("frame was truncated")]
    Truncated,
    #[error("frame length {actual} exceeds limit {limit}")]
    FrameTooLarge { actual: usize, limit: usize },
    #[error("frame I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

pub(crate) async fn write_json_frame<W, T>(
    writer: &mut W,
    value: &T,
    max_frame_bytes: usize,
) -> Result<(), CodecError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(value)?;
    if payload.len() > max_frame_bytes {
        return Err(CodecError::FrameTooLarge {
            actual: payload.len(),
            limit: max_frame_bytes,
        });
    }
    let length = u32::try_from(payload.len()).map_err(|_| CodecError::FrameTooLarge {
        actual: payload.len(),
        limit: max_frame_bytes,
    })?;
    writer.write_all(&length.to_be_bytes()).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

pub(crate) async fn read_json_frame<R, T>(
    reader: &mut R,
    max_frame_bytes: usize,
) -> Result<T, CodecError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut first = [0_u8; 1];
    match reader.read_exact(&mut first).await {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::UnexpectedEof => return Err(CodecError::Eof),
        Err(error) => return Err(CodecError::Io(error)),
    }
    let mut rest = [0_u8; 3];
    match reader.read_exact(&mut rest).await {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::UnexpectedEof => {
            return Err(CodecError::Truncated)
        }
        Err(error) => return Err(CodecError::Io(error)),
    }
    let length = u32::from_be_bytes([first[0], rest[0], rest[1], rest[2]]) as usize;
    if length > max_frame_bytes {
        return Err(CodecError::FrameTooLarge {
            actual: length,
            limit: max_frame_bytes,
        });
    }
    let mut payload = vec![0_u8; length];
    match reader.read_exact(&mut payload).await {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::UnexpectedEof => {
            return Err(CodecError::Truncated)
        }
        Err(error) => return Err(CodecError::Io(error)),
    }
    Ok(serde_json::from_slice(&payload)?)
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use tokio::io::{duplex, AsyncWriteExt};

    use super::{read_json_frame, write_json_frame, CodecError};

    #[tokio::test]
    async fn frame_round_trip_preserves_json() {
        let (mut writer, mut reader) = duplex(1024);
        let send =
            tokio::spawn(
                async move { write_json_frame(&mut writer, &json!({"ok": true}), 1024).await },
            );
        let value: Value = read_json_frame(&mut reader, 1024).await.expect("frame");
        send.await.expect("writer task").expect("write");
        assert_eq!(value, json!({"ok": true}));
    }

    #[tokio::test]
    async fn oversized_and_truncated_frames_are_distinct() {
        let (mut writer, mut reader) = duplex(32);
        writer
            .write_all(&100_u32.to_be_bytes())
            .await
            .expect("header");
        let oversized = read_json_frame::<_, Value>(&mut reader, 16)
            .await
            .expect_err("oversized");
        assert!(matches!(oversized, CodecError::FrameTooLarge { .. }));

        let (mut writer, mut reader) = duplex(32);
        writer.write_all(&[0, 0]).await.expect("partial header");
        drop(writer);
        let truncated = read_json_frame::<_, Value>(&mut reader, 16)
            .await
            .expect_err("truncated");
        assert!(matches!(truncated, CodecError::Truncated));
    }
}
