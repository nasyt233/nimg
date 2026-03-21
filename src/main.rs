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
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::{Frame, Terminal};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

// ==================== 图片查看器核心 ====================
struct Viewer {
    original_image: ImageBuffer<Rgba<u8>, Vec<u8>>,
    scale: f64,
    scroll_x: u32,
    scroll_y: u32,
    display_size: (u16, u16),
    scaled_image: Option<ImageBuffer<Rgba<u8>, Vec<u8>>>,
    scaled_size: (u32, u32),
    format: Option<ImageFormat>,
}

impl Viewer {
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
        Some(f) => format!("{:?}", f),
        None => "Unknown".to_string(),
    }
}

fn run_viewer(mut terminal: Terminal<CrosstermBackend<io::Stdout>>, path: PathBuf) -> Result<()> {
    let mut app = Viewer::new(path)?;

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
            let help_area = Rect::new(1, size.height.saturating_sub(1), size.width.saturating_sub(2), 1);
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
    Ok(())
}

// ==================== 图片选择器 ====================
struct FileSelector {
    current_dir: PathBuf,
    entries: Vec<PathBuf>,
    list_state: ListState,
}

impl FileSelector {
    fn new(start_dir: PathBuf) -> Result<Self> {
        let mut selector = Self {
            current_dir: start_dir,
            entries: Vec::new(),
            list_state: ListState::default(),
        };
        selector.refresh_entries()?;
        selector.list_state.select(Some(0));
        Ok(selector)
    }

    fn refresh_entries(&mut self) -> Result<()> {
        let mut entries = Vec::new();
        let read_dir = fs::read_dir(&self.current_dir)?;
        for entry in read_dir {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                entries.push(path);
            } else if is_image_file(&path) {
                entries.push(path);
            }
        }
        // 排序：目录在前，文件在后，各自按名称排序
        entries.sort_by(|a, b| {
            let a_is_dir = a.is_dir();
            let b_is_dir = b.is_dir();
            if a_is_dir && !b_is_dir {
                std::cmp::Ordering::Less
            } else if !a_is_dir && b_is_dir {
                std::cmp::Ordering::Greater
            } else {
                a.file_name().cmp(&b.file_name())
            }
        });
        // 添加 ".." 条目用于返回上级目录
        if self.current_dir.parent().is_some() {
            entries.insert(0, self.current_dir.join(".."));
        }
        self.entries = entries;
        // 调整选中项
        if let Some(selected) = self.list_state.selected() {
            if selected >= self.entries.len() {
                self.list_state.select(Some(self.entries.len().saturating_sub(1)));
            }
        } else if !self.entries.is_empty() {
            self.list_state.select(Some(0));
        }
        Ok(())
    }

    fn enter_current(&mut self) -> Option<PathBuf> {
        let idx = self.list_state.selected()?;
        let path = self.entries.get(idx)?;
        if path.is_dir() {
            // 进入目录
            self.current_dir = path.clone();
            if let Err(e) = self.refresh_entries() {
                eprintln!("无法进入目录: {}", e);
                // 尝试回退到原目录
                if let Some(parent) = self.current_dir.parent() {
                    self.current_dir = parent.to_path_buf();
                    let _ = self.refresh_entries();
                }
            }
            self.list_state.select(Some(0));
            None
        } else if is_image_file(path) {
            Some(path.clone())
        } else {
            None
        }
    }

    fn go_parent(&mut self) {
        if let Some(parent) = self.current_dir.parent() {
            self.current_dir = parent.to_path_buf();
            if let Err(e) = self.refresh_entries() {
                eprintln!("无法返回上级目录: {}", e);
            }
            self.list_state.select(Some(0));
        }
    }
}

fn is_image_file(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
        let ext = ext.to_string_lossy().to_lowercase();
        matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "tif" | "ico" | "avif" | "qoi")
    } else {
        false
    }
}

fn draw_file_selector(frame: &mut Frame<'_>, selector: &mut FileSelector) {
    let size = frame.size();
    // 标题
    let title = format!("选择图片 - 当前目录: {}", selector.current_dir.display());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title);
    frame.render_widget(block, size);

    // 列表区域
    let list_area = Rect::new(1, 1, size.width.saturating_sub(2), size.height.saturating_sub(3));
    let items: Vec<ListItem> = selector.entries.iter().map(|path| {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let prefix = if path.is_dir() { "📁 " } else { "🖼️ " };
        let display = format!("{}{}", prefix, name);
        ListItem::new(Line::from(Span::styled(display, Style::default())))
    }).collect();
    let list = List::new(items)
        .block(Block::default())
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, list_area, &mut selector.list_state);

    // 帮助信息
    let help_text = "↑↓ 移动 | Enter 选择/进入目录 | Backspace 返回上级 | Esc/q 退出";
    let help_area = Rect::new(1, size.height.saturating_sub(1), size.width.saturating_sub(2), 1);
    let help_para = Paragraph::new(help_text);
    frame.render_widget(help_para, help_area);
}

fn select_image() -> Option<PathBuf> {
    let mut terminal = match setup_terminal() {
        Ok(t) => t,
        Err(_) => return None,
    };
    let mut selector = match FileSelector::new(PathBuf::from(".")) {
        Ok(s) => s,
        Err(_) => return None,
    };

    loop {
        if let Err(_) = terminal.draw(|frame| draw_file_selector(frame, &mut selector)) {
            return None;
        }

        if let Ok(true) = event::poll(std::time::Duration::from_millis(100)) {
            match event::read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    match key.code {
                        KeyCode::Up => {
                            let i = selector.list_state.selected().unwrap_or(0);
                            let new = if i == 0 { selector.entries.len().saturating_sub(1) } else { i - 1 };
                            selector.list_state.select(Some(new));
                        }
                        KeyCode::Down => {
                            let i = selector.list_state.selected().unwrap_or(0);
                            let new = (i + 1) % selector.entries.len();
                            selector.list_state.select(Some(new));
                        }
                        KeyCode::Enter => {
                            if let Some(path) = selector.enter_current() {
                                let _ = restore_terminal(terminal);
                                return Some(path);
                            }
                        }
                        KeyCode::Backspace => {
                            selector.go_parent();
                        }
                        KeyCode::Esc | KeyCode::Char('q') => {
                            let _ = restore_terminal(terminal);
                            return None;
                        }
                        _ => {}
                    }
                }
                Ok(Event::Resize(_, _)) => {
                    // 终端大小改变，下次重绘会适配
                }
                _ => {}
            }
        }
    }
}

// ==================== 终端初始化/恢复 ====================
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

// ==================== 主函数 ====================
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = if args.len() == 2 {
        Some(PathBuf::from(&args[1]))
    } else if args.len() == 1 {
        select_image()
    } else {
        eprintln!("用法: {} [图片文件]", args[0]);
        std::process::exit(1);
    };

    let path = match path {
        Some(p) => p,
        None => return Ok(()),
    };

    if !path.exists() {
        eprintln!("文件不存在: {}", path.display());
        std::process::exit(1);
    }

    let terminal = setup_terminal()?;
    run_viewer(terminal, path)?;
    Ok(())
}