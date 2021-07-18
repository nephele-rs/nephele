mod table;

use bytes::{BufMut, BytesMut};

use self::table::{DECODE_TABLE, ENCODE_TABLE};
use crate::proto::h2::hpack::{DecoderError, EncoderError};

struct Decoder {
    state: usize,
    maybe_eos: bool,
}

const MAYBE_EOS: u8 = 1;
const DECODED: u8 = 2;
const ERROR: u8 = 4;

pub fn decode(src: &[u8], buf: &mut BytesMut) -> Result<BytesMut, DecoderError> {
    let mut decoder = Decoder::new();

    buf.reserve(src.len() << 1);

    for b in src {
        if let Some(b) = decoder.decode4(b >> 4)? {
            buf.put_u8(b);
        }

        if let Some(b) = decoder.decode4(b & 0xf)? {
            buf.put_u8(b);
        }
    }

    if !decoder.is_final() {
        return Err(DecoderError::InvalidHuffmanCode);
    }

    Ok(buf.split())
}

pub fn encode<B: BufMut>(src: &[u8], dst: &mut B) -> Result<(), EncoderError> {
    let mut bits: u64 = 0;
    let mut bits_left = 40;
    let mut rem = dst.remaining_mut();

    for &b in src {
        let (nbits, code) = ENCODE_TABLE[b as usize];

        bits |= code << (bits_left - nbits);
        bits_left -= nbits;

        while bits_left <= 32 {
            if rem == 0 {
                return Err(EncoderError::BufferOverflow);
            }

            dst.put_u8((bits >> 32) as u8);

            bits <<= 8;
            bits_left += 8;
            rem -= 1;
        }
    }

    if bits_left != 40 {
        if rem == 0 {
            return Err(EncoderError::BufferOverflow);
        }

        bits |= (1 << bits_left) - 1;
        dst.put_u8((bits >> 32) as u8);
    }

    Ok(())
}

impl Decoder {
    fn new() -> Decoder {
        Decoder {
            state: 0,
            maybe_eos: false,
        }
    }

    fn decode4(&mut self, input: u8) -> Result<Option<u8>, DecoderError> {
        let (next, byte, flags) = DECODE_TABLE[self.state][input as usize];

        if flags & ERROR == ERROR {
            return Err(DecoderError::InvalidHuffmanCode);
        }

        let mut ret = None;

        if flags & DECODED == DECODED {
            ret = Some(byte);
        }

        self.state = next;
        self.maybe_eos = flags & MAYBE_EOS == MAYBE_EOS;

        Ok(ret)
    }

    fn is_final(&self) -> bool {
        self.state == 0 || self.maybe_eos
    }
}
