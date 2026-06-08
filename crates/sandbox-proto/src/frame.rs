//! Length-prefixed `postcard` framing over any async byte stream.

use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::ProtoError;

/// Hard cap on a single frame (64 MiB) — bounds guest/host memory and rejects a
/// corrupt or malicious length prefix before allocating.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Encode `msg` as postcard and write it as a `u32`-length-prefixed frame.
pub async fn write_frame<W, T>(w: &mut W, msg: &T) -> Result<(), ProtoError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = postcard::to_allocvec(msg).map_err(|e| ProtoError::Encode(e.to_string()))?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(ProtoError::FrameTooLarge(body.len()));
    }
    w.write_all(&(body.len() as u32).to_le_bytes()).await?;
    w.write_all(&body).await?;
    w.flush().await?;
    Ok(())
}

/// Read one length-prefixed postcard frame. Returns `Ok(None)` on a clean EOF at
/// a frame boundary (the peer closed the connection).
pub async fn read_frame<R, T>(r: &mut R) -> Result<Option<T>, ProtoError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(ProtoError::FrameTooLarge(len));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    let msg = postcard::from_bytes(&body).map_err(|e| ProtoError::Decode(e.to_string()))?;
    Ok(Some(msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{GuestRequest, GuestResponse, Language};

    #[tokio::test]
    async fn round_trips_a_request_over_a_pipe() {
        let (mut a, mut b) = tokio::io::duplex(256);
        let req = GuestRequest::Exec {
            exec_id: "e1".into(),
            language: Language::Python,
            code: "print(1)".into(),
            dependencies: vec!["requests".into()],
            stdin: None,
            env: vec![("K".into(), "V".into())],
            timeout_ms: Some(5000),
        };
        write_frame(&mut a, &req).await.unwrap();
        let got: GuestRequest = read_frame(&mut b).await.unwrap().unwrap();
        assert_eq!(got, req);
    }

    #[tokio::test]
    async fn multiple_frames_and_clean_eof() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        write_frame(&mut a, &GuestResponse::Pong).await.unwrap();
        write_frame(
            &mut a,
            &GuestResponse::ExecDone { exec_id: "e1".into(), exit_code: 0, timed_out: false },
        )
        .await
        .unwrap();
        drop(a); // close → reader sees EOF after the two frames

        assert_eq!(read_frame::<_, GuestResponse>(&mut b).await.unwrap(), Some(GuestResponse::Pong));
        assert_eq!(
            read_frame::<_, GuestResponse>(&mut b).await.unwrap(),
            Some(GuestResponse::ExecDone { exec_id: "e1".into(), exit_code: 0, timed_out: false })
        );
        // Clean EOF at a frame boundary → None, not an error.
        assert_eq!(read_frame::<_, GuestResponse>(&mut b).await.unwrap(), None);
    }

    #[tokio::test]
    async fn oversize_length_prefix_is_rejected() {
        let (mut a, mut b) = tokio::io::duplex(16);
        // Write a bogus length prefix larger than the cap, no body.
        let writer = tokio::spawn(async move {
            let huge = (MAX_FRAME_BYTES as u32) + 1;
            let _ = a.write_all(&huge.to_le_bytes()).await;
            let _ = a.flush().await;
            // keep `a` alive briefly
            a
        });
        let err = read_frame::<_, GuestResponse>(&mut b).await.unwrap_err();
        assert!(matches!(err, ProtoError::FrameTooLarge(_)));
        let _ = writer.await;
    }
}
