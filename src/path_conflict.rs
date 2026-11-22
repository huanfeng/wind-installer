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
pub fn data_dir_conflict(install_dir: &str, data_dir: &str) -> Option<String> {
    let data = norm(data_dir);
    if data.is_empty() {
        return Some("数据目录不能为空".into());
    }
    if !data.contains(':') && !data.starts_with("\\\\") {
        return Some("数据目录必须是绝对路径（如 D:\\WindData）".into());
    }
    let install = norm(install_dir);
    if is_same_or_inside(&data, &install) {
        return Some(
            "数据目录不能放在安装目录内（Windows 路径不区分大小写，Data 与 data 是同一个目录），请换一个位置"
                .into(),
        );
    }
    if is_same_or_inside(&install, &data) {
        return Some("安装目录不能位于数据目录内，请换一个位置".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::data_dir_conflict as c;

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
}
