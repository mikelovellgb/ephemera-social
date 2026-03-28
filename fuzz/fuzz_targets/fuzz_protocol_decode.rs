#![no_main]
use libfuzzer_sys::fuzz_target;
use bytes::BytesMut;

fuzz_target!(|data: &[u8]| {
    // Try to decode as a length-prefixed protocol envelope.
    let mut buf = BytesMut::from(data);
    let _ = ephemera_protocol::codec::decode(&mut buf);

    // Also try bare (no length prefix) decoding.
    let _ = ephemera_protocol::codec::decode_bare(data);
});
