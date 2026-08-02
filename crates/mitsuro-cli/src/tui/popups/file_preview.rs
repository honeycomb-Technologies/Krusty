//! File preview popup - renders images and PDFs in terminal

use image::DynamicImage;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use ratatui_image::{protocol::StatefulProtocol, Resize, StatefulImage};
use std::path::PathBuf;
use std::process::Command;

use super::common::{
    center_content, center_rect, popup_block, popup_title, render_popup_background,
};
use crate::tui::graphics::GraphicsContext;
use crate::tui::themes::Theme;

/// File type for preview
#[derive(Debug, Clone, PartialEq)]
pub enum PreviewFileType {
    Image,
    Pdf,
}

/// File preview popup state
pub struct FilePreviewPopup {
    /// Path to the file being previewed
    pub file_path: Option<PathBuf>,
    /// Display name shown in title
    pub display_name: String,
    /// Type of file
    pub file_type: PreviewFileType,
    /// Loaded image (for images or rendered PDF page)
    image: Option<DynamicImage>,
    /// Cached protocol state for rendering
    protocol: Option<StatefulProtocol>,
    /// Current page (for PDFs, 0-indexed)
    pub current_page: usize,
    /// Total pages (for PDFs)
    pub total_pages: usize,
    /// File size in bytes
    pub file_size: u64,
    /// Error message if loading failed
    pub error: Option<String>,
    /// Graphics context for rendering
    graphics_ctx: Option<GraphicsContext>,
}

impl Default for FilePreviewPopup {
    fn default() -> Self {
        Self::new()
    }
}

impl FilePreviewPopup {
    pub fn new() -> Self {
        Self {
            file_path: None,
            display_name: String::new(),
            file_type: PreviewFileType::Image,
            image: None,
            protocol: None,
            current_page: 0,
            total_pages: 1,
            file_size: 0,
            error: None,
            graphics_ctx: None,
        }
    }

    /// Initialize graphics context (call once at startup)
    pub fn init_graphics(&mut self) {
        self.graphics_ctx = Some(GraphicsContext::detect());
    }

    /// Open preview for a file
    pub fn open(&mut self, path: PathBuf) {
        self.file_path = Some(path.clone());
        self.display_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        self.error = None;
        self.current_page = 0;
        self.protocol = None;

        // Detect file type
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        self.file_type = if ext == "pdf" {
            PreviewFileType::Pdf
        } else {
            PreviewFileType::Image
        };

        // Get file size
        self.file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

        // Load the file
        self.load_file();
    }

    /// Load file content
    fn load_file(&mut self) {
        let path = match &self.file_path {
            Some(p) => p.clone(),
            None => return,
        };

        match self.file_type {
            PreviewFileType::Image => self.load_image(&path),
            PreviewFileType::Pdf => self.load_pdf_page(&path, self.current_page),
        }
    }

    /// Load an image file
    fn load_image(&mut self, path: &PathBuf) {
        match image::open(path) {
            Ok(img) => {
                self.image = Some(img);
                self.create_protocol();
            }
            Err(e) => {
                self.error = Some(format!("Failed to load image: {}", e));
                self.image = None;
            }
        }
    }

