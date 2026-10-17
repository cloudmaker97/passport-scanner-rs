//! The Win32 half: find the capture filter, open it, talk to its extension unit.

use crate::{
    cameracontrol::{self, CameraProperty},
    ksp::{self, Direction, IOCTL_KS_PROPERTY, XU_DATA_LEN},
    path::{self, PathInfo},
};
use passport_scanner_core::{Error, Result, Selector, VendorTransport};
use std::ptr;
use windows_sys::{
    Win32::{
        Devices::DeviceAndDriverInstallation::{
            DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO, SP_DEVICE_INTERFACE_DATA,
            SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList,
            SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
        },
        Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
        System::IO::DeviceIoControl,
    },
    core::GUID,
};

/// `KSCATEGORY_CAPTURE` - the interface class a UVC capture filter registers
/// under, and where the scanner's extension unit lives.
const KSCATEGORY_CAPTURE: GUID = GUID {
    data1: 0x65E8_773D,
    data2: 0x8F56,
    data3: 0x11D0,
    data4: [0xA3, 0xB9, 0x00, 0xA0, 0xC9, 0x22, 0x31, 0x96],
};

/// What SetupDiGetClassDevsW returns on failure.
const INVALID_DEVINFO: HDEVINFO = -1;

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;

/// One scanner found on the bus, with the path needed to open it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// What the interface path says about the unit.
    pub info: PathInfo,
    /// The path to hand to [`KsTransport::open`].
    pub device_path: String,
}

fn last_error(context: &str) -> Error {
    // SAFETY: GetLastError only reads this thread's error slot.
    let code = unsafe { GetLastError() };
    Error::Rejected(format!("{context} failed, GetLastError {code}"))
}

