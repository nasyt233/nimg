// Cargo.toml 依赖：
// [dependencies]
// ratatui = "0.26"
// crossterm = "0.27"
// image = "0.24"
// anyhow = "1.0"

use anyhow::{anyhow, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use image::{ImageBuffer, ImageFormat, Rgba};
use image::io::Reader as ImageReader;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};
use std::io;
use std::path::PathBuf;

struct App {
    original_image: ImageBuffer<Rgba<u8>, Vec<u8>>,
    scale: f64,
    scroll_x: u32,
    scroll_y: u32,
    display_size: (u16, u16),
    scaled_image: Option<ImageBuffer<Rgba<u8>, Vec<u8>>>,
    scaled_size: (u32, u32),
    format: Option<ImageFormat>,
}

impl App {
    fn new(path: PathBuf) -> Result<Self> {
        let reader = ImageReader::open(&path)
            .map_err(|e| anyhow!("无法打开图片 {}: {}", path.display(), e))?;
        let reader = reader.with_guessed_format()
            .map_err(|e| anyhow!("无法识别图片格式: {}", e))?;
        let format = reader.format();
        let img = reader.decode()
            .map_err(|e| anyhow!("解码图片失败: {}", e))?;
        let img = img.to_rgba8();
        let (w, h) = img.dimensions();
        Ok(Self {
            original_image: img,
            scale: 1.0,
            scroll_x: 0,
            scroll_y: 0,
            display_size: (0, 0),
            scaled_image: None,
            scaled_size: (w, h),
            format,
        })
    }

    fn update_scaled_image(&mut self) {
        if self.scale <= 0.0 {
            return;
        }
        let (orig_w, orig_h) = self.original_image.dimensions();
        let new_w = (orig_w as f64 * self.scale) as u32;
        let new_h = (orig_h as f64 * self.scale) as u32;
        if new_w == 0 || new_h == 0 {
            return;
        }
        let resized = image::imageops::resize(
            &self.original_image,
            new_w,
            new_h,
            image::imageops::FilterType::Triangle,
        );
        self.scaled_image = Some(resized);
        self.scaled_size = (new_w, new_h);
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        let (disp_w_chars, disp_h_chars) = self.display_size;
        let disp_w_px = disp_w_chars as u32;
        let disp_h_px = (disp_h_chars as u32) * 2;
        let max_scroll_x = self.scaled_size.0.saturating_sub(disp_w_px);
        let max_scroll_y = self.scaled_size.1.saturating_sub(disp_h_px);
        self.scroll_x = self.scroll_x.min(max_scroll_x);
        self.scroll_y = self.scroll_y.min(max_scroll_y);
    }

    fn set_display_size(&mut self, w: u16, h: u16) {
        self.display_size = (w, h);
        let (orig_w, orig_h) = self.original_image.dimensions();
        let disp_w_px = w as u32;
        let disp_h_px = (h as u32) * 2;
        let scale_w = disp_w_px as f64 / orig_w as f64;
        let scale_h = disp_h_px as f64 / orig_h as f64;
        self.scale = scale_w.min(scale_h);
        self.update_scaled_image();
    }

