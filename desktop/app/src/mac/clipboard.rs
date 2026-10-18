// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS clipboard implementation: pbcopy / pbpaste / file handling.

use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

#[derive(Clone, PartialEq, Eq)]
pub enum Image {
    Png(Vec<u8>),
    Bitmap(Vec<u8>),
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, len) = match self {
            Image::Png(b) => ("Png", b.len()),
            Image::Bitmap(b) => ("Bitmap", b.len()),
        };
        f.debug_struct(kind).field("bytes", &len).finish()
    }
}

impl Image {
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        match self {
            Image::Png(png) => Ok(png.clone()),
            Image::Bitmap(b) => Ok(b.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clip {
    Text { text: String, html: Option<String>, rtf: Option<String> },
    Image(Image),
    Private,
    Empty,
}

type OnCopy = Box<dyn Fn(Clip) + Send + Sync>;
static ON_COPY: OnceLock<OnCopy> = OnceLock::new();
static LAST_TEXT: Mutex<Option<String>> = Mutex::new(None);

pub fn start(on_copy: impl Fn(Clip) + Send + Sync + 'static) {
    let _ = ON_COPY.set(Box::new(on_copy));
    std::thread::Builder::new()
        .name("mac-clipboard-watcher".into())
        .spawn(|| {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let clip = read();
                if let Clip::Text { text, .. } = &clip {
                    let mut last = LAST_TEXT.lock().unwrap_or_else(|e| e.into_inner());
                    if last.as_deref() != Some(text) {
                        *last = Some(text.clone());
                        drop(last);
                        if let Some(cb) = ON_COPY.get() {
                            cb(clip);
                        }
                    }
                }
            }
        })
        .ok();
}

pub fn read() -> Clip {
    let output = std::process::Command::new("pbpaste").output().ok();
    if let Some(out) = output {
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if !text.is_empty() {
            return Clip::Text { text, html: None, rtf: None };
        }
    }
    Clip::Empty
}

pub fn read_for_paste() -> Clip {
    read()
}

pub fn read_image_png() -> Option<Vec<u8>> {
    None
}

pub fn write(text: &str) -> Result<(), String> {
    use std::io::Write;
    let mut child = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        *LAST_TEXT.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_string());
        Ok(())
    } else {
        Err("pbcopy failed".into())
    }
}

pub fn write_rich(text: &str, _html: Option<&str>, _rtf: Option<&str>) -> Result<(), String> {
    write(text)
}

pub fn write_private(text: &str) -> Result<(), String> {
    write(text)
}

pub fn write_image(_mime: &str, _bytes: &[u8]) -> Result<(), String> {
    Ok(())
}

pub fn build_dropfiles_payload(_paths: &[PathBuf]) -> Vec<u8> {
    Vec::new()
}

pub fn parse_dropfiles_payload(_bytes: &[u8]) -> Option<Vec<PathBuf>> {
    None
}

pub fn read_files() -> Option<Vec<PathBuf>> {
    None
}

pub fn write_files(_paths: &[PathBuf]) -> bool {
    false
}
