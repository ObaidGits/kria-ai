//! OCR Engine — screen text extraction via Tesseract.
//!
//! Provides screen reading capability for:
//! - Visual verification (verify text is visible on screen)
//! - Popup/dialog text extraction
//! - Screen state understanding
//! - Accessibility fallback when AT-SPI is unavailable
//!
//! Uses `tesseract` CLI (available on the system) via subprocess.
//! Falls back gracefully when tesseract or screenshot tools are unavailable.

use std::path::PathBuf;
use tracing::{debug, info, warn};

/// Result of an OCR operation.
#[derive(Debug, Clone)]
pub struct OcrResult {
    /// Extracted text from the screen/region
    pub text: String,
    /// Whether the extraction succeeded
    pub success: bool,
    /// Evidence/error message
    pub evidence: String,
    /// Path to the screenshot used (if any)
    pub screenshot_path: Option<PathBuf>,
}

impl OcrResult {
    pub fn ok(text: String, evidence: impl Into<String>) -> Self {
        Self {
            text,
            success: true,
            evidence: evidence.into(),
            screenshot_path: None,
        }
    }
    pub fn err(evidence: impl Into<String>) -> Self {
        Self {
            text: String::new(),
            success: false,
            evidence: evidence.into(),
            screenshot_path: None,
        }
    }
    /// Check if the extracted text contains a substring (case-insensitive).
    pub fn contains(&self, needle: &str) -> bool {
        self.text.to_lowercase().contains(&needle.to_lowercase())
    }
}

/// OCR engine using Tesseract.
pub struct OcrEngine;

impl OcrEngine {
    pub fn new() -> Self {
        Self
    }

    /// Check if OCR is available (tesseract installed).
    pub fn is_available() -> bool {
        std::path::Path::new("/usr/bin/tesseract").exists()
            || std::path::Path::new("/usr/local/bin/tesseract").exists()
    }

    /// Take a screenshot and extract all text from it.
    ///
    /// Uses PIL/Python for screenshot (works on X11 and Wayland via XWayland).
    /// Falls back to xwd/import if PIL is unavailable.
    pub async fn read_screen(&self) -> OcrResult {
        if !Self::is_available() {
            return OcrResult::err("tesseract not installed — OCR unavailable");
        }

        // Take screenshot to a temp file
        let screenshot_path = PathBuf::from(format!("/tmp/kria_ocr_{}.png", uuid::Uuid::new_v4()));

        let screenshot_ok = self.take_screenshot(&screenshot_path).await;
        if !screenshot_ok {
            return OcrResult::err("Screenshot failed — display may not be accessible");
        }

        // Run tesseract on the screenshot
        let result = self.run_tesseract(&screenshot_path).await;

        // Clean up screenshot
        let _ = std::fs::remove_file(&screenshot_path);

        result
    }

    /// Read text from a specific region of the screen [x, y, width, height].
    pub async fn read_region(&self, region: [i32; 4]) -> OcrResult {
        if !Self::is_available() {
            return OcrResult::err("tesseract not installed");
        }

        let screenshot_path =
            PathBuf::from(format!("/tmp/kria_ocr_region_{}.png", uuid::Uuid::new_v4()));

        let screenshot_ok = self.take_screenshot_region(&screenshot_path, region).await;
        if !screenshot_ok {
            return OcrResult::err("Region screenshot failed");
        }

        let result = self.run_tesseract(&screenshot_path).await;
        let _ = std::fs::remove_file(&screenshot_path);
        result
    }

    /// Check if specific text is visible on screen.
    pub async fn text_visible_on_screen(&self, text: &str) -> bool {
        let result = self.read_screen().await;
        if result.success {
            let found = result.contains(text);
            if found {
                info!(target: "ocr_engine", text = %text, "Text found on screen via OCR");
            } else {
                debug!(target: "ocr_engine", text = %text, "Text NOT found on screen via OCR");
            }
            found
        } else {
            warn!(target: "ocr_engine", evidence = %result.evidence, "OCR failed");
            false
        }
    }

