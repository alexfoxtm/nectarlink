// SPDX-License-Identifier: GPL-3.0-or-later
//! Wi-Fi on macOS: not supported. Listing saved networks or reading SSIDs needs
//! CoreWLAN and Location Services permission, which this app doesn't ask for, so
//! there are no networks to pick and joining one always reports an error.

/// Where a saved network stands right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    NotInRange,
    // Only the cross-platform hotspot tests build these on macOS.
    #[allow(dead_code)]
    InRange,
    #[allow(dead_code)]
    Connected,
}

pub fn saved_networks() -> Vec<String> {
    Vec::new()
}

pub fn scan() {}

pub fn reach(_profile: &str) -> Reach {
    Reach::NotInRange
}

pub fn connect(_profile: &str) -> Result<(), String> {
    Err("Joining Wi-Fi networks isn't supported on this Mac".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joining_is_refused_never_faked() {
        assert!(saved_networks().is_empty());
        assert_eq!(reach("Pixel hotspot"), Reach::NotInRange);
        assert_eq!(
            connect("Pixel hotspot"),
            Err("Joining Wi-Fi networks isn't supported on this Mac".into())
        );
    }
}
