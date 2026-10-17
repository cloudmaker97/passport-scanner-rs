//! The nokhwa-backed implementation.
//!
//! The camera lives on a thread of its own and is reached over channels. That
//! is not ceremony: nokhwa's handle is not `Send`, and capture wants to be off
//! whatever thread is driving the scan anyway.

use nokhwa::{
    Camera,
    pixel_format::RgbFormat,
    utils::{
        ApiBackend, CameraFormat, CameraIndex, ControlValueDescription, ControlValueSetter,
        FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
    },
};
use passport_scanner_core::{Control, Error, Frame, FrameSource, LightSource, PixelFormat, Result};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

fn wrap(err: impl std::fmt::Display) -> Error {
    Error::Rejected(err.to_string())
}

/// Attached cameras, as index and human-readable name.
///
/// # Errors
/// Fails if the platform's camera stack cannot be queried.
pub fn list_cameras() -> Result<Vec<(u32, String)>> {
    let devices = nokhwa::query(ApiBackend::Auto).map_err(wrap)?;
    Ok(devices
        .into_iter()
        .map(|d| {
            let index = match d.index() {
                CameraIndex::Index(i) => *i,
                CameraIndex::String(_) => 0,
            };
            (index, d.human_name())
        })
        .collect())
}

/// What the thread holding the camera can be asked to do.
enum Request {
    Start(Sender<Result<()>>),
    Stop(Sender<Result<()>>),
    Reopen(Sender<Result<()>>),
    Grab(Instant, Duration, Sender<Result<Frame>>),
    SetSource(LightSource),
    Controls(Sender<Result<Vec<Control>>>),
    ControlDefaults(Sender<Result<Vec<(String, i64)>>>),
    SetControl(String, i64, Sender<Result<()>>),
    Close,
}

/// A camera, running on its own thread.
#[derive(Debug)]
pub struct CameraSource {
    tx: Sender<Request>,
    worker: Option<JoinHandle<()>>,
    resolution: (u32, u32),
    timed_out: AtomicBool,
}

impl CameraSource {
    /// Open the camera at `index`, asking for `width` by `height` MJPEG.
    ///
    /// The nearest format the device offers is taken, so a scanner that only
    /// advertises its native resolution still opens.
    ///
    /// # Errors
    /// Fails if the camera cannot be opened or offers no usable format.
    pub fn open(index: u32, width: u32, height: u32) -> Result<Self> {
        let (tx, rx) = channel::<Request>();
        let (ready_tx, ready_rx) = channel::<Result<(u32, u32)>>();

        let worker = std::thread::Builder::new()
            .name("passport-scanner-capture".to_owned())
            .spawn(move || run(index, width, height, &ready_tx, &rx))
            .map_err(Error::from)?;

        match ready_rx.recv() {
            Ok(Ok(resolution)) => {
                Ok(Self { tx, worker: Some(worker), resolution, timed_out: AtomicBool::new(false) })
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(Error::Rejected("the capture thread stopped while opening".to_owned())),
        }
    }

    /// Open the first camera whose name contains `needle`, case-insensitively.
    ///
    /// # Errors
    /// [`Error::DeviceNotFound`] if no name matches.
    pub fn open_named(needle: &str, width: u32, height: u32) -> Result<Self> {
        let needle = needle.to_ascii_lowercase();
        let (index, _) = list_cameras()?
            .into_iter()
            .find(|(_, name)| name.to_ascii_lowercase().contains(&needle))
            .ok_or(Error::DeviceNotFound)?;
        Self::open(index, width, height)
    }

    /// Tag frames taken from now on as coming from `source`.
    ///
    /// The camera cannot know which lamp is lit; the scan sequence does, and
    /// says so here before taking each frame.
    pub fn set_source(&mut self, source: LightSource) {
        let _ = self.tx.send(Request::SetSource(source));
    }

    /// The resolution the camera actually opened at.
    #[must_use]
    pub fn resolution(&self) -> (u32, u32) {
        self.resolution
    }

    /// Every camera control the device exposes, with its range.
    ///
    /// # Errors
    /// Fails if the capture thread has stopped.
    /// Replace the handle on the device with a newly opened one.
    ///
    /// Needed once per scan, which is not obvious and was not cheap to find.
    /// Measured over ten runs against one unmoved document, the first capture
    /// after opening the device was exposed correctly and every capture after
    /// it was not - identically so, 57 per cent of the frame dark against 15
    /// per cent, which rules out timing. Closing and reopening the whole
    /// device between runs fixed it. Reinitialising only the scanner side did
    /// not, and neither did stopping and restarting the stream. What is left
    /// is the handle itself: something the capture stack configures when the
    /// device is opened does not survive the first stream it serves.
    ///
    /// # Errors
    /// Fails if the device cannot be opened again.
    pub fn reopen(&self) -> Result<()> {
        self.ask(Request::Reopen)
    }

    pub fn controls(&self) -> Result<Vec<Control>> {
        self.ask(Request::Controls)
    }

    /// Read the platform-reported defaults for integer camera controls.
    ///
    /// This only queries metadata; it does not reset controls or change the
    /// camera. Non-integer controls are omitted instead of inventing defaults.
    ///
    /// # Errors
    /// Fails if control enumeration fails or the capture thread is unavailable.
    pub fn control_defaults(&self) -> Result<Vec<(String, i64)>> {
        self.ask(Request::ControlDefaults)
    }

    /// Set one control by the name [`CameraSource::controls`] reports.
    ///
    /// # Errors
    /// Fails if the control is unknown or the device refuses the value.
    pub fn set_control(&mut self, name: &str, value: i64) -> Result<()> {
        let name = name.to_owned();
        self.ask(move |tx| Request::SetControl(name, value, tx))
    }
    fn ask<T>(&self, make: impl FnOnce(Sender<Result<T>>) -> Request) -> Result<T> {
        if self.timed_out.load(Ordering::Acquire) {
            return Err(Error::Rejected(
                "capture timed out; discard this camera source".to_owned(),
            ));
        }
        let (tx, rx) = channel();
        self.tx
            .send(make(tx))
            .map_err(|_| Error::Rejected("the capture thread has stopped".to_owned()))?;
        rx.recv().map_err(|_| Error::Rejected("the capture thread dropped the reply".to_owned()))?
    }
}

/// The body of the capture thread: it owns the camera for its whole life.
fn run(
    index: u32,
    width: u32,
    height: u32,
    ready: &Sender<Result<(u32, u32)>>,
    rx: &Receiver<Request>,
) {
    let format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(
        CameraFormat::new(Resolution::new(width, height), FrameFormat::MJPEG, 30),
    ));

