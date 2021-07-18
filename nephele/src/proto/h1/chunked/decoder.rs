use byte_pool::{Block, BytePool};
use cynthia::future::swap::AsyncRead;
use cynthia::future::swap::{self};
use std::fmt;
use std::future::Future;
use std::ops::Range;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use crate::common::http_types::trailers::{Sender, Trailers};

const INITIAL_CAPACITY: usize = 1024 * 4;
const MAX_CAPACITY: usize = 512 * 1024 * 1024;

lazy_static::lazy_static! {
    pub(crate) static ref POOL: Arc<BytePool> = Arc::new(BytePool::new());
}

#[derive(Debug)]
pub struct ChunkedDecoder<R: AsyncRead> {
    inner: R,
    buffer: Block<'static>,
    current: Range<usize>,
    initial_decode: bool,
    state: State,
    trailer_sender: Option<Sender>,
}

impl<R: AsyncRead> ChunkedDecoder<R> {
    pub(crate) fn new(inner: R, trailer_sender: Sender) -> Self {
        ChunkedDecoder {
            inner,
            buffer: POOL.alloc(INITIAL_CAPACITY),
            current: Range { start: 0, end: 0 },
            initial_decode: false,
            state: State::Init,
            trailer_sender: Some(trailer_sender),
        }
    }
}

impl<R: AsyncRead + Unpin> ChunkedDecoder<R> {
    fn poll_read_chunk(
        &mut self,
        cx: &mut Context<'_>,
        buffer: Block<'static>,
        pos: &Range<usize>,
        buf: &mut [u8],
        current: u64,
        len: u64,
    ) -> swap::Result<DecodeResult> {
        let mut new_pos = pos.clone();
        let remaining = (len - current) as usize;
        let to_read = std::cmp::min(remaining, buf.len());

        let mut new_current = current;
        let mut read = 0;

        if new_pos.len() > 0 {
            let to_read_buf = std::cmp::min(to_read, pos.len());
            buf[..to_read_buf].copy_from_slice(&buffer[new_pos.start..new_pos.start + to_read_buf]);

            if new_pos.start + to_read_buf == new_pos.end {
                new_pos = 0..0
            } else {
                new_pos.start += to_read_buf;
            }
            new_current += to_read_buf as u64;
            read += to_read_buf;

            let new_state = if new_current == len {
                State::ChunkEnd
            } else {
                State::Chunk(new_current, len)
            };

            return Ok(DecodeResult::Some {
                read,
                new_state: Some(new_state),
                new_pos,
                buffer,
                pending: false,
            });
        }

        match Pin::new(&mut self.inner).poll_read(cx, &mut buf[read..read + to_read]) {
            Poll::Ready(val) => {
                let n = val?;
                new_current += n as u64;
                read += n;
                let new_state = if new_current == len {
                    State::ChunkEnd
                } else if n == 0 {
                    State::Done
                } else {
                    State::Chunk(new_current, len)
                };

                Ok(DecodeResult::Some {
                    read,
                    new_state: Some(new_state),
                    new_pos,
                    buffer,
                    pending: false,
                })
            }
            Poll::Pending => Ok(DecodeResult::Some {
                read: 0,
                new_state: Some(State::Chunk(new_current, len)),
                new_pos,
                buffer,
                pending: true,
            }),
        }
    }

