// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS login item / autostart via LaunchAgents.

use std::path::Path;

pub fn is_enabled() -> bool {
    if let Some(dir) = dirs::home_dir() {
        let plist = dir.join("Library/LaunchAgents/com.thtbee.nectarlink.plist");
        plist.exists()
    } else {
        false
    }
}

pub fn is_installed(_exe: &Path) -> bool {
    is_enabled()
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    if let Some(dir) = dirs::home_dir() {
        let agents = dir.join("Library/LaunchAgents");
        let plist = agents.join("com.thtbee.nectarlink.plist");
        if enabled {
            let _ = std::fs::create_dir_all(&agents);
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let content = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.thtbee.nectarlink</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>--minimized</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>"#,
                exe.display()
            );
            std::fs::write(&plist, content).map_err(|e| e.to_string())?;
        } else if plist.exists() {
            let _ = std::fs::remove_file(&plist);
        }
    }
    Ok(())
}

pub fn set(enabled: bool, _exe: &Path) -> Result<(), String> {
    set_enabled(enabled)
}
