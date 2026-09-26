fn main() {
    // Expose the crate version to the program as APP_VERSION (matches the
    // option_env!("APP_VERSION") lookups used throughout the source).
    println!("cargo:rustc-env=APP_VERSION={}", env!("CARGO_PKG_VERSION"));
}
