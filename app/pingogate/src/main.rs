//! PingoGate gateway binary — composition root.
//!
//! Full bootstrap (config load → listeners → signal handlers → `ArcSwap`
//! wiring) is assembled at task T016. This scaffold keeps the binary buildable
//! and prints the version for `--version`-style smoke checks.

fn main() {
    println!("pingogate {}", env!("CARGO_PKG_VERSION"));
}
