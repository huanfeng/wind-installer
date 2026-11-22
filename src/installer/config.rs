use std::path::PathBuf;

use crate::meta;

/// 安装配置
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct InstallConfig {
    /// 应用名称
    pub app_name: String,
    /// 应用版本
    pub app_version: String,
    /// 安装目录
    pub install_dir: PathBuf,
    /// 数据目录（用户配置等应用自有数据）
    pub data_dir: PathBuf,
    /// 是否使用自定义数据目录
    pub use_custom_data_dir: bool,
    /// 自定义数据目录路径
    pub custom_data_dir: Option<PathBuf>,
    /// 开始菜单文件夹名
    pub start_menu_folder: String,
    /// 发布者
    pub publisher: String,
}

impl Default for InstallConfig {
    fn default() -> Self {
        let program_files =
            std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
        let app_data = std::env::var("APPDATA").unwrap_or_else(|_| {
            let mut p = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
            p.push("AppData\\Roaming");
            p.to_string_lossy().to_string()
        });
        let _local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| {
            let mut p = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
            p.push("AppData\\Local");
            p.to_string_lossy().to_string()
        });

        Self {
            app_name: meta::app_display_name().to_string(),
            app_version: meta::app_version().to_string(),
            install_dir: PathBuf::from(program_files).join(meta::app_id()),
            data_dir: PathBuf::from(app_data).join(meta::app_id()),
            use_custom_data_dir: false,
            custom_data_dir: None,
            start_menu_folder: meta::start_menu_folder().to_string(),
            publisher: meta::app_publisher().to_string(),
        }
    }
}

#[allow(dead_code)]
impl InstallConfig {
    /// 获取实际数据目录
    pub fn effective_data_dir(&self) -> &PathBuf {
        if self.use_custom_data_dir {
            if let Some(ref custom) = self.custom_data_dir {
                return custom;
            }
        }
        &self.data_dir
    }
}