    let camera = match Camera::new(CameraIndex::Index(index), format).map_err(wrap) {
        Ok(c) => c,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let res = camera.resolution();
    if ready.send(Ok((res.width(), res.height()))).is_err() {
        return;
    }
    let mut camera = Some(camera);

    let mut source = LightSource::White;
    let mut streaming = false;

    while let Ok(request) = rx.recv() {
        if matches!(request, Request::Close) {
            break;
        }
        if let Request::SetSource(s) = request {
            source = s;
            continue;
        }
        if let Request::Reopen(reply) = request {
            // Release the previous source reader/device before acquiring a new
            // one. Constructing the replacement first briefly owned the same
            // camera twice, which can exhaust exclusive capture resources.
            if let Some(mut old) = camera.take() {
                if streaming {
                    let _ = old.stop_stream();
                }
                drop(old);
            }
            streaming = false;
            let result = Camera::new(CameraIndex::Index(index), format)
                .map_err(wrap)
                .map(|fresh| camera = Some(fresh));
            let _ = reply.send(result);
            continue;
        }
        let Some(camera) = camera.as_mut() else {
            // A failed reopen leaves no stale handle available for scanning.
            // Another explicit Reopen can retry; other operations fail closed.
            let error = || Error::Rejected("camera is unavailable after failed reopen".to_owned());
            match request {
                Request::Start(reply) | Request::Stop(reply) | Request::SetControl(_, _, reply) => {
                    let _ = reply.send(Err(error()));
                }
                Request::Grab(_, _, reply) => {
                    let _ = reply.send(Err(error()));
                }
                Request::Controls(reply) => {
                    let _ = reply.send(Err(error()));
                }
                Request::ControlDefaults(reply) => {
                    let _ = reply.send(Err(error()));
                }
                Request::SetSource(_) | Request::Reopen(_) | Request::Close => unreachable!(),
            }
            continue;
        };
        match request {
            Request::Start(reply) => {
                let r = if streaming {
                    Ok(())
                } else {
                    camera.open_stream().map_err(wrap).inspect(|()| streaming = true)
                };
                let _ = reply.send(r);
            }
            Request::Stop(reply) => {
                let r = if streaming {
                    camera.stop_stream().map_err(wrap).inspect(|()| streaming = false)
                } else {
                    Ok(())
                };
                let _ = reply.send(r);
            }
            Request::Grab(deadline, timeout, reply) => {
                let r =
                    grab_valid_until(deadline, timeout, || grab(camera, &mut streaming, source));
                let _ = reply.send(r);
            }
            Request::Controls(reply) => {
                let r = camera.camera_controls().map_err(wrap).map(|list| {
                    list.into_iter()
                        .map(|c| {
                            let (min, max) = match c.description() {
                                ControlValueDescription::IntegerRange { min, max, .. } => {
                                    (*min, *max)
                                }
                                _ => (0, 0),
                            };
                            let value = match c.value() {
                                ControlValueSetter::Integer(v) => v,
                                _ => 0,
                            };
                            Control { name: format!("{:?}", c.control()), min, max, value }
                        })
                        .collect()
                });
                let _ = reply.send(r);
            }
            Request::ControlDefaults(reply) => {
                let r = camera.camera_controls().map_err(wrap).map(|list| {
                    list.into_iter()
                        .filter_map(|control| {
                            let default = match control.description() {
                                ControlValueDescription::IntegerRange { default, .. }
                                | ControlValueDescription::Integer { default, .. } => *default,
                                _ => return None,
                            };
                            Some((format!("{:?}", control.control()), default))
                        })
                        .collect()
                });
                let _ = reply.send(r);
            }
            Request::SetControl(name, value, reply) => {
                let r = set_control(camera, &name, value);
                let _ = reply.send(r);
            }
            Request::SetSource(_) | Request::Reopen(_) | Request::Close => unreachable!(),
        }
    }

    if let Some(mut camera) = camera.filter(|_| streaming) {
        let _ = camera.stop_stream();
    }
}

/// Apply one control, matched on the name the platform reports.
fn set_control(camera: &mut Camera, name: &str, value: i64) -> Result<()> {
    let found = camera
        .camera_controls()
        .map_err(wrap)?
        .into_iter()
        .find(|c| format!("{:?}", c.control()).eq_ignore_ascii_case(name))
        .ok_or_else(|| Error::unsupported(format!("no camera control named {name}")))?;
    camera.set_camera_control(found.control(), ControlValueSetter::Integer(value)).map_err(wrap)
}

fn grab(camera: &mut Camera, streaming: &mut bool, source: LightSource) -> Result<Frame> {
    if !*streaming {
        camera.open_stream().map_err(wrap)?;
        *streaming = true;
    }
    let buffer = camera.frame().map_err(wrap)?;
    let resolution = buffer.resolution();

    // Hand on whatever the camera produced. MJPEG stays compressed: decoding a
    // 5 MP frame only to re-encode it for display would be work done for nobody.
    let (format, data) = match buffer.source_frame_format() {
        FrameFormat::MJPEG => {
            validate_jpeg_envelope(buffer.buffer())?;
            (PixelFormat::Jpeg, buffer.buffer().to_vec())
        }
        FrameFormat::GRAY => (PixelFormat::Gray8, buffer.buffer().to_vec()),
        // A nearest-format negotiation can select YUYV/NV12. Those bytes are
        // not RGB; decoding is required before advertising an RGB frame.
        _ => (PixelFormat::Rgb8, buffer.decode_image::<RgbFormat>().map_err(wrap)?.into_raw()),
    };

    Ok(Frame { width: resolution.width(), height: resolution.height(), format, source, data })
}

fn validate_jpeg_envelope(data: &[u8]) -> Result<()> {
    if data.len() < 4
        || !data.starts_with(&[0xff, 0xd8])
        || !data[2..].windows(2).any(|bytes| bytes == [0xff, 0xd9])
    {
        return Err(Error::Malformed(format!("incomplete MJPEG sample ({} bytes)", data.len())));
    }
    Ok(())
}

fn grab_valid_until(
    deadline: Instant,
    timeout: Duration,
    mut next: impl FnMut() -> Result<Frame>,
) -> Result<Frame> {
    loop {
        if Instant::now() >= deadline {
            return Err(Error::Timeout(timeout));
        }
        match next() {
            Err(Error::Malformed(reason)) => {
                tracing::warn!(%reason, "discarding malformed camera sample");
            }
            result => return result,
        }
    }
}

impl FrameSource for CameraSource {
    fn control_defaults(&self) -> Vec<(String, i64)> {
        CameraSource::control_defaults(self).unwrap_or_default()
    }