/// List attached capture filters belonging to `vendor`.
///
/// Composite devices expose several interfaces; only the video function carries
/// the extension unit, so anything that names an interface other than zero is
/// skipped.
///
/// # Errors
/// Fails if the device interface class cannot be enumerated.
pub fn enumerate(vendor: u16) -> Result<Vec<Found>> {
    // SAFETY: a null enumerator with DIGCF_DEVICEINTERFACE asks for every
    // present interface of the class; the handle is released below.
    let set = unsafe {
        SetupDiGetClassDevsW(
            &KSCATEGORY_CAPTURE,
            ptr::null(),
            ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    // HDEVINFO is an isize in this binding; its invalid value is -1.
    if set == INVALID_DEVINFO {
        return Err(last_error("SetupDiGetClassDevsW"));
    }

    let mut found = Vec::new();
    let mut index = 0u32;
    loop {
        let mut iface = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            InterfaceClassGuid: GUID { data1: 0, data2: 0, data3: 0, data4: [0; 8] },
            Flags: 0,
            Reserved: 0,
        };
        // SAFETY: `set` is live and `iface` is correctly sized.
        let more = unsafe {
            SetupDiEnumDeviceInterfaces(set, ptr::null(), &KSCATEGORY_CAPTURE, index, &mut iface)
        };
        if more == 0 {
            break;
        }
        index += 1;

        if let Some(path_str) = detail_path(set, &mut iface)
            && let Some(info) = path::parse(&path_str)
            && info.vendor == vendor
            && info.interface.unwrap_or(0) == 0
        {
            found.push(Found { info, device_path: path_str });
        }
    }

    // SAFETY: `set` came from SetupDiGetClassDevsW and is not used afterwards.
    unsafe { SetupDiDestroyDeviceInfoList(set) };
    Ok(found)
}

/// Fetch one interface's path, using the usual ask-twice pattern.
fn detail_path(set: HDEVINFO, iface: &mut SP_DEVICE_INTERFACE_DATA) -> Option<String> {
    let mut needed = 0u32;
    // SAFETY: a null detail buffer asks only for the required size.
    unsafe {
        SetupDiGetDeviceInterfaceDetailW(
            set,
            iface,
            ptr::null_mut(),
            0,
            &mut needed,
            ptr::null_mut(),
        );
    }
    if needed == 0 {
        return None;
    }

    let mut buffer = vec![0u8; needed as usize];
    let detail = buffer.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    // SAFETY: `buffer` is at least `needed` bytes and aligned for the struct,
    // which starts with a u32.
    unsafe {
        (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
        if SetupDiGetDeviceInterfaceDetailW(
            set,
            iface,
            detail,
            needed,
            ptr::null_mut(),
            ptr::null_mut(),
        ) == 0
        {
            return None;
        }
    }

    // DevicePath is a NUL-terminated wide string following the cbSize field.
    // SAFETY: the call above filled the buffer and NUL-terminated the path.
    let wide = unsafe {
        let start = ptr::addr_of!((*detail).DevicePath).cast::<u16>();
        let mut len = 0usize;
        while *start.add(len) != 0 {
            len += 1;
        }
        std::slice::from_raw_parts(start, len)
    };
    Some(String::from_utf16_lossy(wide))
}

/// An open vendor channel to one scanner.
#[derive(Debug)]
pub struct KsTransport {
    handle: HANDLE,
}

// SAFETY: the handle is an owned kernel object; moving it between threads is
// sound, and `&mut self` on every method keeps use serialised.
unsafe impl Send for KsTransport {}

impl KsTransport {
    /// Open the capture filter at `device_path`.
    ///
    /// # Errors
    /// Fails if the path cannot be opened - usually because another process
    /// holds the camera exclusively.
    pub fn open(device_path: &str) -> Result<Self> {
        let wide: Vec<u16> = device_path.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(last_error("CreateFileW on the capture filter"));
        }
        Ok(Self { handle })
    }

    /// Open the first scanner of `vendor` found on the bus.
    ///
    /// # Errors
    /// [`Error::DeviceNotFound`] if nothing matches.
    pub fn open_first(vendor: u16) -> Result<(Self, PathInfo)> {
        let found = enumerate(vendor)?.into_iter().next().ok_or(Error::DeviceNotFound)?;
        Ok((Self::open(&found.device_path)?, found.info))
    }

    /// Read one standard camera-control property.
    ///
    /// Returns the value and its flags, which say whether the device is
    /// choosing it or honouring what it was told.
    ///
    /// # Errors
    /// Fails if the device does not carry the property.
    pub fn camera_control(&mut self, property: CameraProperty) -> Result<(i32, u32)> {
        let request = cameracontrol::request(property, cameracontrol::Kind::Get);
        let mut data = cameracontrol::payload(0, 0);
        self.ioctl(&request, &mut data)?;
        cameracontrol::parse(&data)
            .ok_or_else(|| Error::Malformed("short camera-control reply".to_owned()))
    }

    /// The range a camera-control property accepts.
    ///
    /// # Errors
    /// Fails if the device does not carry the property.
    pub fn camera_control_range(&mut self, property: CameraProperty) -> Result<(i32, i32, i32)> {
        let request = cameracontrol::request(property, cameracontrol::Kind::Range);
        // A range reply is a KSPROPERTY_DESCRIPTION followed by the members.
        let mut data = [0u8; 64];
        self.ioctl(&request, &mut data)?;
        // The stepped range sits after the description and member header.
        let at = 32 + 16;
        let read = |o: usize| -> i32 {
            data.get(o..o + 4).and_then(|b| b.try_into().ok()).map_or(0, i32::from_le_bytes)
        };
        Ok((read(at), read(at + 4), read(at + 8)))
    }

    /// Set one standard camera-control property, taking it off automatic.
    ///
    /// # Errors
    /// Fails if the device refuses the property or the value.
    pub fn set_camera_control(&mut self, property: CameraProperty, value: i32) -> Result<()> {
        let request = cameracontrol::request(property, cameracontrol::Kind::Set);
        let mut data = cameracontrol::payload(value, cameracontrol::FLAG_MANUAL);
        self.ioctl(&request, &mut data)
    }

    /// Issue one property request with a caller-supplied data buffer.
    fn ioctl(&mut self, request: &[u8], data: &mut [u8]) -> Result<()> {
        let mut returned = 0u32;
        // SAFETY: both buffers are owned by the caller, correctly sized, and
        // live for the duration of the call; the handle came from CreateFileW.
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_KS_PROPERTY,
                request.as_ptr().cast(),
                request.len() as u32,
                data.as_mut_ptr().cast(),
                data.len() as u32,
                &mut returned,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            // SAFETY: reads this thread's error slot only.
            let code = unsafe { GetLastError() };
            return Err(Error::VendorCommand { selector: 0, status: code });
        }
        Ok(())
    }

    /// Issue one property request against the extension unit.
    fn property(&mut self, direction: Direction, data: &mut [u8; XU_DATA_LEN]) -> Result<()> {
        let request = ksp::node_request(direction);
        let mut returned = 0u32;
        // SAFETY: both buffers are owned, correctly sized, and live for the
        // duration of the call; the handle came from CreateFileW.
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_KS_PROPERTY,
                request.as_ptr().cast(),
                request.len() as u32,
                data.as_mut_ptr().cast(),
                XU_DATA_LEN as u32,
                &mut returned,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            // SAFETY: reads this thread's error slot only.
            let code = unsafe { GetLastError() };
            return Err(Error::VendorCommand { selector: data[0], status: code });
        }
        Ok(())
    }
}

