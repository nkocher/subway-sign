//! LED matrix display abstraction with hardware FFI.
//!
//! The hardware path uses hzeller's `set_image()` C API for bulk pixel transfer.
//! This is a direct FFI call to the rpi-rgb-led-matrix library that copies an
//! entire RGB framebuffer to the LED canvas in one operation, reducing per-frame
//! FFI overhead from 6,144 calls (one per pixel in 192x32) to 1.
//!
//! ## LedCanvas layout assumption
//!
//! `LedCanvas` (from the `rpi-led-matrix` crate) is a newtype wrapping a raw
//! `*mut CLedCanvas` pointer. We extract this pointer via transmute-style casting
//! to pass it to the C `set_image()` function. A `debug_assert_eq!` on
//! `size_of::<LedCanvas>()` guards against crate updates that change the layout.
//!
//! ## `unsafe impl Send` justification
//!
//! `LedMatrixDisplay` is created, used, and destroyed entirely within one
//! dedicated render thread (`std::thread::spawn` in `main.rs`). It is never
//! shared across threads. The `Send` bound is required because `Box<dyn
//! DisplayTarget>` is moved into that thread, but no concurrent access occurs.

use super::framebuffer::FrameBuffer;

/// Abstraction over the LED matrix hardware.
///
/// The `mock` feature (default) uses `MockDisplay` (no-op).
/// The `hardware` feature uses `LedMatrixDisplay` (real Pi hardware).
pub trait DisplayTarget: Send {
    /// Push a rendered frame to the display.
    fn swap(&mut self, frame: &FrameBuffer);

    /// Update display brightness (0-100).
    fn set_brightness(&mut self, brightness: u8);
}

// ---------------------------------------------------------------------------
// Hardware implementation (Pi only, behind `hardware` feature flag)
// ---------------------------------------------------------------------------
#[cfg(feature = "hardware")]
mod hw {
    use super::{DisplayTarget, FrameBuffer};
    use rpi_led_matrix::{LedCanvas, LedMatrix, LedMatrixOptions, LedRuntimeOptions};

    // Direct FFI to hzeller's C API.
    // set_image: bulk pixel transfer (reduces per-frame FFI from 6,144 calls to 1).
    // led_matrix_{set,get}_brightness: runtime brightness control (not exposed by crate).
    extern "C" {
        fn set_image(
            canvas: *mut std::ffi::c_void,
            canvas_offset_x: std::ffi::c_int,
            canvas_offset_y: std::ffi::c_int,
            image_buffer: *const u8,
            buffer_size_bytes: usize,
            image_width: std::ffi::c_int,
            image_height: std::ffi::c_int,
            is_bgr: std::ffi::c_char,
        );
        fn led_matrix_set_brightness(matrix: *mut std::ffi::c_void, brightness: u8);
        fn led_matrix_get_brightness(matrix: *mut std::ffi::c_void) -> u8;
    }

    /// Real LED matrix display using hzeller's rpi-rgb-led-matrix via the
    /// `rpi-led-matrix` crate.
    pub struct LedMatrixDisplay {
        matrix: LedMatrix,
        canvas: Option<LedCanvas>,
        matrix_ptr: *mut std::ffi::c_void,
    }

    impl LedMatrixDisplay {
        /// Create and configure the LED matrix with our panel layout:
        /// 3 chained 64x32 panels = 192x32.
        pub fn new(brightness: u8) -> Self {
            let mut options = LedMatrixOptions::new();
            let _ = options.set_rows(32);
            let _ = options.set_cols(64);
            let _ = options.set_chain_length(3);
            let _ = options.set_hardware_mapping("regular");
            let _ = options.set_pwm_bits(11);
            let _ = options.set_pwm_lsb_nanoseconds(130);
            let _ = options.set_pwm_dither_bits(0);
            let _ = options.set_limit_refresh(120);
            options.set_hardware_pulsing(true);
            let _ = options.set_brightness(brightness);
            options.set_refresh_rate(false); // suppress Hz spam on stdout

            let mut rt_options = LedRuntimeOptions::new();
            let _ = rt_options.set_gpio_slowdown(3);
            let _ = rt_options.set_drop_privileges(false);

            let matrix = LedMatrix::new(Some(options), Some(rt_options))
                .expect("Failed to initialize LED matrix");

            let canvas = matrix.offscreen_canvas();

            // Extract raw matrix pointer for runtime brightness control.
            // LedMatrix layout: first field is `handle: *mut CLedMatrix`.
            // Verified by reading back the brightness we just set.
            let matrix_ptr = unsafe {
                *(&matrix as *const LedMatrix as *const *mut std::ffi::c_void)
            };
            let readback = unsafe { led_matrix_get_brightness(matrix_ptr) };
            assert_eq!(
                readback, brightness,
                "LedMatrix pointer extraction failed — brightness mismatch ({} != {})",
                readback, brightness
            );

            tracing::info!(
                "LED matrix initialized (192x32, brightness={}%, pulsing=hw, pwm={}/{}ns, dither=0, refresh_cap=120Hz)",
                brightness, 11, 130
            );

            LedMatrixDisplay {
                matrix,
                canvas: Some(canvas),
                matrix_ptr,
            }
        }
    }

