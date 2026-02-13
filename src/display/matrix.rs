use super::framebuffer::FrameBuffer;

/// Abstraction over the LED matrix hardware.
///
/// The `mock` feature (default) uses `MockDisplay` (no-op).
/// The `hardware` feature uses `LedMatrixDisplay` (real Pi hardware).
pub trait DisplayTarget: Send {
    /// Push a rendered frame to the display.
    fn swap(&mut self, frame: &FrameBuffer);

    /// Set display brightness (0-100).
    fn set_brightness(&mut self, brightness: u8);
}

// ---------------------------------------------------------------------------
// Hardware implementation (Pi only, behind `hardware` feature flag)
// ---------------------------------------------------------------------------
#[cfg(feature = "hardware")]
mod hw {
    use super::{DisplayTarget, FrameBuffer};
    use rpi_led_matrix::{LedCanvas, LedMatrix, LedMatrixOptions, LedRuntimeOptions};

    // Direct FFI to hzeller's C API for bulk pixel transfer.
    // The rpi-led-matrix crate only exposes per-pixel set(), but the C library
    // has set_image() which copies an entire RGB buffer in one call.
    // This reduces FFI overhead from 6,144 calls/frame to 1 call/frame.
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
    }

    /// Real LED matrix display using hzeller's rpi-rgb-led-matrix via the
    /// `rpi-led-matrix` crate.
    pub struct LedMatrixDisplay {
        matrix: LedMatrix,
        canvas: Option<LedCanvas>,
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
            let _ = options.set_brightness(brightness);
            options.set_refresh_rate(false); // suppress Hz spam on stdout

            let mut rt_options = LedRuntimeOptions::new();
            let _ = rt_options.set_gpio_slowdown(3);
            let _ = rt_options.set_drop_privileges(false);

            let matrix = LedMatrix::new(Some(options), Some(rt_options))
                .expect("Failed to initialize LED matrix");

            let canvas = matrix.offscreen_canvas();

            tracing::info!(
                "LED matrix initialized (192x32, brightness={}%)",
                brightness
            );

            LedMatrixDisplay {
                matrix,
                canvas: Some(canvas),
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
                debug_assert_eq!(
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
            // Brightness can only be set via LedMatrixOptions at init time
            // with the rpi-led-matrix crate. Log the request for debugging.
            tracing::warn!(
                "Brightness change to {}% requested but requires matrix re-init (not supported at runtime)",
                brightness
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Mock implementation (macOS dev)
// ---------------------------------------------------------------------------
/// Mock display for development on macOS (no hardware).
pub struct MockDisplay {
    #[allow(dead_code)]
    brightness: u8,
    frame_count: u64,
}

impl MockDisplay {
    pub fn new(brightness: u8) -> Self {
        tracing::info!(
            "Mock display initialized (192x32, brightness={})",
            brightness
        );
        MockDisplay {
            brightness,
            frame_count: 0,
        }
    }
}

impl DisplayTarget for MockDisplay {
    fn swap(&mut self, _frame: &FrameBuffer) {
        self.frame_count += 1;
    }

    fn set_brightness(&mut self, brightness: u8) {
        self.brightness = brightness;
    }
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
