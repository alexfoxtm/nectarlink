// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS speaker volume, mute, and state.

fn parse_speaker_state(text: &str) -> Option<(u8, bool)> {
    let mut parts = text.split_whitespace();
    let vol: u8 = parts.next()?.parse().ok()?;
    let muted: bool = parts.next()?.parse().unwrap_or(false);
    Some((vol, muted))
}

pub fn speaker_state() -> Option<(u8, bool)> {
    let output = std::process::Command::new("osascript")
        .args([
            "-e",
            "set v to get volume settings",
            "-e",
            "(output volume of v as string) & \" \" & (output muted of v as string)",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_speaker_state(&String::from_utf8_lossy(&output.stdout))
}

pub fn set_speaker_volume(level: u8) -> Result<(), String> {
    let script = format!("set volume output volume {level}");
    std::process::Command::new("osascript").args(["-e", &script]).output().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_speaker_mute(muted: bool) -> Result<(), String> {
    let script = format!("set volume output muted {muted}");
    std::process::Command::new("osascript").args(["-e", &script]).output().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn toggle_speaker_mute() -> Result<bool, String> {
    let (_, muted) = speaker_state().unwrap_or((50, false));
    let next = !muted;
    set_speaker_mute(next)?;
    Ok(next)
}

pub fn speaker_volume_up() -> Result<(u8, bool), String> {
    let (vol, muted) = speaker_state().unwrap_or((50, false));
    let next = vol.saturating_add(5).min(100);
    set_speaker_volume(next)?;
    Ok((next, muted))
}

pub fn speaker_volume_down() -> Result<(u8, bool), String> {
    let (vol, muted) = speaker_state().unwrap_or((50, false));
    let next = vol.saturating_sub(5);
    set_speaker_volume(next)?;
    Ok((next, muted))
}

pub fn mic_muted() -> Option<bool> {
    let output = std::process::Command::new("osascript")
        .args(["-e", "input volume of (get volume settings)"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let vol: u8 = text.trim().parse().ok()?;
    Some(vol == 0)
}

pub fn toggle_mic_mute() -> Result<bool, String> {
    let muted = mic_muted().unwrap_or(false);
    let next = !muted;
    let vol = if next { 0 } else { 75 };
    let script = format!("set volume input volume {vol}");
    std::process::Command::new("osascript").args(["-e", &script]).output().map_err(|e| e.to_string())?;
    Ok(next)
}

pub fn output_devices() -> Vec<nectarlink_core::AudioOutputDevice> {
    vec![nectarlink_core::AudioOutputDevice {
        id: "default".into(),
        name: "Default macOS Output".into(),
        is_default: true,
    }]
}

pub fn watch(_changed: fn()) {}

pub fn start(_on_changed: impl Fn() + Send + Sync + 'static) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaker_state_parses_volume_and_mute() {
        assert_eq!(parse_speaker_state("25 false"), Some((25, false)));
        assert_eq!(parse_speaker_state("100 true\n"), Some((100, true)));
        assert_eq!(parse_speaker_state("0 false"), Some((0, false)));
        assert_eq!(parse_speaker_state(""), None);
        assert_eq!(parse_speaker_state("invalid"), None);
    }

    #[test]
    fn query_live_speaker_state() {
        let state = speaker_state();
        assert!(state.is_some());
        let (vol, _muted) = state.unwrap();
        assert!(vol <= 100);
    }
}
