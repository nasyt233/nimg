// Cargo.toml 依赖：
// [dependencies]
// ratatui = "0.26"
// crossterm = "0.27"
// image = "0.24"
// anyhow = "1.0"

use anyhow::{anyhow, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use image::{DynamicImage, GenericImageView};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};
use std::io;
use std::path::PathBuf;

/// 应用程序状态
struct App {
    /// 原始图片
    original_image: DynamicImage,
    /// 当前缩放因子（缩放后宽度 / 原始宽度）
    scale: f64,
    /// 滚动偏移量（像素，在缩放后图片中）
    scroll_x: u32,
    scroll_y: u32,
    /// 显示区域尺寸（字符宽，字符高）
    display_size: (u16, u16),
    /// 缓存缩放后的图片（避免每帧重新缩放）
    scaled_image: Option<DynamicImage>,
    /// 缩放后的图片尺寸（宽，高）
    scaled_size: (u32, u32),
}

impl App {
    /// 创建新 App 并加载图片
    fn new(path: PathBuf) -> Result<Self> {
        let img = image::open(&path)
            .map_err(|e| anyhow!("无法打开图片 {}: {}", path.display(), e))?;
        // 转换为 RGBA8 以便统一处理
        let img = img.to_rgba8();

        // 初始缩放因子：使图片适应显示区域（稍后会在第一次渲染时根据终端大小调整）
        let scale = 1.0;
        let scaled_size = (img.width(), img.height());
        Ok(Self {
            original_image: DynamicImage::ImageRgba8(img),
            scale,
            scroll_x: 0,
            scroll_y: 0,
            display_size: (0, 0),
            scaled_image: None,
            scaled_size,
        })
    }

    /// 根据当前缩放因子更新缩放后的图片
    fn update_scaled_image(&mut self) {
        if self.scale <= 0.0 {
            return;
        }
        let (orig_w, orig_h) = (self.original_image.width(), self.original_image.height());
        let new_w = (orig_w as f64 * self.scale) as u32;
        let new_h = (orig_h as f64 * self.scale) as u32;
        if new_w == 0 || new_h == 0 {
            return;
        }
        let resized = image::imageops::resize(
            &self.original_image,
            new_w,
            new_h,
            image::imageops::FilterType::Lanczos3,
        );
        self.scaled_image = Some(DynamicImage::ImageRgba8(resized));
        self.scaled_size = (new_w, new_h);
        // 确保滚动偏移不越界
        self.clamp_scroll();
    }

    /// 确保滚动偏移在有效范围内
    fn clamp_scroll(&mut self) {
        let (disp_w_chars, disp_h_chars) = self.display_size;
        let disp_w_px = disp_w_chars as u32;          // 显示区域宽度（像素），每个字符对应一个像素宽
        let disp_h_px = (disp_h_chars as u32) * 2;    // 显示区域高度（像素），每个字符对应两个像素高

        let max_scroll_x = self.scaled_size.0.saturating_sub(disp_w_px);
        let max_scroll_y = self.scaled_size.1.saturating_sub(disp_h_px);
        self.scroll_x = self.scroll_x.min(max_scroll_x);
        self.scroll_y = self.scroll_y.min(max_scroll_y);
    }

    /// 设置显示区域尺寸（字符宽，字符高），并重新计算缩放因子以适应屏幕
    fn set_display_size(&mut self, w: u16, h: u16) {
        self.display_size = (w, h);
        // 根据显示区域计算初始缩放因子，使图像完整显示（保持宽高比）
        let (orig_w, orig_h) = (self.original_image.width(), self.original_image.height());
        let disp_w_px = w as u32;          // 显示像素宽
        let disp_h_px = (h as u32) * 2;    // 显示像素高
        let scale_w = disp_w_px as f64 / orig_w as f64;
        let scale_h = disp_h_px as f64 / orig_h as f64;
        self.scale = scale_w.min(scale_h);
        // 重新生成缩放图片
        self.update_scaled_image();
    }

