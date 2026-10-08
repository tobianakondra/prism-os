//! PrismOS kernel console.
//!
//! ROLE:
//! Single place for ALL early kernel output. Phase 1 supports:
//!   1. Serial port COM1 (0x3F8) — primary log channel, visible in QEMU
//!      via `-serial stdio`. Always available, even without a display.
//!   2. Framebuffer clear — proves the UEFI GOP framebuffer handed over by
//!      the bootloader works, by painting a solid background color.
//!
//! Full framebuffer TEXT rendering (fonts, scrolling) is intentionally
//! deferred to Phase 2 so this file stays small and auditable.
//!
//! THREAD SAFETY:
//! QEMU Phase 1 is single-core (`-smp 1`) and interrupt-free, but we still
//! use a `spin::Mutex` around the serial port so later SMP/interrupt code
//! cannot interleave bytes. Never hold this lock across a blocking receive.
//!
//! HOST TESTS:
//! The byte-level formatting helpers are pure and tested on the host with
//! `cargo test -p kernel` (std is available for tests via cfg(test)).

use spin::Mutex;
use uart_16550::SerialPort;

/// Standard PC serial port COM1. QEMU forwards this to our terminal.
///
/// `SerialPort::new` is `unsafe` (raw port number, no validation) and runs
/// in a `static` initializer: the port number `0x3F8` is the IBM PC standard
/// for COM1, always present in QEMU. No other port is ever constructed, so
/// aliasing cannot occur. `spin::Mutex` makes later SMP/interrupt use safe.
static SERIAL1: Mutex<SerialPort> = Mutex::new(unsafe { SerialPort::new(0x3F8) });

/// Initialize the console. Must be called once from `kernel_main` before
/// any `println!`. Safe to call exactly once; calling twice re-inits serial
/// harmlessly and repaints the background.
pub fn init(framebuffer: Option<&mut bootloader_api::info::FrameBuffer>) {
    // Serial first: if framebuffer setup fails, we can still report why.
    SERIAL1.lock().init();

    // Paint the screen so a human watching QEMU with a display sees proof
    // that UEFI GOP handoff worked, even before we have font rendering.
    if let Some(fb) = framebuffer {
        paint_background(fb);
    }
}

/// Fill the whole framebuffer with PrismOS deep-space blue.
///
/// This is deliberately simple: one loop, no font, no scrolling. It validates
/// `BootInfo.framebuffer`, pixel format handling, and memory mapping without
/// adding unaudited graphics code to Phase 1.
///
/// Unknown pixel formats are left untouched (early return): a black screen
/// is better than scrambled pixels from a guessed layout. Note we use
/// `stride` (pixels per line, may include padding) rather than `width` when
/// advancing rows — but since we paint a SOLID color, padding pixels get the
/// same color and the distinction does not matter yet. It WILL matter for
/// font rendering in Phase 2, where stride must be honored per row.
fn paint_background(fb: &mut bootloader_api::info::FrameBuffer) {
    use bootloader_api::info::PixelFormat;

    let info = fb.info();

    // Decide byte order up front; bail out cleanly on exotic formats.
    let rgb_order = match info.pixel_format {
        PixelFormat::Rgb => true,
        PixelFormat::Bgr => false,
        // U8 grayscale, unknown, or future formats: do not guess.
        _ => return,
    };

    // PrismOS brand background: RGB (10, 14, 30).
    let (r, g, b) = (10u8, 14u8, 30u8);
    let bytes_per_pixel = info.bytes_per_pixel;
    let frame_buffer = fb.buffer_mut();

    // The bootloader guarantees `buffer_mut` length matches
    // `byte_len`, so chunking into pixels is safe.
    for pixel in frame_buffer.chunks_exact_mut(bytes_per_pixel) {
        if rgb_order {
            pixel[0] = r;
            pixel[1] = g;
            pixel[2] = b;
        } else {
            pixel[0] = b;
            pixel[1] = g;
            pixel[2] = r;
        }
        // Zero any padding/alpha byte (some formats use 4 bytes per pixel).
        for byte in pixel.iter_mut().skip(3) {
            *byte = 0;
        }
    }
}

/// Write raw bytes to serial. Internal helper behind the macros below.
#[doc(hidden)]
pub fn _serial_write_bytes(bytes: &[u8]) {
    let mut serial = SERIAL1.lock();
    for &b in bytes {
        serial.send(b);
    }
}

/// Try to read one byte from serial without blocking.
///
/// Returns `None` when no key has arrived. Used by the shell so the kernel
/// can keep scheduling demo tasks while waiting for input.
///
/// NOTE on the `uart_16550` 0.3 API: `try_receive()` returns
/// `Result<u8, WouldBlockError>` — `Ok(byte)` when data is ready,
/// `Err` when the input buffer is empty. `.ok()` converts that to
/// `Option<u8>`, which is exactly the polling semantic we want.
pub fn try_read_byte() -> Option<u8> {
    SERIAL1.lock().try_receive().ok()
}

/// Serial print macro (no newline).
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        // Formatting needs `alloc`/`std` normally, but `core::fmt` works
        // in `no_std` with a small adapter. We keep it explicit so every
        // contributor sees there is no hidden heap allocation here.
        use core::fmt::Write as _;
        struct SerialWriter;
        impl core::fmt::Write for SerialWriter {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                $crate::console::_serial_write_bytes(s.as_bytes());
                Ok(())
            }
        }
        let _ = core::write!(SerialWriter, $($arg)*);
    }};
}

/// Serial print macro with newline.
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {{
        $crate::print!($($arg)*);
        $crate::print!("\n");
    }};
}
