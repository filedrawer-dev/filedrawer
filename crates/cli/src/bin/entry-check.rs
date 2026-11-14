#![forbid(unsafe_code)]
//! `entry-check FILE...`: `filedrawer check` under the name the submit skill runs
//! (`cargo run -q --bin entry-check -- FILE`). See `entry-check -h`.

use std::process::ExitCode;

#[path = "../check.rs"]
mod check;

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    check::run("entry-check", &args)
}
