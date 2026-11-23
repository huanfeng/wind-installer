//! 安装目录与数据目录的冲突判定。
//!
//! 纯字符串比较、不碰文件系统，所以跨平台可测。Windows 路径不区分大小写且 `/`、`\` 等价，
//! 逐字比较会把 `D:\App\Data` 与安装包自带的 `D:\App\data` 当成两个目录，
//! 实际是同一个，用户数据与程序文件混在一起，升级时被覆盖。

/// 规整成可比较的形态：反斜杠、小写、去尾部分隔符。
fn norm(p: &str) -> String {
    let mut s = p.trim().replace('/', "\\").to_lowercase();
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    s
}

/// `child` 是否等于 `parent` 或位于其下。
fn is_same_or_inside(child: &str, parent: &str) -> bool {
    child == parent
        || child
            .strip_prefix(parent)
            .is_some_and(|rest| parent.ends_with('\\') || rest.starts_with('\\'))
}

/// 数据目录不可用时返回原因。
///
/// `owned` 是安装包的顶层条目名（清单 `app.owned_entries`）。数据目录放进安装目录是允许的，
/// 只要它不等于安装目录、也不落在包内条目里（卸载会整项删除包内条目，升级也会覆盖其中的文件）。
/// `owned` 为空（旧包没有这份清单）时无从判断，保守地一律不许放进安装目录。
pub fn data_dir_conflict(install_dir: &str, data_dir: &str, owned: &[String]) -> Option<String> {
    let data = norm(data_dir);
    if data.is_empty() {
        return Some("数据目录不能为空".into());
    }
    if !data.contains(':') && !data.starts_with("\\\\") {
        return Some("数据目录必须是绝对路径（如 D:\\WindData）".into());
    }
    let install = norm(install_dir);
    if is_same_or_inside(&data, &install) {
        if data == install {
            return Some("数据目录不能就是安装目录，请换一个位置".into());
        }
        if owned.is_empty() {
            return Some(
                "数据目录不能放在安装目录内（Windows 路径不区分大小写，Data 与 data 是同一个目录），请换一个位置"
                    .into(),
            );
        }
        let rest = data[install.len()..].trim_start_matches('\\');
        let first = rest.split('\\').next().unwrap_or("");
        if let Some(hit) = owned.iter().find(|n| n.to_lowercase() == first) {
            return Some(format!(
                "数据目录不能放在安装包自带的「{hit}」里（Windows 路径不区分大小写，Data 与 data 是同一个目录），请换一个位置"
            ));
        }
    }
    if is_same_or_inside(&install, &data) {
        return Some("安装目录不能位于数据目录内，请换一个位置".into());
    }
    None
}

/// 安装目录必须是本软件独占的：目标已存在、非空、又不像我们装过的目录时，
/// 说明是别人的目录（如 `D:\Anaconda`），卸载会整目录删除，所以改装进其下的 `app_id` 子目录。
///
/// 目录名已是 `app_id`、不存在、为空、或里面有 `markers` 中任一文件（我们自己的旧安装）时原样返回。
pub fn dedicated_install_dir(
    dir: &std::path::Path,
    app_id: &str,
    markers: &[&str],
) -> std::path::PathBuf {
    let is_named = dir
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(app_id));
    if is_named {
        return dir.to_path_buf();
    }
    let Ok(mut entries) = std::fs::read_dir(dir) else {
        return dir.to_path_buf();
    };
    if entries.next().is_none() || markers.iter().any(|m| dir.join(m).exists()) {
        return dir.to_path_buf();
    }
    dir.join(app_id)
}

/// 按当前清单的 `app_id` 与主程序/卸载器作标记，规整安装目录。
#[cfg(windows)]
pub fn dedicated_for_app(dir: &std::path::Path) -> std::path::PathBuf {
    dedicated_install_dir(
        dir,
        crate::meta::app_id(),
        &[crate::installer::UNINSTALLER_NAME, crate::meta::main_exe()],
    )
}

