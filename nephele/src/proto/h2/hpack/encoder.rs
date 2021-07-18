use bytes::{buf::Limit, BufMut, BytesMut};
use http::header::{HeaderName, HeaderValue};

use crate::proto::h2::hpack::table::{Index, Table};
use crate::proto::h2::hpack::{huffman, Header};

type DstBuf<'a> = Limit<&'a mut BytesMut>;

#[derive(Debug)]
pub struct Encoder {
    table: Table,
    size_update: Option<SizeUpdate>,
}

#[derive(Debug)]
pub enum Encode {
    Full,
    Partial(EncodeState),
}

#[derive(Debug)]
pub struct EncodeState {
    index: Index,
    value: Option<HeaderValue>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum EncoderError {
    BufferOverflow,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum SizeUpdate {
    One(usize),
    Two(usize, usize),
}

impl Encoder {
    pub fn new(max_size: usize, capacity: usize) -> Encoder {
        Encoder {
            table: Table::new(max_size, capacity),
            size_update: None,
        }
    }

    pub fn update_max_size(&mut self, val: usize) {
        match self.size_update {
            Some(SizeUpdate::One(old)) => {
                if val > old {
                    if old > self.table.max_size() {
                        self.size_update = Some(SizeUpdate::One(val));
                    } else {
                        self.size_update = Some(SizeUpdate::Two(old, val));
                    }
                } else {
                    self.size_update = Some(SizeUpdate::One(val));
                }
            }
            Some(SizeUpdate::Two(min, _)) => {
                if val < min {
                    self.size_update = Some(SizeUpdate::One(val));
                } else {
                    self.size_update = Some(SizeUpdate::Two(min, val));
                }
            }
            None => {
                if val != self.table.max_size() {
                    self.size_update = Some(SizeUpdate::One(val));
                }
            }
        }
    }

    pub fn encode<I>(
        &mut self,
        resume: Option<EncodeState>,
        headers: &mut I,
        dst: &mut DstBuf<'_>,
    ) -> Encode
    where
        I: Iterator<Item = Header<Option<HeaderName>>>,
    {
        let span = tracing::trace_span!("hpack::encode");
        let _e = span.enter();

        let pos = position(dst);
        tracing::trace!(pos, "encoding at");

        if let Err(e) = self.encode_size_updates(dst) {
            if e == EncoderError::BufferOverflow {
                rewind(dst, pos);
            }

            unreachable!("encode_size_updates errored");
        }

        let mut last_index = None;

        if let Some(resume) = resume {
            let pos = position(dst);

            let res = match resume.value {
                Some(ref value) => self.encode_header_without_name(&resume.index, value, dst),
                None => self.encode_header(&resume.index, dst),
            };

            if res.is_err() {
                rewind(dst, pos);
                return Encode::Partial(resume);
            }
            last_index = Some(resume.index);
        }

        for header in headers {
            let pos = position(dst);

            match header.reify() {
                Ok(header) => {
                    let index = self.table.index(header);
                    let res = self.encode_header(&index, dst);

                    if res.is_err() {
                        rewind(dst, pos);
                        return Encode::Partial(EncodeState { index, value: None });
                    }

                    last_index = Some(index);
                }
                Err(value) => {
                    let res = self.encode_header_without_name(
                        last_index.as_ref().unwrap_or_else(|| {
                            panic!("encoding header without name, but no previous index to use for name");
                        }),
                        &value,
                        dst,
                    );

                    if res.is_err() {
                        rewind(dst, pos);
                        return Encode::Partial(EncodeState {
                            index: last_index.unwrap(),
                            value: Some(value),
                        });
                    }
                }
            };
        }

        Encode::Full
    }

    fn encode_size_updates(&mut self, dst: &mut DstBuf<'_>) -> Result<(), EncoderError> {
        match self.size_update.take() {
            Some(SizeUpdate::One(val)) => {
                self.table.resize(val);
                encode_size_update(val, dst)?;
            }
            Some(SizeUpdate::Two(min, max)) => {
                self.table.resize(min);
                self.table.resize(max);
                encode_size_update(min, dst)?;
                encode_size_update(max, dst)?;
            }
            None => {}
        }

        Ok(())
    }

    fn encode_header(&mut self, index: &Index, dst: &mut DstBuf<'_>) -> Result<(), EncoderError> {
        match *index {
            Index::Indexed(idx, _) => {
                encode_int(idx, 7, 0x80, dst)?;
            }
            Index::Name(idx, _) => {
                let header = self.table.resolve(&index);

                encode_not_indexed(idx, header.value_slice(), header.is_sensitive(), dst)?;
            }
            Index::Inserted(_) => {
                let header = self.table.resolve(&index);

                assert!(!header.is_sensitive());

                if !dst.has_remaining_mut() {
                    return Err(EncoderError::BufferOverflow);
                }

                dst.put_u8(0b0100_0000);

                encode_str(header.name().as_slice(), dst)?;
                encode_str(header.value_slice(), dst)?;
            }
            Index::InsertedValue(idx, _) => {
                let header = self.table.resolve(&index);

                assert!(!header.is_sensitive());

                encode_int(idx, 6, 0b0100_0000, dst)?;
                encode_str(header.value_slice(), dst)?;
            }
            Index::NotIndexed(_) => {
                let header = self.table.resolve(&index);

                encode_not_indexed2(
                    header.name().as_slice(),
                    header.value_slice(),
                    header.is_sensitive(),
                    dst,
                )?;
            }
        }

        Ok(())
    }

    fn encode_header_without_name(
        &mut self,
        last: &Index,
        value: &HeaderValue,
        dst: &mut DstBuf<'_>,
    ) -> Result<(), EncoderError> {
        match *last {
            Index::Indexed(..)
            | Index::Name(..)
            | Index::Inserted(..)
            | Index::InsertedValue(..) => {
                let idx = self.table.resolve_idx(last);

                encode_not_indexed(idx, value.as_ref(), value.is_sensitive(), dst)?;
            }
            Index::NotIndexed(_) => {
                let last = self.table.resolve(last);

                encode_not_indexed2(
                    last.name().as_slice(),
                    value.as_ref(),
                    value.is_sensitive(),
                    dst,
                )?;
            }
        }

        Ok(())
    }
}

impl Default for Encoder {
    fn default() -> Encoder {
        Encoder::new(4096, 0)
    }
}

fn encode_size_update<B: BufMut>(val: usize, dst: &mut B) -> Result<(), EncoderError> {
    encode_int(val, 5, 0b0010_0000, dst)
}

fn encode_not_indexed(
    name: usize,
    value: &[u8],
    sensitive: bool,
    dst: &mut DstBuf<'_>,
) -> Result<(), EncoderError> {
    if sensitive {
        encode_int(name, 4, 0b10000, dst)?;
    } else {
        encode_int(name, 4, 0, dst)?;
    }

    encode_str(value, dst)?;
    Ok(())
}

fn encode_not_indexed2(
    name: &[u8],
    value: &[u8],
    sensitive: bool,
    dst: &mut DstBuf<'_>,
) -> Result<(), EncoderError> {
    if !dst.has_remaining_mut() {
        return Err(EncoderError::BufferOverflow);
    }

    if sensitive {
        dst.put_u8(0b10000);
    } else {
        dst.put_u8(0);
    }

    encode_str(name, dst)?;
    encode_str(value, dst)?;
    Ok(())
}

fn encode_str(val: &[u8], dst: &mut DstBuf<'_>) -> Result<(), EncoderError> {
    if !dst.has_remaining_mut() {
        return Err(EncoderError::BufferOverflow);
    }

    if !val.is_empty() {
        let idx = position(dst);

        dst.put_u8(0);

        huffman::encode(val, dst)?;

        let huff_len = position(dst) - (idx + 1);

        if encode_int_one_byte(huff_len, 7) {
            dst.get_mut()[idx] = 0x80 | huff_len as u8;
        } else {
            const PLACEHOLDER_LEN: usize = 8;
            let mut buf = [0u8; PLACEHOLDER_LEN];

            let head_len = {
                let mut head_dst = &mut buf[..];
                encode_int(huff_len, 7, 0x80, &mut head_dst)?;
                PLACEHOLDER_LEN - head_dst.remaining_mut()
            };

            if dst.remaining_mut() < head_len {
                return Err(EncoderError::BufferOverflow);
            }

            dst.put_slice(&buf[1..head_len]);

            let written = dst.get_mut();
            for i in 0..huff_len {
                let src_i = idx + 1 + (huff_len - (i + 1));
                let dst_i = idx + head_len + (huff_len - (i + 1));
                written[dst_i] = written[src_i];
            }

            for i in 0..head_len {
                written[idx + i] = buf[i];
            }
        }
    } else {
        dst.put_u8(0);
    }

    Ok(())
}

fn encode_int<B: BufMut>(
    mut value: usize,
    prefix_bits: usize,
    first_byte: u8,
    dst: &mut B,
) -> Result<(), EncoderError> {
    let mut rem = dst.remaining_mut();

    if rem == 0 {
        return Err(EncoderError::BufferOverflow);
    }

    if encode_int_one_byte(value, prefix_bits) {
        dst.put_u8(first_byte | value as u8);
        return Ok(());
    }

    let low = (1 << prefix_bits) - 1;

    value -= low;

    if value > 0x0fff_ffff {
        panic!("value out of range");
    }

    dst.put_u8(first_byte | low as u8);
    rem -= 1;

    while value >= 128 {
        if rem == 0 {
            return Err(EncoderError::BufferOverflow);
        }

        dst.put_u8(0b1000_0000 | value as u8);
        rem -= 1;

        value >>= 7;
    }

    if rem == 0 {
        return Err(EncoderError::BufferOverflow);
    }

    dst.put_u8(value as u8);

    Ok(())
}

fn encode_int_one_byte(value: usize, prefix_bits: usize) -> bool {
    value < (1 << prefix_bits) - 1
}

fn position(buf: &DstBuf<'_>) -> usize {
    buf.get_ref().len()
}

fn rewind(buf: &mut DstBuf<'_>, pos: usize) {
    buf.get_mut().truncate(pos);
}
