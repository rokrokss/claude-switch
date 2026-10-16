// SPDX-License-Identifier: MIT

// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use claude_switch_lib::cli::{invocation, open_profile, usage, watch_clones, Invocation, HELP};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match invocation(&args) {
        Invocation::Gui => claude_switch_lib::run(),
        Invocation::OpenProfile(id) => {
            if let Err(message) = open_profile(id) {
                eprintln!("claude-switch: {message}");
                std::process::exit(1);
            }
        }
        Invocation::WatchClones => watch_clones(),
        Invocation::Usage(args) => std::process::exit(usage(&args)),
        Invocation::Help => print!("{HELP}"),
        Invocation::Version => println!("claude-switch {}", env!("CARGO_PKG_VERSION")),
        Invocation::Misuse(message) => {
            eprintln!("claude-switch: {message}");
            std::process::exit(2);
        }
    }
}
