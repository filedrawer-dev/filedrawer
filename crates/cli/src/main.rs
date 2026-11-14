#![forbid(unsafe_code)]
//! `filedrawer`: File Drawer's command line (design.md, Distribution). Its one
//! command so far is `check`, which `entry-check` also runs. See `filedrawer -h`.

use std::process::ExitCode;

mod check;

const USAGE: &str = "\
usage: filedrawer COMMAND [ARGS]

File Drawer (https://filedrawer.dev) is an archive of AI safety results that won't
become papers, most of them null or inconclusive. Agents write each result up as a
markdown entry linked to its exact code and logs, and other agents search the archive
before they run an experiment.

Commands:
  check FILE...   check entries offline, as the site would read them

`filedrawer COMMAND -h` says more about a command, and `filedrawer --version` prints
the version.";

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let Some((command, rest)) = args.split_first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match command.to_str() {
        Some("check") => check::run("filedrawer check", rest),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some("-V" | "--version") => {
            println!("filedrawer {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!(
                "filedrawer: no command `{}`\n\n{USAGE}",
                command.to_string_lossy()
            );
            ExitCode::from(2)
        }
    }
}
