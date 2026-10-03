//! Runs the local `tesseract` binary and reads its TSV word boxes.
//!
//! The image bytes are written to the process standard input. No shell is
//! involved, and the recognized words are not written to the log.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{PdfError, PdfResult};

/// Counts returned by [`super::add_searchable_text_layer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OcrReport {
    /// Pages that painted exactly one image and no text.
    pub pages_seen: usize,
    /// Pages that received at least one invisible word.
    pub pages_recognized: usize,
    /// Invisible words appended to content streams.
    pub words_inserted: usize,
}

/// One recognized word and its pixel box. The origin is the top-left of the image.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OcrWord {
    pub text: String,
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub confidence: f32,
}

const MAX_TSV_BYTES: usize = 1024 * 1024;
const RECOGNITION_TIMEOUT: Duration = Duration::from_secs(20);

pub(crate) fn accept_language(language: &str) -> PdfResult<()> {
    let bytes = language.as_bytes();
    if !(3..=16).contains(&bytes.len()) || !bytes[..3].iter().all(|b| b.is_ascii_alphabetic()) {
        return Err(rejected_language());
    }
    if bytes.len() == 3 {
        return Ok(());
    }
    if bytes[3] != b'_' && bytes[3] != b'+' {
        return Err(rejected_language());
    }
    if bytes.len() == 4 || !bytes[4..].iter().all(|b| b.is_ascii_alphanumeric()) {
        return Err(rejected_language());
    }
    Ok(())
}

fn rejected_language() -> PdfError {
    PdfError::OperationError("OCR language was rejected.".to_string())
}

pub(crate) fn recognize_image(image: &[u8], language: &str) -> PdfResult<Vec<OcrWord>> {
    let binary = locate_tesseract()?;
    let mut child = Command::new(&binary)
        .args(["stdin", "stdout", "--psm", "3", "-l", language, "tsv"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| unavailable())?;

    let mut stdin = child.stdin.take().ok_or_else(unavailable)?;
    let mut stdout = child.stdout.take().ok_or_else(unavailable)?;
    let mut stderr = child.stderr.take().ok_or_else(unavailable)?;
    let image = image.to_vec();

    let writer = thread::spawn(move || {
        let wrote = stdin.write_all(&image).is_ok();
        drop(stdin);
        wrote
    });
    let reader = thread::spawn(move || read_capped(&mut stdout, MAX_TSV_BYTES));
    let _stderr = thread::spawn(move || {
        let _ = read_capped(&mut stderr, 4096);
    });

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let wrote = writer.join().unwrap_or(false);
                let captured = reader.join().unwrap_or(Err(()));
                if !status.success() || !wrote {
                    return Err(failed());
                }
                let bytes = captured.map_err(|_| failed())?;
                return Ok(parse_tsv(&bytes));
            }
            Ok(None) if started.elapsed() > RECOGNITION_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(PdfError::OperationError(
                    "Optical character recognition timed out.".to_string(),
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                let _ = child.kill();
                return Err(failed());
            }
        }
    }
}

fn read_capped(reader: &mut impl Read, cap: usize) -> Result<Vec<u8>, ()> {
    let mut output = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => return Ok(output),
            Ok(count) => {
                if output.len().saturating_add(count) > cap {
                    return Err(());
                }
                output.extend_from_slice(&chunk[..count]);
            }
            Err(_) => return Err(()),
        }
    }
}

pub(crate) fn parse_tsv(bytes: &[u8]) -> Vec<OcrWord> {
    let text = String::from_utf8_lossy(bytes);
    let mut words = Vec::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with("level\t") {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() < 12 || columns[0] != "5" {
            continue;
        }
        let (Ok(left), Ok(top), Ok(width), Ok(height), Ok(confidence)) = (
            columns[6].parse::<i32>(),
            columns[7].parse::<i32>(),
            columns[8].parse::<i32>(),
            columns[9].parse::<i32>(),
            columns[10].parse::<f32>(),
        ) else {
            continue;
        };
        if confidence < 0.0 || width <= 0 || height <= 0 || left < 0 || top < 0 {
            continue;
        }
        let word = columns[11..].join(" ").trim().to_string();
        if word.is_empty() || word.chars().count() > 80 {
            continue;
        }
        words.push(OcrWord {
            text: word,
            left,
            top,
            width,
            height,
            confidence,
        });
    }
    words
}

fn locate_tesseract() -> PdfResult<PathBuf> {
    if let Some(configured) = std::env::var_os("PDFENGINE_TESSERACT") {
        let path = PathBuf::from(configured);
        let name_ok = path.file_name().and_then(|name| name.to_str()) == Some("tesseract");
        if path.is_absolute() && name_ok && path.is_file() {
            return Ok(path);
        }
        return Err(unavailable());
    }
    for candidate in [
        "/opt/homebrew/bin/tesseract",
        "/usr/local/bin/tesseract",
        "/usr/bin/tesseract",
    ] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Ok(path);
        }
    }
    Ok(PathBuf::from("tesseract"))
}

fn unavailable() -> PdfError {
    PdfError::OperationError("Optical character recognition is unavailable.".to_string())
}

fn failed() -> PdfError {
    PdfError::OperationError("Optical character recognition failed.".to_string())
}
