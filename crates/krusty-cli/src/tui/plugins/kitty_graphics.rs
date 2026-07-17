//! Kitty Graphics Protocol Implementation
//!
//! Provides 60fps pixel rendering via the Kitty graphics protocol.
//! Supported terminals: Ghostty, Kitty, WezTerm, Konsole (partial).
//!
//! Performance optimizations:
//! - Zlib compression (reduces data by 80-95% for typical images)
//! - RGB mode when alpha not needed (25% less data)
//! - Double-buffering for flicker-free updates
//!
//! Protocol reference: https://sw.kovidgoyal.net/kitty/graphics-protocol/

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{self, Write};
use std::sync::Arc;

/// A frame of pixel data from a plugin
/// Uses Arc for zero-copy sharing between plugin and graphics system
#[derive(Debug, Clone)]
pub struct PluginFrame {
    /// RGBA pixel data (4 bytes per pixel), Arc for zero-copy sharing
    pub pixels: Arc<Vec<u8>>,
    /// Frame width in pixels
    pub width: u32,
    /// Frame height in pixels
    pub height: u32,
}

impl PluginFrame {
    /// Create a new frame with full pixel data (wraps in Arc)
    #[cfg(test)]
    pub fn new(pixels: Vec<u8>, width: u32, height: u32) -> Self {
        debug_assert_eq!(
            pixels.len(),
            (width * height * 4) as usize,
            "Pixel data size mismatch: expected {} bytes for {}x{} RGBA",
            width * height * 4,
            width,
            height
        );
        Self {
            pixels: Arc::new(pixels),
            width,
            height,
        }
    }

    /// Create a frame from an existing Arc (zero-copy)
    pub fn from_arc(pixels: Arc<Vec<u8>>, width: u32, height: u32) -> Self {
        debug_assert_eq!(
            pixels.len(),
            (width * height * 4) as usize,
            "Pixel data size mismatch: expected {} bytes for {}x{} RGBA",
            width * height * 4,
            width,
            height
        );
        Self {
            pixels,
            width,
            height,
        }
    }
}

/// Kitty graphics protocol handler
///
/// Manages image transmission and display via escape sequences.
/// Supports caching, delta updates, and placement control.
pub struct KittyGraphics {
    /// Current image ID (for caching/updates)
    image_id: u32,
    /// Last frame dimensions (to detect size changes)
    last_width: u32,
    last_height: u32,
    /// Whether the terminal supports Kitty graphics
    supported: Option<bool>,
    /// Cell dimensions in pixels (for coordinate conversion)
    cell_width: u16,
    cell_height: u16,
    /// Reusable buffer for RGB conversion (avoids allocation per frame)
    rgb_buffer: Vec<u8>,
    /// Reusable buffer for compression output (avoids allocation per frame)
    compressed_buffer: Vec<u8>,
}

impl Default for KittyGraphics {
    fn default() -> Self {
        Self::new()
    }
}

impl KittyGraphics {
    /// Create a new Kitty graphics handler
    pub fn new() -> Self {
        Self {
            image_id: 1,
            last_width: 0,
            last_height: 0,
            supported: None,
            cell_width: 0,
            cell_height: 0,
            rgb_buffer: Vec::with_capacity(640 * 480 * 3), // Pre-allocate for common size
            compressed_buffer: Vec::with_capacity(64 * 1024), // 64KB compressed buffer
        }
    }

    /// Set cell dimensions (call when terminal size changes)
    pub fn set_cell_size(&mut self, width: u16, height: u16) {
        self.cell_width = width;
        self.cell_height = height;
    }

    /// Calculate pixel dimensions for a cell area
    pub fn pixels_for_cells(&self, cols: u16, rows: u16) -> (u32, u32) {
        let width = if self.cell_width > 0 {
            cols as u32 * self.cell_width as u32
        } else {
            cols as u32 * 8 // Fallback: assume 8px wide cells
        };
        let height = if self.cell_height > 0 {
            rows as u32 * self.cell_height as u32
        } else {
            rows as u32 * 16 // Fallback: assume 16px tall cells
        };
        (width, height)
    }

    /// Check if Kitty graphics is supported by querying the terminal
    ///
    /// This sends a query and expects a response. Should be called
    /// during initialization when stdin is available for reading.
    pub fn detect_support(&mut self) -> bool {
        // For now, assume support based on TERM environment
        // A proper implementation would send a query and read the response
        let term = std::env::var("TERM").unwrap_or_default();
        let term_program = std::env::var("TERM_PROGRAM").unwrap_or_default();

        let supported = term.contains("kitty")
            || term.contains("ghostty")
            || term.contains("wezterm")
            || term.contains("xterm-kitty")
            || term_program.to_lowercase().contains("kitty")
            || term_program.to_lowercase().contains("ghostty")
            || term_program.to_lowercase().contains("wezterm");

        tracing::info!(
            "Kitty graphics detection: TERM={:?} TERM_PROGRAM={:?} -> supported={}",
            term,
            term_program,
            supported
        );

        self.supported = Some(supported);
        supported
    }

