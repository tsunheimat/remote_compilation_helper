//! Platform-independent bounded output capture for child processes.
//!
//! Both remote transports and the local updater use the same byte limit and
//! failure semantics. Keeping this separate avoids importing Unix-only SSH
//! machinery for a Windows executable-version probe.

use tokio::io::{AsyncRead, AsyncReadExt};

pub(crate) async fn read_bounded_output_stream<R>(
    mut reader: R,
    max_bytes: usize,
) -> std::io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::new();
    // Keep the I/O buffer on the heap: an inline array multiplies through
    // nested async futures and previously overflowed remote-execution stacks.
    let mut chunk = vec![0_u8; 16 * 1024];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            return Ok(output);
        }
        let next_len = output.len().checked_add(read).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "command output size overflow",
            )
        })?;
        if next_len > max_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("command output exceeded {max_bytes} bytes"),
            ));
        }
        output.extend_from_slice(&chunk[..read]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error, ErrorKind};
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::{AsyncWriteExt, ReadBuf};

    #[tokio::test]
    async fn empty_exact_and_excessive_output_preserve_the_byte_contract() {
        assert_eq!(read_bounded_output_stream(&b""[..], 0).await.unwrap(), b"");
        assert_eq!(
            read_bounded_output_stream(&b"A\0\xffB"[..], 4)
                .await
                .unwrap(),
            b"A\0\xffB"
        );
        for (bytes, limit) in [(&b"a"[..], 0), (&b"abcde"[..], 4)] {
            let error = read_bounded_output_stream(bytes, limit).await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidData);
        }
    }

    #[tokio::test]
    async fn fragmented_binary_output_is_not_truncated_or_reordered() {
        let expected = vec![137_u8; 40_000];
        let payload = expected.clone();
        let (mut writer, reader) = tokio::io::duplex(7);
        let producer = tokio::spawn(async move {
            writer.write_all(&payload).await.unwrap();
            writer.shutdown().await.unwrap();
        });
        let captured = read_bounded_output_stream(reader, expected.len())
            .await
            .unwrap();
        producer.await.unwrap();
        assert_eq!(captured, expected);
    }

    struct BrokenReader;

    impl AsyncRead for BrokenReader {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Err(Error::new(
                ErrorKind::ConnectionReset,
                "injected read failure",
            )))
        }
    }

    #[tokio::test]
    async fn reader_failure_is_not_reported_as_successful_empty_output() {
        let error = read_bounded_output_stream(BrokenReader, 4096)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ConnectionReset);
        assert_eq!(error.to_string(), "injected read failure");
    }

    #[test]
    fn the_read_buffer_does_not_live_inside_the_future() {
        let future = read_bounded_output_stream(&b""[..], 4096);
        assert!(std::mem::size_of_val(&future) < 1024);
    }
}
