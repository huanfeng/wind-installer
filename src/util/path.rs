#![allow(dead_code)]

use std::path::PathBuf;

/// 获取 Program Files 路径
pub fn get_program_files() -> PathBuf {
    std::env::var("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Program Files"))
}

/// 获取 APPDATA 路径
pub fn get_app_data() -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let user_profile = std::env::var("USERPROFILE").unwrap_or_default();
            PathBuf::from(user_profile).join("AppData\\Roaming")
        })
}

/// 获取 LOCALAPPDATA 路径
pub fn get_local_app_data() -> PathBuf {
    std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let user_profile = std::env::var("USERPROFILE").unwrap_or_default();
            PathBuf::from(user_profile).join("AppData\\Local")
        })
}

/// 获取临时目录路径
pub fn get_temp_dir() -> PathBuf {
    std::env::temp_dir()
}

/// 获取桌面路径
pub fn get_desktop() -> PathBuf {
    let user_profile = std::env::var("USERPROFILE").unwrap_or_default();
    PathBuf::from(user_profile).join("Desktop")
}

/// 获取开始菜单程序路径
pub fn get_start_menu_programs() -> PathBuf {
    let program_data =
        std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".to_string());
    PathBuf::from(program_data)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
}

/// 获取 Windows 字体目录
pub fn get_fonts_dir() -> PathBuf {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_string());
    PathBuf::from(windir).join("Fonts")
}

/// 检查是否为 64 位系统
pub fn is_64bit_system() -> bool {
    std::env::var("PROCESSOR_ARCHITECTURE")
        .map(|arch| arch == "AMD64" || arch == "IA64")
        .unwrap_or(false)
}

/// 把路径里的 `%VAR%` 换成本进程环境里的实际值，供界面显示与安装共用。
///
/// 界面上必须显示展开后的路径：普通用户看不懂 `%APPDATA%`，而安装时写入的正是这里展开出的
/// 位置——两处用同一个函数，显示的就是实际落盘的地方。取不到值的变量原样保留。
pub fn expand_env_vars(s: &str) -> String {
    expand_env_vars_with(s, env_value)
}

/// 环境变量取值；常用目录变量取不到时按 [`get_app_data`] 等同样的口径兜底（与卸载清理一致）。
fn env_value(name: &str) -> Option<String> {
    if let Ok(v) = std::env::var(name) {
        if !v.is_empty() {
            return Some(v);
        }
    }
    let fallback = match name.to_ascii_uppercase().as_str() {
        "PROGRAMFILES" => get_program_files(),
        "APPDATA" => get_app_data(),
        "LOCALAPPDATA" => get_local_app_data(),
        "USERPROFILE" => PathBuf::from(r"C:\Users\Default"),
        _ => return None,
    };
    Some(fallback.to_string_lossy().into_owned())
}

/// [`expand_env_vars`] 的本体，取值函数由参数给（测试不必改进程环境）。
fn expand_env_vars_with(s: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => match lookup(&after[..end]) {
                Some(val) => {
                    out.push_str(&val);
                    rest = &after[end + 1..];
                }
                None => {
                    // 不认识的变量连同两个 % 原样留下（与 ExpandEnvironmentStrings 一致）
                    out.push('%');
                    out.push_str(&after[..=end]);
                    rest = &after[end + 1..];
                }
            },
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod expand_tests {
    use super::expand_env_vars_with;

    fn lookup(name: &str) -> Option<String> {
        match name {
            "APPDATA" => Some(r"C:\Users\li\AppData\Roaming".into()),
            "ProgramFiles" => Some(r"C:\Program Files".into()),
            _ => None,
        }
    }

    #[test]
    fn known_variables_become_real_paths() {
        assert_eq!(
            expand_env_vars_with(r"%APPDATA%\Demo", lookup),
            r"C:\Users\li\AppData\Roaming\Demo"
        );
        assert_eq!(
            expand_env_vars_with(r"%ProgramFiles%\Demo", lookup),
            r"C:\Program Files\Demo"
        );
    }

    #[test]
    fn plain_paths_are_unchanged() {
        assert_eq!(expand_env_vars_with(r"D:\MyData\Demo", lookup), r"D:\MyData\Demo");
        // 已展开的路径再展开一次不变：界面值在安装时还会过一遍
        let once = expand_env_vars_with(r"%APPDATA%\Demo", lookup);
        assert_eq!(expand_env_vars_with(&once, lookup), once);
    }

    #[test]
    fn unknown_variables_and_stray_percent_stay_literal() {
        assert_eq!(expand_env_vars_with(r"%NOPE%\Demo", lookup), r"%NOPE%\Demo");
        assert_eq!(expand_env_vars_with(r"D:\100%\Demo", lookup), r"D:\100%\Demo");
        assert_eq!(expand_env_vars_with(r"%%\Demo", lookup), r"%%\Demo");
        assert_eq!(
            expand_env_vars_with(r"%NOPE%APPDATA%\x", lookup),
            r"%NOPE%APPDATA%\x"
        );
    }
}