    fn handle_key(&mut self, key: KeyCode) -> bool {
        match key {
            KeyCode::Char('q') => return true,
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.scale *= 1.1;
                self.update_scaled_image();
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                self.scale /= 1.1;
                self.update_scaled_image();
            }
            KeyCode::Left => {
                let step = (self.display_size.0 as u32).max(1);
                self.scroll_x = self.scroll_x.saturating_sub(step);
                self.clamp_scroll();
            }
            KeyCode::Right => {
                let step = (self.display_size.0 as u32).max(1);
                self.scroll_x = self.scroll_x.saturating_add(step);
                self.clamp_scroll();
            }
            KeyCode::Up => {
                let step = (self.display_size.1 as u32).max(1) * 2;
                self.scroll_y = self.scroll_y.saturating_sub(step);
                self.clamp_scroll();
            }
            KeyCode::Down => {
                let step = (self.display_size.1 as u32).max(1) * 2;
                self.scroll_y = self.scroll_y.saturating_add(step);
                self.clamp_scroll();
            }
            _ => {}
        }
        false
    }

    fn draw_image(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let Some(img) = &self.scaled_image else { return };
        let (img_w, img_h) = img.dimensions();
        let disp_w = area.width as usize;
        let disp_h = area.height as usize;

        let start_x = self.scroll_x as usize;
        let start_y = self.scroll_y as usize;

        if start_x >= img_w as usize || start_y >= img_h as usize {
            return;
        }

        let buffer = frame.buffer_mut();
        for cell_y in 0..disp_h {
            let y_top = start_y + cell_y * 2;
            if y_top >= img_h as usize {
                break;
            }
            let y_bottom = y_top + 1;
            let row_base = (area.y + cell_y as u16) as usize;
            let col_base = area.x as usize;

            let max_cell_x = disp_w.min(img_w as usize - start_x);
            for cell_x in 0..max_cell_x {
                let x = start_x + cell_x;
                let top = img.get_pixel(x as u32, y_top as u32);
                let top_color = Color::Rgb(top[0], top[1], top[2]);
                let bottom_color = if y_bottom < img_h as usize {
                    let bottom = img.get_pixel(x as u32, y_bottom as u32);
                    Color::Rgb(bottom[0], bottom[1], bottom[2])
                } else {
                    Color::Black
                };
                let cell = buffer.get_mut((col_base + cell_x) as u16, row_base as u16);
                cell.set_char('▀')
                    .set_fg(top_color)
                    .set_bg(bottom_color);
            }
        }
    }
}

/// 将 ImageFormat 转换为可读字符串
fn format_name(format: Option<ImageFormat>) -> String {
    match format {
        Some(ImageFormat::Png) => "PNG".to_string(),
        Some(ImageFormat::Jpeg) => "JPEG".to_string(),
        Some(ImageFormat::WebP) => "WebP".to_string(),
        Some(ImageFormat::Gif) => "GIF".to_string(),
        Some(ImageFormat::Bmp) => "BMP".to_string(),
        Some(ImageFormat::Tiff) => "TIFF".to_string(),
        Some(ImageFormat::Ico) => "ICO".to_string(),
        Some(ImageFormat::Avif) => "AVIF".to_string(),
        Some(ImageFormat::Qoi) => "QOI".to_string(),
        Some(f) => format!("{:?}", f), // 其他格式使用调试输出
        None => "Unknown".to_string(),
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(mut terminal: Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("用法: {} <图片文件>", args[0]);
        std::process::exit(1);
    }
    let path = PathBuf::from(&args[1]);
    if !path.exists() {
        eprintln!("文件不存在: {}", path.display());
        std::process::exit(1);
    }

    let mut terminal = setup_terminal()?;
    let mut app = App::new(path)?;

    let size = terminal.size()?;
    let area = Rect::new(1, 1, size.width.saturating_sub(2), size.height.saturating_sub(2));
    app.set_display_size(area.width, area.height);

    loop {
        terminal.draw(|frame| {
            let size = frame.size();
            let area = Rect::new(1, 1, size.width.saturating_sub(2), size.height.saturating_sub(2));
            if area.width != app.display_size.0 || area.height != app.display_size.1 {
                app.set_display_size(area.width, area.height);
            }

            let format_str = format_name(app.format);
            let title = format!(
                "{} - 缩放: {:.1}% | 尺寸: {}x{} | 滚动: ({},{})",
                format_str,
                app.scale * 100.0,
                app.scaled_size.0,
                app.scaled_size.1,
                app.scroll_x,
                app.scroll_y
            );
            let block = Block::default().borders(Borders::ALL).title(title);
            frame.render_widget(block, size);

            let inner_area = Rect::new(1, 1, size.width.saturating_sub(2), size.height.saturating_sub(2));
            app.draw_image(frame, inner_area);

            let help_text = "←↑↓→ 滚动 | +/- 缩放 | q 退出";
            let help_area = Rect::new(1, size.height - 1, help_text.len() as u16, 1);
            let help_para = Paragraph::new(help_text);
            frame.render_widget(help_para, help_area);
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if app.handle_key(key.code) {
                        break;
                    }
                }
                Event::Resize(new_w, new_h) => {
                    terminal.resize(Rect::new(0, 0, new_w, new_h))?;
                }
                _ => {}
            }
        }
    }

    restore_terminal(terminal)?;
    Ok(())
}