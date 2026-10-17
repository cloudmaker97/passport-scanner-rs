# Passport Scanner local Media Foundation fixes

Based on the unmodified crates.io `nokhwa-bindings-windows` 0.4.6 sources
(Apache-2.0). The original copyright and license notices remain in `src/lib.rs`.

Changes:

- Pair every successful `IMFMediaBuffer::Lock` with `Unlock`, including empty
  samples; copy into owned memory before unlocking.
- Initialise the sample output to `None` and stop the read loop for terminal
  error/end-of-stream flags.
- Deselect and synchronously flush the video stream when stopping. The
  upstream method only changed a boolean, retaining queued samples across
  illumination changes. Its public `stop_stream` return type remains unchanged.

API contracts:

- https://learn.microsoft.com/en-us/windows/win32/api/mfobjects/nf-mfobjects-imfmediabuffer-lock
- https://learn.microsoft.com/en-us/windows/win32/api/mfreadwrite/nf-mfreadwrite-imfsourcereader-flush
- https://learn.microsoft.com/en-us/windows/win32/api/mfreadwrite/nf-mfreadwrite-imfsourcereader-readsample

These fixes do not change camera control IDs or ranges. Synchronous native
`ReadSample` is still blocking; the capture wrapper enforces the caller's
timeout and retires a timed-out source, but does not forcibly terminate a
thread or claim that an unresponsive driver has released its device handle.