    // Safety: LedMatrixDisplay is created, used, and destroyed entirely within
    // the dedicated render thread (std::thread::spawn in main.rs). It is never
    // shared across threads. The Send bound is required by DisplayTarget
    // (Box<dyn DisplayTarget> is moved into that thread), but no concurrent
    // access occurs.
    unsafe impl Send for LedMatrixDisplay {}

    impl DisplayTarget for LedMatrixDisplay {
        fn swap(&mut self, frame: &FrameBuffer) {
            if let Some(canvas) = self.canvas.take() {
                let pixels = frame.raw_pixels();
                let width = frame.width();
                let height = frame.height();

                // Extract the raw C pointer from LedCanvas.
                // Safety: LedCanvas is a single-field newtype wrapping
                // *mut CLedCanvas. We read the pointer value without moving
                // or dropping the LedCanvas struct. The size assertion
                // verifies this assumption holds.
                assert_eq!(
                    std::mem::size_of::<LedCanvas>(),
                    std::mem::size_of::<*mut std::ffi::c_void>(),
                    "LedCanvas layout changed — set_image FFI assumption broken"
                );
                let canvas_ptr: *mut std::ffi::c_void = unsafe {
                    *(&canvas as *const LedCanvas as *const *mut std::ffi::c_void)
                };

                // Bulk copy the entire framebuffer in one FFI call.
                // Safety: canvas_ptr is valid (just extracted from live LedCanvas),
                // pixels buffer is valid for its length, dimensions match our
                // framebuffer layout (192x32 RGB, row-major).
                unsafe {
                    set_image(
                        canvas_ptr,
                        0,
                        0,
                        pixels.as_ptr(),
                        pixels.len(),
                        width as std::ffi::c_int,
                        height as std::ffi::c_int,
                        0, // is_bgr = false (our buffer is RGB)
                    );
                }

                // swap() returns the previously-displayed canvas for reuse
                self.canvas = Some(self.matrix.swap(canvas));
            }
        }

        fn set_brightness(&mut self, brightness: u8) {
            unsafe { led_matrix_set_brightness(self.matrix_ptr, brightness); }
        }
    }
}

// ---------------------------------------------------------------------------
// Mock implementation (macOS dev)
// ---------------------------------------------------------------------------
/// Mock display for development on macOS (no hardware).
pub struct MockDisplay;

impl MockDisplay {
    pub fn new(brightness: u8) -> Self {
        tracing::info!(
            "Mock display initialized (192x32, brightness={})",
            brightness
        );
        MockDisplay
    }
}

impl DisplayTarget for MockDisplay {
    fn swap(&mut self, _frame: &FrameBuffer) {}

    fn set_brightness(&mut self, _brightness: u8) {}
}

// ---------------------------------------------------------------------------
// Factory function
// ---------------------------------------------------------------------------

/// Create the appropriate display target based on compile-time features.
#[cfg(feature = "hardware")]
pub fn create_display(brightness: u8) -> Box<dyn DisplayTarget> {
    Box::new(hw::LedMatrixDisplay::new(brightness))
}

#[cfg(not(feature = "hardware"))]
pub fn create_display(brightness: u8) -> Box<dyn DisplayTarget> {
    Box::new(MockDisplay::new(brightness))
}