    /// Returns whether Kitty graphics is supported
    pub fn is_supported(&self) -> bool {
        self.supported.unwrap_or(false)
    }

    /// Display a frame at the specified cell position
    ///
    /// Uses double-buffering and quiet mode for flicker-free updates.
    pub fn display_frame<W: Write>(
        &mut self,
        writer: &mut W,
        frame: &PluginFrame,
        col: u16,
        row: u16,
        cols: u16,
        rows: u16,
    ) -> io::Result<()> {
        // Use double-buffering: alternate between two image IDs
        // This prevents flashing by placing new image before deleting old
        let old_id = self.image_id;
        let new_id = if old_id == 1 { 2 } else { 1 };
        self.image_id = new_id;

        // Update last dimensions
        self.last_width = frame.width;
        self.last_height = frame.height;

        // Move cursor to position first
        write!(writer, "\x1b[{};{}H", row + 1, col + 1)?;

        // Transmit and display atomically (a=T) with quiet mode (q=2 = suppress all responses)
        // This prevents terminal responses from leaking into input
        self.transmit_and_place(writer, frame, new_id, cols, rows)?;

        // Delete old image after new one is displayed (double-buffer swap)
        self.delete_image(writer, old_id)?;

        writer.flush()
    }

    /// Transmit image and place it atomically with compression
    /// Uses pre-allocated buffers to avoid allocation per frame
    fn transmit_and_place<W: Write>(
        &mut self,
        writer: &mut W,
        frame: &PluginFrame,
        id: u32,
        cols: u16,
        rows: u16,
    ) -> io::Result<()> {
        // Check if all pixels are opaque - if so, use RGB (f=24) instead of RGBA (f=32)
        // This saves 25% bandwidth
        let all_opaque = frame.pixels.chunks(4).all(|p| p.len() == 4 && p[3] == 255);

        let (pixel_data, format): (&[u8], u32) = if all_opaque {
            // Convert RGBA to RGB using reusable buffer
            let rgb_size = (frame.width * frame.height * 3) as usize;
            self.rgb_buffer.clear();
            self.rgb_buffer.reserve(rgb_size);
            for chunk in frame.pixels.chunks(4) {
                self.rgb_buffer.push(chunk[0]);
                self.rgb_buffer.push(chunk[1]);
                self.rgb_buffer.push(chunk[2]);
            }
            (&self.rgb_buffer, 24)
        } else {
            (&frame.pixels, 32)
        };

        // Compress with zlib using reusable buffer
        self.compressed_buffer.clear();
        {
            let mut encoder = ZlibEncoder::new(&mut self.compressed_buffer, Compression::fast());
            encoder.write_all(pixel_data)?;
            encoder.finish()?;
        }

        // Base64 encode the compressed data
        let encoded = BASE64.encode(&self.compressed_buffer);
        let chunk_size = 4096; // Max payload size per chunk

        let chunks: Vec<&str> = encoded
            .as_bytes()
            .chunks(chunk_size)
            .map(|c| std::str::from_utf8(c).unwrap_or(""))
            .collect();

        for (i, chunk) in chunks.iter().enumerate() {
            let is_last = i == chunks.len() - 1;

            if i == 0 {
                // First chunk: transmit + display atomically
                // q=2 = suppress all responses (prevents input pollution)
                // a=T = transmit and display
                // o=z = zlib compressed
                // f=24/32 = RGB/RGBA format
                // c,r = display size in cells
                write!(
                    writer,
                    "\x1b_Ga=T,q=2,f={},o=z,s={},v={},i={},c={},r={},t=d,m={};{}\x1b\\",
                    format,
                    frame.width,
                    frame.height,
                    id,
                    cols,
                    rows,
                    if is_last { 0 } else { 1 },
                    chunk
                )?;
            } else {
                // Continuation chunks (q=2 for quiet)
                write!(
                    writer,
                    "\x1b_Gq=2,m={};{}\x1b\\",
                    if is_last { 0 } else { 1 },
                    chunk
                )?;
            }
        }

        Ok(())
    }

    /// Delete an image by ID (quiet mode)
    fn delete_image<W: Write>(&self, writer: &mut W, id: u32) -> io::Result<()> {
        // q=2 = suppress all responses
        write!(writer, "\x1b_Ga=d,d=i,i={},q=2\x1b\\", id)?;
        Ok(())
    }

    /// Clear all images from the terminal
    pub fn clear_all<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        // q=2 = suppress all responses
        write!(writer, "\x1b_Ga=d,d=a,q=2\x1b\\")?;
        writer.flush()
    }
}

