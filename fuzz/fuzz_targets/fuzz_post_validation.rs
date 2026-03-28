#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Try to deserialize arbitrary bytes as a Post.
    if let Ok(post) = serde_json::from_slice::<ephemera_post::Post>(data) {
        // If deserialization succeeds, run validation.
        // This must not panic regardless of content.
        let _ = ephemera_post::validate_post(&post);
    }
});
