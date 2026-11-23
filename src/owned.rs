//! 「安装包自己的顶层条目」：卸载只删这些，其余一律保留。
//!
//! 清单由打包器在打包时从源目录的**第一层**枚举写入（不遍历子目录），所以新增目录
//! 无需手工登记；卸载器按名字整项删除（目录递归），不认识的文件和目录（用户自建的
//! 数据目录、第三方定制层等）原样留下，删完后目录不空就不删安装目录本身。
//!
//! 纯文件系统操作、不依赖清单，跨平台可测。

use std::path::{Path, PathBuf};

#[allow(dead_code)] // 仅打包器（经 lib）使用，两个 bin 里各自声明模块时不会引用
/// 枚举目录的第一层条目名（已排序，保证同一源目录两次打包的清单字节一致）。
pub fn top_level_names(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    Ok(names)
}

/// 条目名必须是单层相对名：空、含分隔符、`.`/`..`、带盘符的一律丢弃，
/// 防止一份坏清单把删除目标指到安装目录之外。
fn is_safe_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
}

/// 清单里的条目名 → 安装目录下的绝对路径（丢弃不安全的名字）。
pub fn owned_paths(install_dir: &Path, names: &[String]) -> Vec<PathBuf> {
    names
        .iter()
        .filter(|n| is_safe_name(n))
        .map(|n| install_dir.join(n))
        .collect()
}

/// 删除一项：目录递归，其余按文件。符号链接/junction 按「文件」处理，只摘链接本身。
fn remove_entry(path: &Path) -> std::io::Result<()> {
    let is_real_dir = std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_dir())
        .unwrap_or(false);
    if is_real_dir {
        std::fs::remove_dir_all(path)
    } else {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            // 指向目录的 junction：remove_file 恒失败，remove_dir 只摘链接
            Err(_) if path.is_dir() => std::fs::remove_dir(path),
            Err(e) => Err(e),
        }
    }
}

/// 路径明确不存在（读不到不算不存在）。
fn is_gone(path: &Path) -> bool {
    matches!(
        std::fs::symlink_metadata(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound
    )
}

/// 逐项删除 `paths`，返回**仍然存在**的那些（交给调用方排重启或重试）。
pub fn remove_all(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|p| !is_gone(p) && remove_entry(p).is_err() && !is_gone(p))
        .cloned()
        .collect()
}

/// 安装目录里还有别的内容时目录本身不删；只在已空时删它，返回目录是否已不存在。
pub fn remove_dir_if_empty(dir: &Path) -> bool {
    is_gone(dir) || std::fs::remove_dir(dir).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("wi_owned_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn lists_only_first_level_sorted() {
        let d = tmp("list");
        std::fs::create_dir_all(d.join("data/sub")).unwrap();
        std::fs::write(d.join("b.exe"), "x").unwrap();
        std::fs::write(d.join("data/sub/f"), "x").unwrap();
        assert_eq!(top_level_names(&d).unwrap(), ["b.exe", "data"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unsafe_names_are_dropped() {
        let base = Path::new("C:/App");
        let names: Vec<String> = ["ok.exe", "", "..", ".", "a/b", r"a\b", "C:x", "data"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let got = owned_paths(base, &names);
        assert_eq!(got, [base.join("ok.exe"), base.join("data")]);
    }

    #[test]
    fn removes_owned_keeps_foreign_and_keeps_dir() {
        let d = tmp("keep");
        std::fs::create_dir_all(d.join("data/themes")).unwrap();
        std::fs::write(d.join("data/themes/a"), "x").unwrap();
        std::fs::write(d.join("app.exe"), "x").unwrap();
        // 用户自建的嵌套数据目录与第三方定制层：不在清单里
        std::fs::create_dir_all(d.join("UserData")).unwrap();
        std::fs::write(d.join("UserData/cfg"), "x").unwrap();
        std::fs::create_dir_all(d.join("data_custom")).unwrap();

        let owned = owned_paths(&d, &["app.exe".into(), "data".into()]);
        assert!(remove_all(&owned).is_empty());
        assert!(!d.join("app.exe").exists() && !d.join("data").exists());
        assert!(d.join("UserData/cfg").exists());
        assert!(d.join("data_custom").exists());
        // 目录非空：不删
        assert!(!remove_dir_if_empty(&d));
        assert!(d.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn empty_dir_is_removed_after_everything_owned_is_gone() {
        let d = tmp("empty");
        std::fs::write(d.join("app.exe"), "x").unwrap();
        assert!(remove_all(&owned_paths(&d, &["app.exe".into()])).is_empty());
        assert!(remove_dir_if_empty(&d));
        assert!(!d.exists());
    }

    #[test]
    fn missing_owned_entries_are_not_failures() {
        let d = tmp("missing");
        assert!(remove_all(&owned_paths(&d, &["never_there".into()])).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
