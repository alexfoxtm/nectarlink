// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS speaker volume, mute, and state.

pub fn speaker_state() -> Option<(u8, bool)> {
    let output = std::process::Command::new("osascript")
        .args(["-e", "output volume of (get volume settings)", "-e", "output muted of (get volume settings)"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines();
    let vol: u8 = lines.next()?.trim().parse().ok()?;
    let muted: bool = lines.next()?.trim().parse().unwrap_or(false);
    Some((vol, muted))
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