    fn start(&mut self) -> Result<()> {
        self.ask(Request::Start)
    }

    fn stop(&mut self) -> Result<()> {
        self.ask(Request::Stop)
    }

    fn next_frame(&mut self, timeout: Duration) -> Result<Frame> {
        if self.timed_out.load(Ordering::Acquire) {
            return Err(Error::Rejected(
                "capture timed out; discard this camera source".to_owned(),
            ));
        }
        let Some(deadline) = Instant::now().checked_add(timeout) else {
            return Err(Error::Rejected("capture timeout is too large".to_owned()));
        };
        if timeout.is_zero() {
            return Err(Error::Timeout(timeout));
        }
        let (tx, rx) = channel();
        self.tx
            .send(Request::Grab(deadline, timeout, tx))
            .map_err(|_| Error::Rejected("the capture thread has stopped".to_owned()))?;
        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                // Synchronous platform reads cannot be safely killed. Prevent
                // later lamp sequences from using this outstanding request.
                self.timed_out.store(true, Ordering::Release);
                Err(Error::Timeout(timeout))
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err(Error::Rejected("the capture thread dropped the reply".to_owned()))
            }
        }
    }

    fn set_control(&mut self, name: &str, value: i64) -> Result<()> {
        CameraSource::set_control(self, name, value)
    }

    fn reopen(&mut self) -> Result<()> {
        CameraSource::reopen(self)
    }

    fn controls(&self) -> Vec<Control> {
        // A camera that will not say what it carries is reported as carrying
        // nothing, which is what a caller does with an error here anyway.
        CameraSource::controls(self).unwrap_or_default()
    }
}

