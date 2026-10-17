//! The OCR engine behind the zone reader.
//!
//! One trait, so the engine is a choice rather than a dependency. Tesseract is
//! what ships; anything that turns a grey image into text can take its place
//! without the reader above noticing.

use image::GrayImage;
use passport_scanner_core::{Error, Result};
use std::{
    io::Write,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

/// Something that turns an image into text.
pub trait Engine: Send + Sync + std::fmt::Debug {
    /// Recognise `image`.
    ///
    /// # Errors
    /// Fails if the engine cannot be run.
    fn read(&self, image: &GrayImage) -> Result<String>;

    /// Confident clockwise correction for a whole document image, if known.
    /// Returns 0, 90, 180, or -90. This does not publish recognized text.
    fn orientation(&self, _image: &GrayImage) -> Result<Option<i16>> {
        Ok(None)
    }

    /// Whether independent source images may be processed concurrently.
    /// Stateful engines retain sequential calls unless they explicitly opt in.
    fn parallel_sources(&self) -> bool {
        false
    }
}

/// Tesseract, driven as a subprocess.
///
/// Going through the command line rather than the C API keeps this crate free
/// of a build-time native dependency: the binary is found at run time, and its
/// absence is a plain error rather than a link failure.
#[derive(Debug, Clone)]
pub struct Tesseract {
    /// Path to the executable.
    pub binary: PathBuf,
    /// Traineddata to use.
    pub language: String,
}

impl Default for Tesseract {
    fn default() -> Self {
        Self { binary: Self::find(), language: "eng".to_owned() }
    }
}

impl Tesseract {
    /// Keep OCR and availability probes silent when called from the native GUI.
    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command.stdin(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    }

    /// Where the executable is, taking `TESSERACT` if it is set and otherwise
    /// looking where the Windows installer puts it.
    #[must_use]
    pub fn find() -> PathBuf {
        if let Ok(p) = std::env::var("TESSERACT") {
            return PathBuf::from(p);
        }
        let installed = PathBuf::from(r"C:\Program Files\Tesseract-OCR\tesseract.exe");
        if installed.exists() { installed } else { PathBuf::from("tesseract") }
    }

    /// Is the engine actually runnable?
    #[must_use]
    pub fn available(&self) -> bool {
        self.command().arg("--version").output().is_ok()
    }

    pub(crate) fn orientation_text(&self, image: &GrayImage, osd: bool) -> Result<Option<String>> {
        let input = ZoneInput::new(image)?;
        let mut command = self.command();
        command.arg(&input.0).arg("stdout").args(["--psm", if osd { "0" } else { "11" }]);
        command.args(["-l", if osd { "osd" } else { &self.language }]);
        if !osd {
            command.arg("tsv");
        }
        let output = command
            .output()
            .map_err(|e| Error::Unsupported(format!("orientation OCR could not run: {e}")))?;
        // OSD legitimately fails on sparse/blank cards or without osd data.
        // Never log its stderr, which may contain recognized document text.
        Ok(output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned()))
    }
}

impl Engine for Tesseract {
    fn orientation(&self, image: &GrayImage) -> Result<Option<i16>> {
        crate::orientation::detect(self, image)
    }
    fn parallel_sources(&self) -> bool {
        true
    }
    fn read(&self, image: &GrayImage) -> Result<String> {
        let input = ZoneInput::new(image)?;
        let output = self.command()
            .arg(&input.0)
            .arg("stdout")
            .args(["-l", &self.language])
            // One uniform block of text, which is what a zone is.
            .args(["--psm", "6"])
            // The zone's whole alphabet. Anything else is a misread.
            .arg("-c")
            .arg("tessedit_char_whitelist=ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789<")
            // Dictionaries only hurt: these are not words.
            .args(["-c", "load_system_dawg=0", "-c", "load_freq_dawg=0"])
            .output()
            .map_err(|e| {
                Error::Unsupported(format!(
                    "tesseract could not be run from {}: {e}. Install it, or point TESSERACT at it.",
                    self.binary.display()
                ))
            })?;

        if !output.status.success() {
            return Err(Error::Rejected(format!(
                "tesseract exited with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// Per-call file ownership prevents concurrent OCR requests from reading each
/// other's input. Each file is removed when its OCR attempt finishes.
struct ZoneInput(PathBuf);
impl ZoneInput {
    fn new(image: &GrayImage) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "passport-scanner-{}-{}.png",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::File::create_new(&path) {
                Ok(file) => {
                    let input = Self(path);
                    let mut writer = std::io::BufWriter::new(file);
                    image::DynamicImage::ImageLuma8(image.clone())
                        .write_to(&mut writer, image::ImageFormat::Png)
                        .map_err(|e| {
                            Error::Malformed(format!("could not write zone image: {e}"))
                        })?;
                    writer.flush()?;
                    return Ok(input);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }
}
impl Drop for ZoneInput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Canned(&'static str);
    impl Engine for Canned {
        fn read(&self, _image: &GrayImage) -> Result<String> {
            Ok(self.0.to_owned())
        }
    }

    #[test]
    fn an_engine_is_swappable_without_tesseract_present() {
        let e = Canned("ABC<<<");
        assert_eq!(e.read(&GrayImage::new(1, 1)).unwrap(), "ABC<<<");
    }

    #[test]
    fn the_default_engine_points_somewhere_concrete() {
        let t = Tesseract::default();
        assert!(!t.binary.as_os_str().is_empty());
        assert_eq!(t.language, "eng");
    }

    #[test]
    fn concurrent_inputs_are_distinct_lossless_and_removed_on_drop() {
        let original = GrayImage::from_raw(16, 16, (0..=255).collect()).unwrap();
        let first = ZoneInput::new(&original).unwrap();
        let second = ZoneInput::new(&GrayImage::new(2, 3)).unwrap();
        assert_ne!(first.0, second.0);
        assert_eq!(image::open(&first.0).unwrap().to_luma8(), original);
        let path = first.0.clone();
        drop(first);
        assert!(!path.exists());
        assert!(second.0.exists());
    }
}
