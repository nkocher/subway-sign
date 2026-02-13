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
    use rpi_led_matrix::{LedCanvas, LedColor, LedMatrix, LedMatrixOptions, LedRuntimeOptions};

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
            options.set_rows(32);
            options.set_cols(64);
            options.set_chain_length(3);
            options.set_hardware_mapping("regular");
            options.set_pwm_bits(11);
            options.set_pwm_lsb_nanoseconds(130);
            options.set_brightness(brightness);

            let mut rt_options = LedRuntimeOptions::new();
            rt_options.set_gpio_slowdown(3);
            rt_options.set_drop_privileges(false);

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

    impl DisplayTarget for LedMatrixDisplay {
        fn swap(&mut self, frame: &FrameBuffer) {
            if let Some(mut canvas) = self.canvas.take() {
                let pixels = frame.raw_pixels();
                let width = frame.width();
                let height = frame.height();

                for y in 0..height {
                    for x in 0..width {
                        let idx = (y * width + x) * 3;
                        canvas.set(
                            x as i32,
                            y as i32,
                            &LedColor {
                                red: pixels[idx],
                                green: pixels[idx + 1],
                                blue: pixels[idx + 2],
                            },
                        );
                    }
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
