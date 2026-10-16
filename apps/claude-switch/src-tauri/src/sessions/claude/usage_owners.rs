//! Which home each Claude session transcript belongs to, for counting what
//! the session cost: the one the Sessions list shows it under
//! ([`owned_by`]). Ownership turns on transcript ids and desktop records
//! only, so the transcripts are listed, not read.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::desktop::{self, read_records};
use super::ownership::{owned_by, HomeScan};
use super::transcript::TranscriptSummary;
use crate::sessions::Home;

/// The owning home's id of each session transcript of `homes`
/// (`<config>/projects/<slug>/<id>.jsonl`) that a home owns. Of homes
/// owning the same transcript, as two desktop apps with a record of one
/// session can, the first in `homes` has it, so its cost counts once.
pub fn transcript_owners(homes: &[Home]) -> HashMap<PathBuf, String> {
    desktop::forget_gone();
    let scans: Vec<HomeScan> = homes
        .iter()
        .map(|home| HomeScan {
            home_id: home.id.clone(),
            transcripts: listed_transcripts(&home.config_dir),
            records: read_records(&home.gui_data_dir),
        })
        .collect();
    let mut owners = HashMap::new();
    for home in homes {
        for owned in owned_by(&home.id, &scans) {
            for held in owned.claimed_transcripts {
                owners
                    .entry(held.summary.path)
                    .or_insert_with(|| home.id.clone());
            }
        }
    }
    owners
}

/// The session transcripts in `config_dir`, one level under each project
/// folder, known by their file names alone.
fn listed_transcripts(config_dir: &Path) -> Vec<TranscriptSummary> {
    let Ok(projects) = fs::read_dir(config_dir.join("projects")) else {
        return Vec::new();
    };
    let mut transcripts = Vec::new();
    for project in projects.flatten() {
        let Ok(files) = fs::read_dir(project.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "jsonl")
            {
                continue;
            }
            let Some(session_id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let last_used_at = file
                .metadata()
                .and_then(|metadata| metadata.modified())
                .map(DateTime::<Utc>::from)
                .unwrap_or_default();
            transcripts.push(TranscriptSummary {
                session_id: session_id.to_owned(),
                path: path.clone(),
                cwd: None,
                custom_title: None,
                ai_title: None,
                first_prompt: None,
                last_prompt: None,
                last_used_at,
                plan_slugs: BTreeSet::new(),
            });
        }
    }
    transcripts
}
