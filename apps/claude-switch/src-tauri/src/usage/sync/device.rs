//! Which Mac this is, to iCloud sync: a random id kept in the login
//! Keychain, so the Mac goes on writing the same file after claude-switch is
//! reinstalled or its data reset, and the name the Mac goes by.

use std::process::{Command, Output};

use uuid::Uuid;

/// The Keychain item holding the id, as OpenUsage names its own
/// (`KeychainICloudDeviceIDStore`).
const SERVICE: &str = "app.claude-switch.icloud-sync-device-id.v1";
/// What `security` exits with when there is no such item.
const NOT_FOUND: i32 = 44;

#[derive(Debug, Clone)]
pub struct Device {
    pub id: String,
    pub name: String,
}

/// This Mac, its id made and saved the first time. Fails when the Keychain
/// can't keep the id: a new one each launch would leave a file behind each
/// time.
pub fn this_mac() -> Result<Device, String> {
    let id = match stored_id()? {
        Some(id) => id,
        None => {
            let id = Uuid::new_v4().to_string();
            store_id(&id)?;
            id
        }
    };
    // The name Sharing settings give the Mac, as Finder shows it.
    let name = Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Mac".to_owned());
    Ok(Device { id, name })
}

fn stored_id() -> Result<Option<String>, String> {
    let output = security("find-generic-password", &["-s", SERVICE, "-w"])
        .map_err(|err| format!("couldn't ask the Keychain for this Mac's sync id: {err}"))?;
    if output.status.code() == Some(NOT_FOUND) {
        return Ok(None);
    }
    if !output.status.success() {
        return Err(format!(
            "the Keychain didn't hand over this Mac's sync id (security exited with {})",
            output.status.code().unwrap_or(-1)
        ));
    }
    // Something else in the item is replaced, as a missing id would be.
    Ok(
        Uuid::parse_str(String::from_utf8_lossy(&output.stdout).trim())
            .ok()
            .map(|id| id.to_string()),
    )
}

fn store_id(id: &str) -> Result<(), String> {
    let output = security("add-generic-password", &["-U", "-s", SERVICE, "-w", id])
        .map_err(|err| format!("couldn't save this Mac's sync id in the Keychain: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "couldn't save this Mac's sync id in the Keychain (security exited with {})",
        output.status.code().unwrap_or(-1)
    ))
}

/// Runs `/usr/bin/security <subcommand>` on the current user's item, as
/// `credentials.rs` asks it.
fn security(subcommand: &str, args: &[&str]) -> std::io::Result<Output> {
    let mut command = Command::new("/usr/bin/security");
    command.arg(subcommand);
    if let Ok(user) = std::env::var("USER") {
        command.args(["-a", &user]);
    }
    command.args(args).output()
}
