#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Fuzz image validation with random bytes.
    // Must not panic regardless of input.
    let _ = ephemera_media::validation::validate_media(data);

    // Also fuzz video validation.
    let _ = ephemera_media::video_validation::validate_video(data);

    // Fuzz format detection.
    let _ = ephemera_media::validation::detect_format(data);
});
