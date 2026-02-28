use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

const CONTENT_LENGTH: &str = "Content-Length: ";

/// Read a single DAP message from the stream.
///
/// DAP uses HTTP-style framing: `Content-Length: N\r\n\r\n<body>`.
/// Returns `None` on EOF.
pub async fn read_message<R: AsyncRead + Unpin>(reader: &mut BufReader<R>) -> Option<Vec<u8>> {
    let mut content_length: Option<usize> = None;
    let mut header_line = String::new();

    // Parse headers until empty line
    loop {
        header_line.clear();
        let n = reader.read_line(&mut header_line).await.ok()?;
        if n == 0 {
            return None; // EOF
        }

        let trimmed = header_line.trim();
        if trimmed.is_empty() {
            break; // End of headers
        }

        if let Some(val) = trimmed.strip_prefix(CONTENT_LENGTH) {
            content_length = val.parse().ok();
        }
    }

    let length = content_length?;
    let mut body = vec![0u8; length];
    tokio::io::AsyncReadExt::read_exact(reader, &mut body)
        .await
        .ok()?;
    Some(body)
}

/// Write a DAP message with Content-Length framing.
pub async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    body: &[u8],
) -> std::io::Result<()> {
    let header = format!("{CONTENT_LENGTH}{}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(body).await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[tokio::test]
    async fn read_single_message() {
        let data = b"Content-Length: 13\r\n\r\n{\"seq\": 1234}";
        let mut reader = BufReader::new(Cursor::new(data));
        let msg = read_message(&mut reader).await.unwrap();
        assert_eq!(msg, b"{\"seq\": 1234}");
    }

    #[tokio::test]
    async fn read_eof_returns_none() {
        let data = b"";
        let mut reader = BufReader::new(Cursor::new(data));
        assert!(read_message(&mut reader).await.is_none());
    }

    #[tokio::test]
    async fn read_two_messages() {
        let data = b"Content-Length: 3\r\n\r\nabcContent-Length: 2\r\n\r\nxy";
        let mut reader = BufReader::new(Cursor::new(data));
        assert_eq!(read_message(&mut reader).await.unwrap(), b"abc");
        assert_eq!(read_message(&mut reader).await.unwrap(), b"xy");
    }

    #[tokio::test]
    async fn read_ignores_unknown_headers() {
        let data = b"X-Custom: foo\r\nContent-Length: 2\r\n\r\nok";
        let mut reader = BufReader::new(Cursor::new(data));
        assert_eq!(read_message(&mut reader).await.unwrap(), b"ok");
    }

    #[tokio::test]
    async fn write_message_format() {
        let mut buf = Vec::new();
        write_message(&mut buf, b"{\"test\": true}").await.unwrap();
        assert_eq!(buf, b"Content-Length: 14\r\n\r\n{\"test\": true}");
    }

    #[tokio::test]
    async fn roundtrip() {
        let original = b"{\"seq\": 42, \"type\": \"request\"}";
        let mut buf = Vec::new();
        write_message(&mut buf, original).await.unwrap();

        let mut reader = BufReader::new(Cursor::new(buf));
        let msg = read_message(&mut reader).await.unwrap();
        assert_eq!(msg, original);
    }
}
