use bytes::BytesMut;
use std::io;

pub trait Encoder<Item> {
    type Error: From<io::Error>;

    fn encode(&mut self, item: Item, dst: &mut BytesMut) -> Result<(), Self::Error>;
}
