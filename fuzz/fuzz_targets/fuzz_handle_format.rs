#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Convert to a string (lossy) and try handle validation.
    // Must not panic regardless of input.
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = ephemera_social::validate_handle_format(s);
    }
    // Also try with lossy conversion for truly arbitrary bytes.
    let lossy = String::from_utf8_lossy(data);
    let _ = ephemera_social::validate_handle_format(&lossy);
});
