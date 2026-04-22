fn main() {
    // UniFFI proc-macro mode: `uniffi::setup_scaffolding!("seoyu")` in
    // lib.rs emits the scaffolding at compile time. No UDL file and no
    // manual generation step needed here; this build.rs exists only so
    // cargo knows to re-run on changes to the exported surface.
    println!("cargo:rerun-if-changed=src/uniffi_api.rs");
}
