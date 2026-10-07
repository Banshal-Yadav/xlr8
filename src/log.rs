use std::fmt;
use std::io::Write;
use std::sync::OnceLock;

fn max_level() -> u8 {
    static MAX: OnceLock<u8> = OnceLock::new();
    *MAX.get_or_init(|| match std::env::var("RUST_LOG").as_deref() {
        Ok("off") => 0,
        Ok("warn") => 1,
        Ok("debug") => 3,
        _ => 2,
    })
}

pub fn write(level: &str, lvl: u8, args: fmt::Arguments) {
    if lvl > max_level() {
        return;
    }
    let mut e = std::io::stderr().lock();
    let _ = writeln!(e, "[{level}] {args}");
}

#[macro_export]
macro_rules! info {
    ($($a:tt)*) => { $crate::log::write("INFO", 2, format_args!($($a)*)) };
}

#[macro_export]
macro_rules! warn {
    ($($a:tt)*) => { $crate::log::write("WARN", 1, format_args!($($a)*)) };
}

/// Per-request phase timings: off unless `RUST_LOG=debug` — a stderr write in
/// the blocking worker lands on the response path.
#[macro_export]
macro_rules! debug {
    ($($a:tt)*) => { $crate::log::write("DEBUG", 3, format_args!($($a)*)) };
}
