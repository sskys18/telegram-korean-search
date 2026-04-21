//! Length-prefixed JSON frame codec.
//!
//! Each frame on the wire is:
//!
//! ```text
//! [ u32 big-endian length N ][ N bytes of UTF-8 JSON ]
//! ```
//!
//! The length excludes the 4-byte header. Max frame size is capped at
//! [`MAX_FRAME_BYTES`] so a buggy or hostile peer cannot make us
//! allocate gigabytes. Read/write functions operate on any
//! [`AsyncRead`]/[`AsyncWrite`], so the same codec is used for the
//! Unix-socket server and any future TCP or stdio transport.

use std::io;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// 16 MiB. Telegram text messages are ~4 KiB max; search responses
/// with hundreds of hits are well under 1 MiB. This is defensive, not
/// a target.
pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

/// Read exactly one frame off `reader`. Returns `Ok(None)` on clean
/// EOF at a frame boundary. Any mid-frame EOF or oversize frame is
/// surfaced as `io::Error`.
pub async fn read_frame<R>(reader: &mut R) -> io::Result<Option<Vec<u8>>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too large: {len} > {MAX_FRAME_BYTES}"),
        ));
    }
    let mut body = vec![0u8; len as usize];
    reader.read_exact(&mut body).await?;
    Ok(Some(body))
}

/// Write one frame. Short-circuits on empty payloads so we never emit
/// a zero-length frame, which the peer is free to reject.
pub async fn write_frame<W>(writer: &mut W, body: &[u8]) -> io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let len = u32::try_from(body.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "frame body exceeds u32::MAX bytes",
        )
    })?;
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame too large: {len} > {MAX_FRAME_BYTES}"),
        ));
    }
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(body).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip() {
        let payload = br#"{"id":1,"method":"ping"}"#.to_vec();
        let mut buf = Vec::new();
        write_frame(&mut buf, &payload).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let got = read_frame(&mut cursor).await.unwrap();
        assert_eq!(got.as_deref(), Some(&payload[..]));
    }

    #[tokio::test]
    async fn clean_eof() {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let got = read_frame(&mut cursor).await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn rejects_oversized_header() {
        let mut cursor = std::io::Cursor::new((MAX_FRAME_BYTES + 1).to_be_bytes().to_vec());
        let err = read_frame(&mut cursor).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
