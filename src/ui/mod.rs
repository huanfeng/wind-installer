pub mod install_wizard;
pub mod theme;
pub mod uninstall_wizard;

use windui::geometry::Color;
use windui::platform::Renderer;
use windui::theme::Theme;

/// 无边框模式自绘标题栏的高度。
pub const TITLE_BAR_H: i32 = 36;

/// 清单里的窗口尺寸 → 向导窗口客户区尺寸。清单尺寸是**内容区**（标题栏之下）的大小。
///
/// windui 0.21 起无边框窗口的外框即客户区；此前客户区比请求的大约 +16 × +39（建窗按带框窗口
/// 反算外框、随后非客户区被消掉），向导的排版是按那份多出来的空间调出来的。现在客户区恰等于
/// 请求尺寸，标题栏占掉其中 `TITLE_BAR_H`，故无边框时高度要把标题栏加上，否则内容区只剩
/// `h - TITLE_BAR_H`，页面里靠下的按钮被挤到窗口外。
pub fn window_size(w: i32, h: i32) -> (i32, i32) {
    if cfg!(feature = "frameless") {
        (w, h + TITLE_BAR_H)
    } else {
        (w, h)
    }
}

/// 向导窗口使用的渲染后端。
/// 默认 [`Renderer::Auto`]（D2D 硬件加速优先，设备建不起来自动回退软光栅）；
/// 以下任一条件强制软光栅：
///   - 环境变量 `WIND_SOFT_RENDER` 非空且不为 `"0"`
///   - 命令行包含 `--soft-render`
///
/// **不用 [`Renderer::Gpu`]**：那一档在拿不到 GPU 时报错终止，而安装器往往是用户在
/// 这台机器上运行的第一个程序——显卡驱动没装好就起不来，用户无从自救。这里要的
/// 恰恰是静默回退。
pub fn renderer() -> Renderer {
    let env_soft = std::env::var("WIND_SOFT_RENDER")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);
    let arg_soft = std::env::args().any(|a| a == "--soft-render");
    if env_soft || arg_soft {
        Renderer::Software
    } else {
        Renderer::Auto
    }
}

/// 向导主题：把清单（或内置默认）的主色铺到 windui 的强调色槽上，
/// 单选、复选、进度条、主按钮这些吃主题强调色的控件才与手写 `.bg(accent)` 的按钮同色。
/// 只换强调色三态，其余沿用 windui 默认主题。
pub fn app_theme() -> Theme {
    let mut t = Theme::default();
    t.palette.accent = Color::hex(theme::accent());
    t.palette.accent_hover = Color::hex(theme::accent_hover());
    t.palette.accent_active = Color::hex(theme::accent_pressed());
    t.palette.on_accent = Color::WHITE;
    t
}