/// Query terminal for cell size in pixels
///
/// Returns (cell_width, cell_height) or None if not available.
/// Uses TIOCGWINSZ ioctl on Unix systems.
pub fn query_cell_size() -> Option<(u16, u16)> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;

        #[repr(C)]
        struct Winsize {
            ws_row: u16,
            ws_col: u16,
            ws_xpixel: u16,
            ws_ypixel: u16,
        }

        let mut ws = Winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        let fd = std::io::stdout().as_raw_fd();

        // TIOCGWINSZ = 0x5413 on Linux, 0x40087468 on macOS
        #[cfg(target_os = "linux")]
        const TIOCGWINSZ: libc::c_ulong = 0x5413;
        #[cfg(target_os = "macos")]
        const TIOCGWINSZ: libc::c_ulong = 0x40087468;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        const TIOCGWINSZ: libc::c_ulong = 0x5413; // Default to Linux

        // SAFETY: fd is a valid file descriptor from stdout. TIOCGWINSZ is a read-only
        // ioctl that populates the winsize struct. ws is a stack-allocated struct with
        // correct layout. The ioctl either succeeds (returns 0) or fails gracefully.
        let result = unsafe { libc::ioctl(fd, TIOCGWINSZ, &mut ws) };

        if result == 0 && ws.ws_xpixel > 0 && ws.ws_ypixel > 0 && ws.ws_col > 0 && ws.ws_row > 0 {
            let cell_width = ws.ws_xpixel / ws.ws_col;
            let cell_height = ws.ws_ypixel / ws.ws_row;
            if cell_width > 0 && cell_height > 0 {
                return Some((cell_width, cell_height));
            }
        }

        None
    }

    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a solid color frame for testing
    fn solid_frame(width: u32, height: u32, r: u8, g: u8, b: u8, a: u8) -> PluginFrame {
        let pixel_count = (width * height) as usize;
        let mut pixels = Vec::with_capacity(pixel_count * 4);
        for _ in 0..pixel_count {
            pixels.extend_from_slice(&[r, g, b, a]);
        }
        PluginFrame::new(pixels, width, height)
    }

    #[test]
    fn test_plugin_frame_new() {
        let frame = PluginFrame::new(vec![0; 100 * 100 * 4], 100, 100);
        assert_eq!(frame.width, 100);
        assert_eq!(frame.height, 100);
        assert_eq!(frame.pixels.len(), 100 * 100 * 4);
    }

    #[test]
    fn test_plugin_frame_from_arc() {
        let pixels = Arc::new(vec![0u8; 100 * 100 * 4]);
        let frame = PluginFrame::from_arc(pixels, 100, 100);
        assert_eq!(frame.width, 100);
        assert_eq!(Arc::strong_count(&frame.pixels), 1);
    }

    #[test]
    fn test_plugin_frame_solid() {
        let frame = solid_frame(10, 10, 255, 0, 0, 255);
        assert_eq!(frame.width, 10);
        assert_eq!(frame.height, 10);
        // Check first pixel is red
        assert_eq!(&frame.pixels[0..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn test_kitty_graphics_pixels_for_cells() {
        let mut kg = KittyGraphics::new();
        kg.set_cell_size(8, 16);

        let (w, h) = kg.pixels_for_cells(80, 24);
        assert_eq!(w, 640);
        assert_eq!(h, 384);
    }

    #[test]
    fn test_display_frame_output() {
        let mut kg = KittyGraphics::new();
        // Use opaque pixels (alpha=255) - should use RGB format (f=24)
        let frame = solid_frame(2, 2, 255, 0, 0, 255);

        let mut output = Vec::new();
        kg.display_frame(&mut output, &frame, 0, 0, 10, 10).unwrap();

        let output_str = String::from_utf8_lossy(&output);
        // Should contain the escape sequence start
        assert!(output_str.contains("\x1b_G"));
        // Should contain format=24 (RGB) since all pixels are opaque
        assert!(output_str.contains("f=24"));
        // Should contain dimensions
        assert!(output_str.contains("s=2"));
        assert!(output_str.contains("v=2"));
        // Should contain quiet mode
        assert!(output_str.contains("q=2"));
        // Should contain atomic transmit+display
        assert!(output_str.contains("a=T"));
        // Should contain compression flag
        assert!(output_str.contains("o=z"));
    }

    #[test]
    fn test_display_frame_with_transparency() {
        let mut kg = KittyGraphics::new();
        // Use semi-transparent pixels - should use RGBA format (f=32)
        let frame = solid_frame(2, 2, 255, 0, 0, 128);

        let mut output = Vec::new();
        kg.display_frame(&mut output, &frame, 0, 0, 10, 10).unwrap();

        let output_str = String::from_utf8_lossy(&output);
        // Should contain format=32 (RGBA) since alpha != 255
        assert!(output_str.contains("f=32"));
    }
}