#[cfg(test)]
mod dedicated_tests {
    use super::dedicated_install_dir as d;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("wi_ded_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn foreign_non_empty_dir_gets_subdir() {
        let p = tmp("foreign");
        std::fs::write(p.join("python.exe"), "x").unwrap();
        assert_eq!(d(&p, "WindInput", &["wind_input.exe"]), p.join("WindInput"));
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn own_previous_install_is_kept() {
        let p = tmp("own");
        std::fs::write(p.join("wind_input.exe"), "x").unwrap();
        assert_eq!(d(&p, "WindInput", &["wind_input.exe"]), p);
        let _ = std::fs::remove_dir_all(&p);
    }

    #[test]
    fn empty_missing_or_already_named_dir_is_kept() {
        let p = tmp("empty");
        assert_eq!(d(&p, "WindInput", &[]), p);
        let missing = p.join("nope");
        assert_eq!(d(&missing, "WindInput", &[]), missing);
        let named = p.join("windinput");
        std::fs::create_dir_all(&named).unwrap();
        std::fs::write(named.join("a"), "x").unwrap();
        assert_eq!(d(&named, "WindInput", &[]), named);
        let _ = std::fs::remove_dir_all(&p);
    }
}

#[cfg(test)]
mod tests {
    use super::data_dir_conflict;

    /// 旧包：没有顶层条目清单
    fn c(install: &str, data: &str) -> Option<String> {
        data_dir_conflict(install, data, &[])
    }

    fn owned() -> Vec<String> {
        vec!["data".into(), "wind_input.exe".into()]
    }

    #[test]
    fn case_variant_of_bundled_data_dir_is_rejected() {
        assert!(c(r"D:\WindInput", r"D:\WindInput\Data").is_some());
        assert!(c(r"D:\WindInput", r"d:\windinput\data").is_some());
    }

    #[test]
    fn slash_and_trailing_separator_are_equivalent() {
        assert!(c(r"D:\WindInput", "D:/WindInput/data/").is_some());
        assert!(c(r"D:\WindInput\", r"D:\WindInput").is_some());
    }

    #[test]
    fn anything_inside_install_dir_is_rejected() {
        assert!(c(r"D:\WindInput", r"D:\WindInput\user\cfg").is_some());
    }

    #[test]
    fn install_dir_inside_data_dir_is_rejected() {
        assert!(c(r"D:\Data\App", r"D:\Data").is_some());
    }

    #[test]
    fn sibling_with_same_prefix_is_fine() {
        assert!(c(r"D:\WindInput", r"D:\WindInputData").is_none());
        assert!(c(
            r"C:\Program Files\WindInput",
            r"C:\Users\a\AppData\Roaming\WindInput"
        )
        .is_none());
    }

    #[test]
    fn empty_and_relative_are_rejected() {
        assert!(c(r"D:\App", "").is_some());
        assert!(c(r"D:\App", "data").is_some());
    }

    #[test]
    fn with_owned_list_sibling_dir_inside_install_is_allowed() {
        let o = owned();
        assert!(data_dir_conflict(r"D:\WindInput", r"D:\WindInput\UserData", &o).is_none());
        assert!(data_dir_conflict(r"D:\WindInput", r"D:\WindInput\UserData\cfg", &o).is_none());
    }

    #[test]
    fn with_owned_list_package_dirs_are_still_rejected_case_insensitively() {
        let o = owned();
        assert!(data_dir_conflict(r"D:\WindInput", r"D:\WindInput\Data", &o).is_some());
        assert!(data_dir_conflict(r"D:\WindInput", "d:/windinput/DATA/user", &o).is_some());
    }

    #[test]
    fn data_dir_equal_to_install_dir_is_rejected_either_way() {
        assert!(data_dir_conflict(r"D:\WindInput", r"D:\WindInput\", &owned()).is_some());
    }

    #[test]
    fn install_dir_inside_data_dir_is_rejected_with_owned_list() {
        assert!(data_dir_conflict(r"D:\Data\App", r"D:\Data", &owned()).is_some());
    }
}