    fn poll_read_inner(
        &mut self,
        cx: &mut Context<'_>,
        buffer: Block<'static>,
        pos: &Range<usize>,
        buf: &mut [u8],
    ) -> swap::Result<DecodeResult> {
        match self.state {
            State::Init => decode_init(buffer, pos),
            State::Chunk(current, len) => self.poll_read_chunk(cx, buffer, pos, buf, current, len),
            State::ChunkEnd => decode_chunk_end(buffer, pos),
            State::Trailer => decode_trailer(buffer, pos),
            State::TrailerDone(ref mut headers) => {
                let headers = std::mem::replace(headers, Trailers::new());
                let sender = self.trailer_sender.take();
                let sender =
                    sender.expect("invalid chunked state, tried sending multiple trailers");

                let fut = Box::pin(sender.send(headers));
                Ok(DecodeResult::Some {
                    read: 0,
                    new_state: Some(State::TrailerSending(fut)),
                    new_pos: pos.clone(),
                    buffer,
                    pending: false,
                })
            }
            State::TrailerSending(ref mut fut) => {
                match Pin::new(fut).poll(cx) {
                    Poll::Ready(_) => {}
                    Poll::Pending => {
                        return Ok(DecodeResult::Some {
                            read: 0,
                            new_state: None,
                            new_pos: pos.clone(),
                            buffer,
                            pending: true,
                        });
                    }
                }

                Ok(DecodeResult::Some {
                    read: 0,
                    new_state: Some(State::Done),
                    new_pos: pos.clone(),
                    buffer,
                    pending: false,
                })
            }
            State::Done => Ok(DecodeResult::Some {
                read: 0,
                new_state: Some(State::Done),
                new_pos: pos.clone(),
                buffer,
                pending: false,
            }),
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for ChunkedDecoder<R> {
    #[allow(missing_doc_code_examples)]
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<swap::Result<usize>> {
        let this = &mut *self;

        if let State::Done = this.state {
            return Poll::Ready(Ok(0));
        }

        let mut n = std::mem::replace(&mut this.current, 0..0);
        let buffer = std::mem::replace(&mut this.buffer, POOL.alloc(INITIAL_CAPACITY));
        let mut needs_read = !matches!(this.state, State::Chunk(_, _));

        let mut buffer = if n.len() > 0 && this.initial_decode {
            match this.poll_read_inner(cx, buffer, &n, buf)? {
                DecodeResult::Some {
                    read,
                    buffer,
                    new_pos,
                    new_state,
                    pending,
                } => {
                    this.current = new_pos.clone();
                    if let Some(state) = new_state {
                        this.state = state;
                    }

                    if pending {
                        this.buffer = buffer;
                        return Poll::Pending;
                    }

                    if let State::Done = this.state {
                        this.buffer = buffer;
                        return Poll::Ready(Ok(read));
                    }

                    if read > 0 {
                        this.buffer = buffer;
                        return Poll::Ready(Ok(read));
                    }

                    n = new_pos;
                    needs_read = false;
                    buffer
                }
                DecodeResult::None(buffer) => buffer,
            }
        } else {
            buffer
        };

        loop {
            if n.len() >= buffer.capacity() {
                if buffer.capacity() + 1024 <= MAX_CAPACITY {
                    buffer.realloc(buffer.capacity() + 1024);
                } else {
                    this.buffer = buffer;
                    this.current = n;
                    return Poll::Ready(Err(swap::Error::new(
                        swap::ErrorKind::Other,
                        "incoming data too large",
                    )));
                }
            }

            if needs_read {
                let bytes_read = match Pin::new(&mut this.inner).poll_read(cx, &mut buffer[n.end..])
                {
                    Poll::Ready(result) => result?,
                    Poll::Pending => {
                        this.initial_decode = false;
                        this.buffer = buffer;
                        this.current = n;
                        return Poll::Pending;
                    }
                };
                match (bytes_read, &this.state) {
                    (0, State::Done) => {}
                    (0, _) => {
                        this.state = State::Done;
                    }
                    _ => {}
                }
                n.end += bytes_read;
            }
            match this.poll_read_inner(cx, buffer, &n, buf)? {
                DecodeResult::Some {
                    read,
                    buffer: new_buffer,
                    new_pos,
                    new_state,
                    pending,
                } => {
                    this.initial_decode = true;
                    if let Some(state) = new_state {
                        this.state = state;
                    }
                    this.current = new_pos.clone();
                    n = new_pos;

                    if let State::Done = this.state {
                        this.buffer = new_buffer;
                        return Poll::Ready(Ok(read));
                    }

                    if read > 0 {
                        this.buffer = new_buffer;
                        return Poll::Ready(Ok(read));
                    }

                    if pending {
                        this.buffer = new_buffer;
                        return Poll::Pending;
                    }

                    buffer = new_buffer;
                    needs_read = false;
                    continue;
                }
                DecodeResult::None(buf) => {
                    buffer = buf;

                    if this.buffer.is_empty() || n.start == 0 && n.end == 0 {
                        this.initial_decode = false;
                        this.buffer = buffer;
                        this.current = n;

                        return Poll::Ready(Ok(0));
                    } else {
                        needs_read = true;
                    }
                }
            }
        }
    }
}

enum DecodeResult {
    Some {
        read: usize,
        buffer: Block<'static>,
        new_pos: Range<usize>,
        new_state: Option<State>,
        pending: bool,
    },
    None(Block<'static>),
}

enum State {
    Init,
    Chunk(u64, u64),
    ChunkEnd,
    Trailer,
    TrailerDone(Trailers),
    TrailerSending(Pin<Box<dyn Future<Output = ()> + 'static + Send + Sync>>),
    Done,
}
impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use State::*;
        match self {
            Init => write!(f, "State::Init"),
            Chunk(a, b) => write!(f, "State::Chunk({}, {})", a, b),
            ChunkEnd => write!(f, "State::ChunkEnd"),
            Trailer => write!(f, "State::Trailer"),
            TrailerDone(trailers) => write!(f, "State::TrailerDone({:?})", &trailers),
            TrailerSending(_) => write!(f, "State::TrailerSending"),
            Done => write!(f, "State::Done"),
        }
    }
}

impl fmt::Debug for DecodeResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeResult::Some {
                read,
                buffer,
                new_pos,
                new_state,
                pending,
            } => f
                .debug_struct("DecodeResult::Some")
                .field("read", read)
                .field("block", &buffer.len())
                .field("new_pos", new_pos)
                .field("new_state", new_state)
                .field("pending", pending)
                .finish(),
            DecodeResult::None(block) => write!(f, "DecodeResult::None({})", block.len()),
        }
    }
}