    /// Load a PDF page as image using system tools (pdftoppm or magick)
    fn load_pdf_page(&mut self, path: &PathBuf, page: usize) {
        // Get page count first using pdfinfo or qpdf
        self.total_pages = self.get_pdf_page_count(path).unwrap_or(1);

        if self.total_pages == 0 {
            self.error = Some("PDF has no pages".to_string());
            return;
        }

        // Try pdftoppm first (from poppler-utils), then magick
        let temp_dir = std::env::temp_dir();
        let output_base = temp_dir.join(format!("mitsuro_pdf_{}", std::process::id()));

        // pdftoppm uses 1-indexed pages
        let page_num = page + 1;

        // Try pdftoppm (most common on Linux)
        let pdftoppm_result = Command::new("pdftoppm")
            .args([
                "-png",
                "-f",
                &page_num.to_string(),
                "-l",
                &page_num.to_string(),
                "-r",
                "150", // 150 DPI for good quality
                "-singlefile",
                path.to_str().unwrap_or(""),
                output_base.to_str().unwrap_or(""),
            ])
            .output();

        let output_path = PathBuf::from(format!("{}.png", output_base.display()));

        match pdftoppm_result {
            Ok(output) if output.status.success() && output_path.exists() => {
                // Load the rendered image
                match image::open(&output_path) {
                    Ok(img) => {
                        self.image = Some(img);
                        self.create_protocol();
                    }
                    Err(e) => {
                        self.error = Some(format!("Failed to load rendered page: {}", e));
                    }
                }
                // Clean up temp file
                let _ = std::fs::remove_file(&output_path);
                return;
            }
            _ => {}
        }

        // Try ImageMagick's magick (or convert on older systems)
        let magick_output = temp_dir.join(format!("mitsuro_pdf_{}.png", std::process::id()));
        let input_spec = format!("{}[{}]", path.display(), page);

        let magick_result = Command::new("magick")
            .args([
                &input_spec,
                "-density",
                "150",
                "-background",
                "white",
                "-alpha",
                "remove",
                magick_output.to_str().unwrap_or(""),
            ])
            .output();

        match magick_result {
            Ok(output) if output.status.success() && magick_output.exists() => {
                match image::open(&magick_output) {
                    Ok(img) => {
                        self.image = Some(img);
                        self.create_protocol();
                    }
                    Err(e) => {
                        self.error = Some(format!("Failed to load rendered page: {}", e));
                    }
                }
                let _ = std::fs::remove_file(&magick_output);
                return;
            }
            _ => {}
        }

        // No PDF renderer available
        self.error = Some("No PDF renderer found. Install poppler-utils or imagemagick. Press O to open externally.".to_string());
    }

