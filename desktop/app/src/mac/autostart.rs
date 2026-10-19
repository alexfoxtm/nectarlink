// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS login item / autostart via LaunchAgents.

use std::{io, path::Path};

const LABEL: &str = "com.thtbee.nectarlink";
const PLIST_NAME: &str = "com.thtbee.nectarlink.plist";

pub fn is_enabled() -> bool {
    dirs::home_dir().is_some_and(|dir| dir.join("Library/LaunchAgents").join(PLIST_NAME).exists())
}

pub fn is_installed(_exe: &Path) -> bool {
    is_enabled()
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    if !enabled {
        // Removal uses the home-directory plist, not the executable path.
        return set(false, Path::new(""));
    }
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    set(true, &exe)
}

pub fn set(enabled: bool, exe: &Path) -> Result<(), String> {
    let Some(home) = dirs::home_dir() else {
        return Err("no home directory; can't update the login item".to_string());
    };
    let agents = home.join("Library/LaunchAgents");
    let plist = agents.join(PLIST_NAME);
    if enabled {
        let content = launch_agent_plist(exe)?;
        std::fs::create_dir_all(&agents).map_err(|err| err.to_string())?;
        std::fs::write(&plist, content).map_err(|err| err.to_string())
    } else {
        remove_login_item(&plist)
    }
}

fn remove_login_item(plist: &Path) -> Result<(), String> {
    match std::fs::remove_file(plist) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}

/// Plist body for `exe`. The path is XML text, never interpolated raw:
/// `Path::display` would write a lossy path and still report success.
fn launch_agent_plist(exe: &Path) -> Result<String, String> {
    let exe = xml_text(executable_path(exe)?)?;
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#
    ))
}

fn executable_path(exe: &Path) -> Result<&str, String> {
    let text = exe.to_str().ok_or("executable path is not valid Unicode")?;
    if text.is_empty() { Err("executable path is empty".to_string()) } else { Ok(text) }
}

fn xml_text(value: &str) -> Result<String, String> {
    let mut encoded = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => encoded.push_str("&amp;"),
            '<' => encoded.push_str("&lt;"),
            '>' => encoded.push_str("&gt;"),
            '"' => encoded.push_str("&quot;"),
            '\'' => encoded.push_str("&apos;"),
            ch if is_xml_char(ch) => encoded.push(ch),
            ch => {
                return Err(format!(
                    "executable path contains XML-illegal character U+{:04X}",
                    u32::from(ch)
                ));
            }
        }
    }
    Ok(encoded)
}

/// XML 1.0 Char production, excluding the characters escaped above.
fn is_xml_char(ch: char) -> bool {
    matches!(ch, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&ch)
        || ('\u{E000}'..='\u{FFFD}').contains(&ch)
        || ('\u{10000}'..='\u{10FFFF}').contains(&ch)
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

    use super::*;

    #[test]
    fn launch_agent_plist_escapes_the_executable_path() {
        let exe = Path::new("/Applications/Nectar & Link <mac> \"'app\".app/Contents/MacOS/nectarlink");
        let plist = launch_agent_plist(exe).expect("a Unicode path with XML metacharacters encodes");
        let escaped =
            "/Applications/Nectar &amp; Link &lt;mac&gt; &quot;&apos;app&quot;.app/Contents/MacOS/nectarlink";
        assert!(plist.contains(&format!("<string>{escaped}</string>")));
        assert!(!plist.contains("--minimized"), "no tray on macOS, so a login launch must show the window");
        assert!(plist.contains("<string>com.thtbee.nectarlink</string>"));
        assert!(!plist.contains("Nectar & Link"));
        assert!(!plist.contains("<mac>"));
    }

    #[test]
    fn launch_agent_plist_rejects_a_path_that_cannot_be_xml() {
        let control = Path::new("/Applications/Nectar\u{0001}Link.app");
        assert!(
            launch_agent_plist(control).is_err(),
            "an illegal XML character must not be written as success"
        );

        let non_utf8 = PathBuf::from(OsString::from_vec(b"/Applications/Nectar\xff.app".to_vec()));
        assert!(
            launch_agent_plist(&non_utf8).is_err(),
            "a lossy display path must not be written as success"
        );
    }
}