fn decode_init(buffer: Block<'static>, pos: &Range<usize>) -> swap::Result<DecodeResult> {
    use httparse::Status;
    match httparse::parse_chunk_size(&buffer[pos.start..pos.end]) {
        Ok(Status::Complete((used, chunk_len))) => {
            let new_pos = Range {
                start: pos.start + used,
                end: pos.end,
            };

            let new_state = if chunk_len == 0 {
                State::Trailer
            } else {
                State::Chunk(0, chunk_len)
            };

            Ok(DecodeResult::Some {
                read: 0,
                buffer,
                new_pos,
                new_state: Some(new_state),
                pending: false,
            })
        }
        Ok(Status::Partial) => Ok(DecodeResult::None(buffer)),
        Err(err) => Err(swap::Error::new(swap::ErrorKind::Other, err.to_string())),
    }
}

fn decode_chunk_end(buffer: Block<'static>, pos: &Range<usize>) -> swap::Result<DecodeResult> {
    if pos.len() < 2 {
        return Ok(DecodeResult::None(buffer));
    }

    if &buffer[pos.start..pos.start + 2] == b"\r\n" {
        return Ok(DecodeResult::Some {
            read: 0,
            buffer,
            new_pos: Range {
                start: pos.start + 2,
                end: pos.end,
            },
            new_state: Some(State::Init),
            pending: false,
        });
    }

    Err(swap::Error::from(swap::ErrorKind::InvalidData))
}

fn decode_trailer(buffer: Block<'static>, pos: &Range<usize>) -> swap::Result<DecodeResult> {
    use httparse::Status;

    let mut headers = [httparse::EMPTY_HEADER; 16];

    match httparse::parse_headers(&buffer[pos.start..pos.end], &mut headers) {
        Ok(Status::Complete((used, headers))) => {
            let mut trailers = Trailers::new();
            for header in headers {
                trailers.insert(header.name, String::from_utf8_lossy(header.value).as_ref());
            }

            Ok(DecodeResult::Some {
                read: 0,
                buffer,
                new_state: Some(State::TrailerDone(trailers)),
                new_pos: Range {
                    start: pos.start + used,
                    end: pos.end,
                },
                pending: false,
            })
        }
        Ok(Status::Partial) => Ok(DecodeResult::None(buffer)),
        Err(err) => Err(swap::Error::new(swap::ErrorKind::Other, err.to_string())),
    }
}