impl Drop for CameraSource {
    fn drop(&mut self) {
        let _ = self.tx.send(Request::Close);
        if let Some(worker) = self.worker.take() {
            // Do not undo next_frame's timeout by waiting forever for the same
            // platform read here. The worker closes its handle when it returns.
            if !self.timed_out.load(Ordering::Acquire) || worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_marker_and_truncated_samples() {
        for sample in [&[][..], &[0xff, 0xd8], &[0xff, 0xd8, 0xff, 0xda, 0x01]] {
            assert!(matches!(validate_jpeg_envelope(sample), Err(Error::Malformed(_))));
        }
        assert!(validate_jpeg_envelope(&[0xff, 0xd8, 0xff, 0xda, 1, 2, 0xff, 0xd9]).is_ok());
    }

    #[test]
    fn retries_malformed_sample_before_returning_valid_frame() {
        let mut attempts = 0;
        let frame = grab_valid_until(
            Instant::now() + Duration::from_secs(1),
            Duration::from_secs(1),
            || {
                attempts += 1;
                if attempts == 1 {
                    return Err(Error::Malformed("two-byte sample".to_owned()));
                }
                Ok(Frame {
                    width: 1,
                    height: 1,
                    format: PixelFormat::Rgb8,
                    source: LightSource::White,
                    data: vec![1, 2, 3],
                })
            },
        )
        .unwrap();
        assert_eq!(attempts, 2);
        assert_eq!(frame.data, [1, 2, 3]);
    }

    #[test]
    fn expired_request_does_not_touch_camera() {
        let result = grab_valid_until(Instant::now(), Duration::ZERO, || panic!("expired grab"));
        assert!(matches!(result, Err(Error::Timeout(_))));
    }

    #[test]
    fn caller_timeout_retires_source_without_waiting_for_worker() {
        let (tx, rx) = channel();
        let (release_tx, release_rx) = channel();
        let worker = std::thread::spawn(move || {
            let request = rx.recv().unwrap();
            assert!(matches!(request, Request::Grab(..)));
            // Retain the reply channel to simulate a blocked native read.
            release_rx.recv().unwrap();
            drop(request);
        });
        let mut camera = CameraSource {
            tx,
            worker: Some(worker),
            resolution: (1, 1),
            timed_out: AtomicBool::new(false),
        };
        assert!(matches!(camera.next_frame(Duration::from_millis(5)), Err(Error::Timeout(_))));
        assert!(matches!(camera.start(), Err(Error::Rejected(_))));
        // Drop must return before the simulated native read is released.
        drop(camera);
        release_tx.send(()).unwrap();
    }
}
