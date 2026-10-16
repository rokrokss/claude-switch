//! claude-switch's own log, for when the menu bar or usage misbehave:
//! `~/Library/Logs/claude-switch/claude-switch.log`, moved aside to `.1.log`
//! once past 10 MB. Lines below the level chosen in Settings are dropped.
//! Tokens and the home folder never reach it.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

use chrono::Local;

use crate::settings::LogLevel;

const MAX_BYTES: u64 = 10 * 1024 * 1024;
/// A word this long of token characters is taken for a token.
const TOKEN_MIN_LEN: usize = 32;

static LEVEL: AtomicU8 = AtomicU8::new(rank(LogLevel::Info));
static WRITING: Mutex<()> = Mutex::new(());

const fn rank(level: LogLevel) -> u8 {
    match level {
        LogLevel::Error => 0,
        LogLevel::Warning => 1,
        LogLevel::Info => 2,
        LogLevel::Debug => 3,
    }
}

/// Sets the level below which lines are dropped.
pub fn set_level(level: LogLevel) {
    LEVEL.store(rank(level), Ordering::Relaxed);
}

/// Where the log is written, for Settings to reveal.
#[tauri::command]
pub fn log_file() -> Option<String> {
    path().map(|path| path.display().to_string())
}

/// Where the log is written.
pub fn path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join("Library/Logs/claude-switch/claude-switch.log"))
}

pub fn error(tag: &str, message: &str) {
    write(LogLevel::Error, tag, message);
}

pub fn warn(tag: &str, message: &str) {
    write(LogLevel::Warning, tag, message);
}

pub fn info(tag: &str, message: &str) {
    write(LogLevel::Info, tag, message);
}

fn write(level: LogLevel, tag: &str, message: &str) {
    if rank(level) > LEVEL.load(Ordering::Relaxed) {
        return;
    }
    let line = format!(
        "{} {:<5} [{tag}] {}\n",
        Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        format!("{level:?}").to_uppercase(),
        redact(
            message,
            dirs::home_dir().as_deref().and_then(|home| home.to_str())
        ),
    );
    eprint!("claude-switch: {line}");
    let Some(path) = path() else {
        return;
    };
    let _writing = WRITING.lock().unwrap();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > MAX_BYTES) {
        let _ = fs::rename(&path, path.with_extension("1.log"));
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}

/// `message` with the home folder shown as `~`, and anything that looks
/// like a token cut to its first and last four characters.
fn redact(message: &str, home: Option<&str>) -> String {
    let message = match home.filter(|home| home.len() > 1) {
        Some(home) => message.replace(home, "~"),
        None => message.to_owned(),
    };
    let mut out = String::with_capacity(message.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.len() >= TOKEN_MIN_LEN || word.starts_with("sk-ant-") {
            out.push_str(&word[..4.min(word.len())]);
            out.push_str("...");
            out.push_str(&word[word.len().saturating_sub(4)..]);
        } else {
            out.push_str(word);
        }
        word.clear();
    };
    for character in message.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
            word.push(character);
        } else {
            flush(&mut word, &mut out);
            out.push(character);
        }
    }
    flush(&mut word, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_cut_short() {
        let token = "sk-ant-oat01-abcdefghijklmnopqrstuvwxyz0123456789";
        assert_eq!(
            redact(&format!("Bearer {token} failed"), None),
            "Bearer sk-a...6789 failed"
        );
        assert_eq!(redact("sk-ant-short", None), "sk-a...hort");
    }

    #[test]
    fn the_home_folder_reads_as_a_tilde() {
        assert_eq!(
            redact("/Users/ada/.claude/projects", Some("/Users/ada")),
            "~/.claude/projects"
        );
    }

    #[test]
    fn ordinary_words_are_kept() {
        assert_eq!(
            redact("fetched 3 profiles in 1.2s", None),
            "fetched 3 profiles in 1.2s"
        );
    }

    #[test]
    fn levels_rank_from_errors_to_debug() {
        assert!(rank(LogLevel::Error) < rank(LogLevel::Warning));
        assert!(rank(LogLevel::Info) < rank(LogLevel::Debug));
    }
}
