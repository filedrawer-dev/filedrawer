#![forbid(unsafe_code)]
//! Checks File Drawer entries before submission, offline, with the analyzer the server
//! uses. See `entry-check -h`.

use std::process::ExitCode;

use filedrawer_analyzer::{Analysis, CardContext, analyze, render_card};

const USAGE: &str = "\
usage: entry-check FILE...

Reads each entry the way the File Drawer server does (docs/entry-format.md) and prints
whether the server would accept it, the reasons it would refuse it, its card as
analyzer v1 renders it, the primary Results row as the analyzer read it, and every
warning. Nothing is fetched: link hosts and pins are checked by the URL's shape, and
the card shows placeholders such as <id> and <author> for what only the server knows.

Exit status: 0 when every entry would be accepted (warnings allowed), 1 when any would
be refused, 2 when a file can't be read or no file is given.";

fn main() -> ExitCode {
    let paths: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if paths.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if paths.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let mut refused = false;
    let mut unreadable = false;
    for (index, path) in paths.iter().enumerate() {
        if index > 0 {
            println!();
        }
        let name = path.to_string_lossy();
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let analysis = analyze(&text);
                refused |= analysis.refused();
                print!("{}", report(&name, &analysis));
            }
            Err(error) => {
                eprintln!("entry-check: can't read {name}: {error}");
                unreadable = true;
            }
        }
    }
    match (unreadable, refused) {
        (true, _) => ExitCode::from(2),
        (false, true) => ExitCode::from(1),
        (false, false) => ExitCode::SUCCESS,
    }
}

/// The report for one entry: verdict, refusals, card, primary row, warnings.
fn report(path: &str, analysis: &Analysis) -> String {
    let warnings = match analysis.warnings.len() {
        1 => "1 warning".to_string(),
        n => format!("{n} warnings"),
    };
    let verdict = if analysis.refused() {
        "refused"
    } else {
        "accepted"
    };
    let mut out = format!("{path}: {verdict}, {warnings}\n");
    for refusal in &analysis.refusals {
        out.push_str(&format!("refused: {refusal}\n"));
    }
    out.push_str("card:\n");
    for line in render_card(analysis, &CardContext::default()).lines() {
        out.push_str(&format!("  {line}\n"));
    }
    out.push_str(&format!(
        "primary row: {}\n",
        analysis.primary_row_summary()
    ));
    for warning in &analysis.warnings {
        out.push_str(&format!("warning: {warning}\n"));
    }
    out
}