    /// 处理键盘输入
    fn handle_key(&mut self, key: KeyCode) -> bool {
        match key {
            KeyCode::Char('q') => return true, // 退出
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.scale *= 1.2;
                self.update_scaled_image();
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                self.scale /= 1.2;
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
                let step = (self.display_size.1 as u32).max(1) * 2; // 每次滚动半个屏幕高度（像素）
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

    /// 绘制图像区域
    fn draw_image(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let Some(img) = &self.scaled_image else { return };
        let (img_w, img_h) = (img.width(), img.height());
        let (disp_w, disp_h) = (area.width as usize, area.height as usize);

        // 计算显示区域对应的缩放后图片的起始坐标（像素）
        let start_x = self.scroll_x as usize;
        let start_y = self.scroll_y as usize;

        // 使用 ratatui 的 Buffer 直接绘图
        let buffer = frame.buffer_mut();
        for cell_y in 0..disp_h {
            // 每个单元格对应两个像素行：上像素 y = start_y + cell_y*2，下像素 y = start_y + cell_y*2 + 1
            let y_top = start_y + cell_y * 2;
            let y_bottom = y_top + 1;
            for cell_x in 0..disp_w {
                let x = start_x + cell_x;
                if x >= img_w as usize || y_top >= img_h as usize {
                    // 超出图片范围，留空
                    continue;
                }
                // 获取上像素颜色
                let top_pixel = img.get_pixel(x as u32, y_top as u32);
                let top_color = Color::Rgb(top_pixel[0], top_pixel[1], top_pixel[2]);
                // 获取下像素颜色（如果存在）
                let bottom_color = if y_bottom < img_h as usize {
                    let bottom_pixel = img.get_pixel(x as u32, y_bottom as u32);
                    Color::Rgb(bottom_pixel[0], bottom_pixel[1], bottom_pixel[2])
                } else {
                    // 奇数高度，下部分用背景色填充（默认黑色）
                    Color::Black
                };
                // 计算缓冲区中的位置
                let buf_x = area.x + cell_x as u16;
                let buf_y = area.y + cell_y as u16;
                // 直接获取 Cell 并设置（假设坐标有效）
                let cell = buffer.get_mut(buf_x, buf_y);
                cell.set_char('▀')
                    .set_fg(top_color)
                    .set_bg(bottom_color);
            }
        }
    }
}

/// 初始化终端
fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// 恢复终端
fn restore_terminal(mut terminal: Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn main() -> Result<()> {
    // 解析命令行参数
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

    // 初始获取终端尺寸并设置显示区域
    let size = terminal.size()?;
    // 留出边框空间（上下各一行，左右各一列）
    let area = Rect::new(1, 1, size.width.saturating_sub(2), size.height.saturating_sub(2));
    app.set_display_size(area.width, area.height);

    // 主循环
    loop {
        terminal.draw(|frame| {
            // 获取当前终端大小（可能已改变）
            let size = frame.size();
            let area = Rect::new(1, 1, size.width.saturating_sub(2), size.height.saturating_sub(2));
            // 如果显示区域改变，更新 App
            if area.width != app.display_size.0 || area.height != app.display_size.1 {
                app.set_display_size(area.width, area.height);
            }

            // 绘制标题栏
            let title = format!(
                "图片查看器 - 缩放: {:.1}% | 尺寸: {}x{} | 滚动: ({},{})",
                app.scale * 100.0,
                app.scaled_size.0,
                app.scaled_size.1,
                app.scroll_x,
                app.scroll_y
            );
            let block = Block::default()
                .borders(Borders::ALL)
                .title(title);
            frame.render_widget(block, size);

            // 绘制图像区域（在边框内部）
            let inner_area = Rect::new(1, 1, size.width.saturating_sub(2), size.height.saturating_sub(2));
            app.draw_image(frame, inner_area);

            // 显示帮助信息
            let help_text = "←↑↓→ 滚动 | +/- 缩放 | q 退出";
            let help_area = Rect::new(1, size.height - 1, help_text.len() as u16, 1);
            let help_para = Paragraph::new(help_text);
            frame.render_widget(help_para, help_area);
        })?;

        // 处理事件
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