impl Drop for KsTransport {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE {
            // SAFETY: the handle came from CreateFileW and is closed once.
            unsafe { CloseHandle(self.handle) };
        }
    }
}

impl VendorTransport for KsTransport {
    fn set(&mut self, selector: Selector, payload: &[u8]) -> Result<()> {
        if matches!(selector.get(), 0x00 | 0x01) {
            let packets = ksp::register_write_packets(selector.get(), payload)
                .ok_or_else(|| Error::Malformed("register write needs address and value".into()))?;
            for mut packet in packets {
                self.property(Direction::Set, &mut packet)?;
            }
            return Ok(());
        }
        if selector.get() != 0xf0 {
            return Err(Error::unsupported(format!("write selector {:#04x}", selector.get())));
        }
        let mut data = [0u8; XU_DATA_LEN];
        let n = payload.len().min(XU_DATA_LEN);
        data[..n].copy_from_slice(&payload[..n]);
        self.property(Direction::Set, &mut data)
    }

    /// Read, which takes two requests.
    ///
    /// The extension unit answers in place, but only after it has been told
    /// what is being asked: the query goes in with a `SET`, and a following
    /// `GET` collects the reply. Issuing only the `GET` returns whatever the
    /// unit happened to hold from the previous exchange - which looks like a
    /// working read until the values stop changing.
    fn get(&mut self, selector: Selector, buffer: &mut [u8]) -> Result<()> {
        let mut data = [0u8; XU_DATA_LEN];
        let n = buffer.len().min(XU_DATA_LEN);
        if matches!(selector.get(), 0x00 | 0x01) {
            if buffer.len() < 2 {
                return Err(Error::Malformed("register read needs an address".into()));
            }
            data[0] = selector.get();
            data[1] = buffer[1];
            data[2] = buffer[0];
        } else if matches!(selector.get(), 0xf0 | 0xee) {
            data[..n].copy_from_slice(&buffer[..n]);
        } else {
            return Err(Error::unsupported(format!("read selector {:#04x}", selector.get())));
        }

        self.property(Direction::Set, &mut data)?;
        // The bridge completes the request asynchronously. The shipped
        // etLib2 UVC read path waits 5 ms between selecting and retrieving.
        std::thread::sleep(std::time::Duration::from_millis(5));
        self.property(Direction::Get, &mut data)?;

        buffer[..n].copy_from_slice(&data[..n]);
        Ok(())
    }

    fn payload_len(&self) -> usize {
        XU_DATA_LEN
    }
}
