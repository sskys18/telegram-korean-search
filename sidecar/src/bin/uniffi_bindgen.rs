//! Trivial wrapper so the Xcode build phase can run
//! `cargo run --bin uniffi-bindgen generate --library … --language swift …`
//! without depending on a globally-installed `uniffi-bindgen` binary.
//!
//! UniFFI in 0.29+ exposes `uniffi::uniffi_bindgen_main` for exactly
//! this use case.

fn main() {
    uniffi::uniffi_bindgen_main()
}
