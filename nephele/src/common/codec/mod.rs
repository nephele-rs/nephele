mod bytes_codec;
pub use bytes_codec::BytesCodec;

mod decoder;
pub use decoder::Decoder;

mod encoder;
pub use encoder::Encoder;

mod framed_impl;
#[allow(unused_imports)]
pub(crate) use framed_impl::{FramedImpl, RWFrames, ReadFrame, WriteFrame};

mod framed;
pub use framed::{Framed, FramedParts};

mod framed_read;
pub use framed_read::FramedRead;

mod framed_write;
pub use self::framed_write::FramedWrite;

pub mod length_delimited;
pub use self::length_delimited::{LengthDelimitedCodec, LengthDelimitedCodecError};

mod lines_codec;
pub use self::lines_codec::{LinesCodec, LinesCodecError};

mod util {
    use cynthia::future::swap::{self, AsyncRead, AsyncWrite};

    use bytes::{Buf, BufMut};
    use futures_core::ready;

    #[allow(unused_imports)]
    use std::mem::MaybeUninit;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    #[cfg_attr(not(feature = "io"), allow(unreachable_pub))]
    pub fn poll_read_buf<T: AsyncRead, B: BufMut>(
        io: Pin<&mut T>,
        cx: &mut Context<'_>,
        buf: &mut B,
    ) -> Poll<swap::Result<usize>> {
        if !buf.has_remaining_mut() {
            return Poll::Ready(Ok(0));
        }

        let n = {
            let dst = buf.chunk_mut();
            let dst = unsafe { &mut *(dst as *mut _ as *mut [u8]) };

            let nn = ready!(io.poll_read(cx, dst)?);
            nn
        };

        unsafe {
            buf.advance_mut(n);
        }

        Poll::Ready(Ok(n))
    }

    #[cfg_attr(not(feature = "io"), allow(unreachable_pub))]
    pub fn poll_write_buf<T: AsyncWrite, B: Buf>(
        io: Pin<&mut T>,
        cx: &mut Context<'_>,
        buf: &mut B,
    ) -> Poll<swap::Result<usize>> {
        if !buf.has_remaining() {
            return Poll::Ready(Ok(0));
        }

        /*
        const MAX_BUFS: usize = 64;
        let n = if io.is_write_vectored() {
            let mut slices = [IoSlice::new(&[]); MAX_BUFS];
            let cnt = buf.chunks_vectored(&mut slices);
            ready!(io.poll_write_vectored(cx, &slices[..cnt]))?
        } else {
            ready!(io.poll_write(cx, buf.chunk()))?
        };
        */

        let n = ready!(io.poll_write(cx, buf.chunk()))?;

        buf.advance(n);

        Poll::Ready(Ok(n))
    }
}