    /// Take a full-screen screenshot using the best available method.
    ///
    /// Priority order:
    /// 1. xdg-desktop-portal (Wayland-native, works on GNOME/KDE/wlroots)
    /// 2. Python PIL ImageGrab (X11/XWayland)
    /// 3. xwd (X11 only)
    /// 4. gnome-screenshot (X11/Wayland via DBus)
    async fn take_screenshot(&self, path: &PathBuf) -> bool {
        let path_str = path.to_string_lossy().to_string();

        // Strategy 1: xdg-desktop-portal Screenshot (Wayland-native)
        // This works on GNOME, KDE, and wlroots compositors.
        // The portal returns an object path; we then read the file from the URI.
        let portal_result = tokio::process::Command::new("python3")
            .args([
                "-c",
                &format!(
                    r#"
import subprocess, sys, os, shutil, glob, time

# Use gdbus to call the portal
result = subprocess.run([
    'gdbus', 'call', '--session',
    '--dest', 'org.freedesktop.portal.Desktop',
    '--object-path', '/org/freedesktop/portal/desktop',
    '--method', 'org.freedesktop.portal.Screenshot.Screenshot',
    'x11:0',
    '{{}}'
], capture_output=True, text=True, timeout=10)

if result.returncode != 0:
    print('portal_failed:' + result.stderr[:100])
    sys.exit(1)

# Wait briefly for the file to appear
time.sleep(0.5)

# The portal saves to ~/Pictures/ on GNOME
home = os.path.expanduser('~')
search_patterns = [
    os.path.join(home, 'Pictures', 'Screenshot*.png'),
    os.path.join(home, 'Pictures', 'screenshot*.png'),
    '/tmp/screenshot*.png',
    '/tmp/*.png',
]

for pattern in search_patterns:
    files = sorted(glob.glob(pattern), key=os.path.getmtime, reverse=True)
    if files:
        newest = files[0]
        # Must be recent (within 5 seconds)
        if time.time() - os.path.getmtime(newest) < 5:
            shutil.copy2(newest, '{}')
            print('portal_ok')
            sys.exit(0)

print('portal_no_file')
sys.exit(1)
"#,
                    path_str
                ),
            ])
            .output()
            .await;

        if let Ok(out) = portal_result {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.trim() == "portal_ok" && path.exists() {
                return true;
            }
        }

        // Strategy 2: Python PIL ImageGrab (X11/XWayland)
        let py_result = tokio::process::Command::new("python3")
            .args([
                "-c",
                &format!(
                    "from PIL import ImageGrab; img = ImageGrab.grab(); img.save('{}')",
                    path_str
                ),
            ])
            .output()
            .await;

        if let Ok(out) = py_result {
            if out.status.success() && path.exists() {
                return true;
            }
        }

        // Strategy 3: xwd (X11 only)
        let xwd_result = tokio::process::Command::new("xwd")
            .args(["-root", "-silent", "-out", &path_str])
            .output()
            .await;

        if let Ok(out) = xwd_result {
            if out.status.success() {
                let _ = tokio::process::Command::new("convert")
                    .args([&path_str, &path_str])
                    .output()
                    .await;
                return path.exists();
            }
        }

        // Strategy 4: gnome-screenshot
        let gnome_result = tokio::process::Command::new("gnome-screenshot")
            .args(["-f", &path_str])
            .output()
            .await;

        if let Ok(out) = gnome_result {
            if out.status.success() {
                return path.exists();
            }
        }

        false
    }

    /// Take a screenshot of a specific region.
    async fn take_screenshot_region(&self, path: &PathBuf, region: [i32; 4]) -> bool {
        let path_str = path.to_string_lossy().to_string();
        let [x, y, w, h] = region;

        let py_result = tokio::process::Command::new("python3")
            .args([
                "-c",
                &format!(
                    "from PIL import ImageGrab; img = ImageGrab.grab(bbox=({},{},{},{})); img.save('{}')",
                    x, y, x + w, y + h, path_str
                ),
            ])
            .output()
            .await;

        if let Ok(out) = py_result {
            if out.status.success() && path.exists() {
                return true;
            }
        }

        false
    }

    /// Run tesseract on an image file and return extracted text.
    /// Output is capped at 100KB to prevent OOM.
    async fn run_tesseract(&self, image_path: &PathBuf) -> OcrResult {
        let output_base = format!("/tmp/kria_tess_{}", uuid::Uuid::new_v4());
        let output_txt = format!("{}.txt", output_base);

        let result = tokio::process::Command::new("tesseract")
            .args([
                image_path.to_str().unwrap_or(""),
                &output_base,
                "-l",
                "eng",
                "--psm",
                "3", // Fully automatic page segmentation
            ])
            .output()
            .await;

        match result {
            Ok(out) if out.status.success() => {
                // Cap OCR output at 100KB to prevent OOM
                let text = {
                    use std::io::Read;
                    match std::fs::File::open(&output_txt) {
                        Ok(mut f) => {
                            let mut buf = String::new();
                            let _ = f.by_ref().take(102_400).read_to_string(&mut buf);
                            buf.trim().to_string()
                        }
                        Err(_) => String::new(),
                    }
                };
                let _ = std::fs::remove_file(&output_txt);

                if text.is_empty() {
                    OcrResult::err("Tesseract produced no text (screen may be blank or unreadable)")
                } else {
                    info!(
                        target: "ocr_engine",
                        chars = text.len(),
                        "OCR extracted text from screen"
                    );
                    OcrResult::ok(text, "OCR succeeded")
                }
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let _ = std::fs::remove_file(&output_txt);
                OcrResult::err(format!(
                    "Tesseract failed: {}",
                    &stderr.trim()[..stderr.trim().len().min(200)]
                ))
            }
            Err(e) => OcrResult::err(format!("Failed to run tesseract: {}", e)),
        }
    }
}

impl Default for OcrEngine {
    fn default() -> Self {
        Self::new()
    }
}
