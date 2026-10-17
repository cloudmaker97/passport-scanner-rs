//! Reading a USB device interface path.
//!
//! Kept apart from the Win32 calls so the parsing is testable on any host.

/// What a device interface path tells us about the unit behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathInfo {
    /// USB vendor id.
    pub vendor: u16,
    /// USB product id.
    pub product: u16,
    /// Interface number, when the path names one. A composite device exposes
    /// its video function as `MI_00`.
    pub interface: Option<u8>,
    /// The instance segment, which for these scanners is the serial number.
    pub instance: Option<String>,
}

/// Pull vendor, product, interface and instance out of a device interface path.
///
/// Paths look like
/// `\\?\usb#vid_07b3&pid_1a05&mi_00#7&1a2b3c&0&0000#{guid}`.
/// Matching is case-insensitive, because Windows is inconsistent about it.
#[must_use]
pub fn parse(path: &str) -> Option<PathInfo> {
    let lower = path.to_ascii_lowercase();
    let hex4 = |key: &str| -> Option<u16> {
        let at = lower.find(key)? + key.len();
        u16::from_str_radix(lower.get(at..at + 4)?, 16).ok()
    };

    let vendor = hex4("vid_")?;
    let product = hex4("pid_")?;

    let interface = lower.find("mi_").and_then(|at| {
        let at = at + 3;
        u8::from_str_radix(lower.get(at..at + 2)?, 16).ok()
    });

    // segments are separated by '#'; the third is the device instance
    let instance =
        path.split('#').nth(2).map(str::to_owned).filter(|s| !s.is_empty() && !s.starts_with('{'));

    Some(PathInfo { vendor, product, interface, instance })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference device, as Windows reports it.
    const REAL: &str = concat!(
        r"\\?\usb#vid_07b3&pid_1a05&mi_00#9&fd73ba9&0&0000#",
        "{65e8773d-8f56-11d0-a3b9-00a0c9223196}\\global"
    );

    #[test]
    fn reads_the_reference_device() {
        let info = parse(REAL).expect("path parses");
        assert_eq!(info.vendor, 0x07B3);
        assert_eq!(info.product, 0x1A05);
        assert_eq!(info.interface, Some(0));
        assert_eq!(info.instance.as_deref(), Some("9&fd73ba9&0&0000"));
    }

    #[test]
    fn is_case_insensitive() {
        let upper = REAL.to_ascii_uppercase();
        let info = parse(&upper).expect("path parses");
        assert_eq!(info.vendor, 0x07B3);
        assert_eq!(info.product, 0x1A05);
    }

    #[test]
    fn a_path_without_an_interface_number_still_parses() {
        let p = r"\\?\usb#vid_07b3&pid_1a05#KJ12040G6002758#{65e8773d-8f56-11d0-a3b9-00a0c9223196}";
        let info = parse(p).expect("path parses");
        assert_eq!(info.interface, None);
        assert_eq!(info.instance.as_deref(), Some("KJ12040G6002758"));
    }

    #[test]
    fn rejects_something_that_is_not_a_usb_path() {
        assert!(parse(r"\\?\root#system#0000#{guid}").is_none());
    }

    #[test]
    fn a_truncated_identifier_is_rejected_rather_than_guessed() {
        assert!(parse(r"\\?\usb#vid_07#pid_1a05#x#{guid}").is_none());
    }
}