    /// Get PDF page count using pdfinfo or qpdf
    fn get_pdf_page_count(&self, path: &PathBuf) -> Option<usize> {
        // Try pdfinfo first
        if let Ok(output) = Command::new("pdfinfo").arg(path).output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if line.starts_with("Pages:") {
                        if let Some(count) = line.split_whitespace().nth(1) {
                            return count.parse().ok();
                        }
                    }
                }
            }
        }

        // Try qpdf
        if let Ok(output) = Command::new("qpdf")
            .args(["--show-npages", path.to_str().unwrap_or("")])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                return stdout.trim().parse().ok();
            }
        }

        // Default to 1 if we can't determine
        Some(1)
    }

    /// Create rendering protocol from loaded image
    fn create_protocol(&mut self) {
        let image = match &self.image {
            Some(img) => img.clone(),
            None => return,
        };

        let picker = match &self.graphics_ctx {
            Some(ctx) => match &ctx.picker {
                Some(p) => p,
                None => {
                    self.error = Some("No graphics support detected".to_string());
                    return;
                }
            },
            None => {
                self.error = Some("Graphics not initialized".to_string());
                return;
            }
        };

        self.protocol = Some(picker.new_resize_protocol(image));
    }

    /// Navigate to next PDF page
    pub fn next_page(&mut self) {
        if self.file_type == PreviewFileType::Pdf && self.current_page < self.total_pages - 1 {
            self.current_page += 1;
            self.load_file();
        }
    }

    /// Navigate to previous PDF page
    pub fn prev_page(&mut self) {
        if self.file_type == PreviewFileType::Pdf && self.current_page > 0 {
            self.current_page -= 1;
            self.load_file();
        }
    }

    /// Open file in external viewer
    pub fn open_external(&self) {
        if let Some(path) = &self.file_path {
            let _ = open::that(path);
        }
    }

    /// Calculate centered area for image within content area
    fn calculate_centered_image_area(&self, content_area: Rect) -> Rect {
        let Some(img) = &self.image else {
            return content_area;
        };

        let img_width = img.width();
        let img_height = img.height();

        // Estimate terminal cell size (typical: 8x16 pixels per cell)
        let cell_width = 8u32;
        let cell_height = 16u32;

        // Available space in pixels (approximate)
        let avail_width = content_area.width as u32 * cell_width;
        let avail_height = content_area.height as u32 * cell_height;

        // Calculate scale to fit
        let scale_x = avail_width as f32 / img_width as f32;
        let scale_y = avail_height as f32 / img_height as f32;
        let scale = scale_x.min(scale_y).min(1.0); // Don't upscale

        // Scaled image size in cells
        let scaled_width = ((img_width as f32 * scale) / cell_width as f32).ceil() as u16;
        let scaled_height = ((img_height as f32 * scale) / cell_height as f32).ceil() as u16;

        // Center within content area
        let x_offset = content_area
            .width
            .saturating_sub(scaled_width)
            .saturating_div(2);
        let y_offset = content_area
            .height
            .saturating_sub(scaled_height)
            .saturating_div(2);

        Rect {
            x: content_area.x + x_offset,
            y: content_area.y + y_offset,
            width: scaled_width.min(content_area.width),
            height: scaled_height.min(content_area.height),
        }
    }

    /// Format file size for display
    fn format_size(&self) -> String {
        if self.file_size < 1024 {
            format!("{} B", self.file_size)
        } else if self.file_size < 1024 * 1024 {
            format!("{:.1} KB", self.file_size as f64 / 1024.0)
        } else {
            format!("{:.1} MB", self.file_size as f64 / (1024.0 * 1024.0))
        }
    }

    /// Render the popup
    pub fn render(&mut self, f: &mut Frame, theme: &Theme) {
        // Use 80% of screen
        let area = f.area();
        let w = (area.width as f32 * 0.8) as u16;
        let h = (area.height as f32 * 0.8) as u16;
        let popup_area = center_rect(w, h, area);

        render_popup_background(f, popup_area, theme);

        let block = popup_block(theme);
        let inner = block.inner(popup_area);
        f.render_widget(block, popup_area);

        // Layout: Title | Content | Footer
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Title
                Constraint::Min(5),    // Content (image)
                Constraint::Length(2), // Footer
            ])
            .split(inner);

        // Title with file info
        let title_text = if self.file_type == PreviewFileType::Pdf {
            format!(
                "{} ({}, page {}/{})",
                self.display_name,
                self.format_size(),
                self.current_page + 1,
                self.total_pages
            )
        } else {
            format!("{} ({})", self.display_name, self.format_size())
        };
        let title = Paragraph::new(popup_title(&title_text, theme)).alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Content area - render image or error (centered)
        let content_area = center_content(chunks[1], 4);
        // Pre-calculate centered area before mutable borrow
        let centered_area = self.calculate_centered_image_area(content_area);

        if let Some(error) = &self.error {
            let error_text = Paragraph::new(Line::from(vec![
                Span::styled("Error: ", Style::default().fg(theme.error_color)),
                Span::raw(error),
            ]))
            .alignment(Alignment::Center);
            f.render_widget(error_text, content_area);
        } else if let Some(ref mut protocol) = self.protocol {
            let image_widget = StatefulImage::default().resize(Resize::Fit(None));
            f.render_stateful_widget(image_widget, centered_area, protocol);
        } else {
            let loading = Paragraph::new("Loading...").alignment(Alignment::Center);
            f.render_widget(loading, content_area);
        }

        // Footer with keybinds - styled keys
        let key_style = Style::default().fg(theme.accent_color);
        let dim_style = Style::default().fg(theme.dim_color);
        let sep = Span::styled("  │  ", dim_style);

        let footer_spans = if self.file_type == PreviewFileType::Pdf {
            vec![
                Span::styled("Esc", key_style),
                Span::styled(" Close", dim_style),
                sep.clone(),
                Span::styled("O", key_style),
                Span::styled(" Open externally", dim_style),
                sep,
                Span::styled("←/→", key_style),
                Span::styled(" Navigate pages", dim_style),
            ]
        } else {
            vec![
                Span::styled("Esc", key_style),
                Span::styled(" Close", dim_style),
                sep,
                Span::styled("O", key_style),
                Span::styled(" Open externally", dim_style),
            ]
        };
        let footer = Paragraph::new(Line::from(footer_spans)).alignment(Alignment::Center);
        f.render_widget(footer, chunks[2]);
    }

    /// Reset popup state
    pub fn reset(&mut self) {
        self.file_path = None;
        self.display_name.clear();
        self.image = None;
        self.protocol = None;
        self.current_page = 0;
        self.total_pages = 1;
        self.file_size = 0;
        self.error = None;
    }
}
