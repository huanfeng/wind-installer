//! `[paths].user_data`：卸载器「删除用户数据」的目标目录解析。
//!
//! 断言的是 `resolve_user_data_dir()`（确认页显示）与 `CleanupOptions::user_data_dir()`
//! （实际删除）给出同一个答案，且是清单声明的目录（AGENTS.md 规则 5）。
//! 未声明时沿用 `%APPDATA%\{id}` 的缺省行为由 `wizard_data_dir.rs` 守护。
//!
//! 文件名不含 "install"/"setup"/"update"/"patch"（UAC 安装器检测启发式）。

#![cfg(windows)]

use std::path::PathBuf;
use wind_installer::installer::step::Reporter;
use wind_installer::manifest::AppManifest;
use wind_installer::{meta, uninstaller::cleanup};

const MANIFEST: &str = r#"
[app]
id           = "Demo"
display_name = "Demo App"
version      = "1.0.0"
publisher    = "Demo Inc"
main_exe     = "demo.exe"

[paths]
user_data = '%APPDATA%\vendor\{id}'
"#;

/// 吞掉所有输出的 Reporter，只为满足接口。
struct NullReporter;

impl Reporter for NullReporter {
    fn step_begin(&mut self, _index: usize, _total: usize, _name: &str) {}
    fn step_progress(&mut self, _detail: &str, _fraction: f32) {}
    fn log(&mut self, _msg: &str) {}
    fn warn(&mut self, msg: &str) {
        panic!("unexpected warning: {msg}");
    }
}

#[test]
fn uninstaller_targets_declared_user_data_dir() {
    meta::init(AppManifest::from_toml_bytes(MANIFEST.as_bytes()).expect("解析清单失败"));

    let roaming = std::env::temp_dir().join(format!("wind-userdata-{}", std::process::id()));
    std::fs::create_dir_all(&roaming).expect("建临时目录失败");
    std::env::set_var("APPDATA", &roaming);
    // 没有 [datadir] 声明，也就没有 conf 文件可读：不依赖 LOCALAPPDATA 的内容
    let expected: PathBuf = roaming.join("vendor").join("Demo");

    let shown = cleanup::resolve_user_data_dir();
    let deleted = cleanup::CleanupOptions::default().user_data_dir();
    assert_eq!(shown, expected);
    assert_eq!(shown, deleted, "确认页显示的路径与实际删除的路径不一致");

    // 实际删除：整个声明的目录被删，同级兄弟目录不受影响
    let sibling = roaming.join("vendor").join("Other");
    std::fs::create_dir_all(&expected).expect("建数据目录失败");
    std::fs::write(expected.join("a.db"), b"x").expect("写文件失败");
    std::fs::create_dir_all(&sibling).expect("建兄弟目录失败");
    let mut reporter = NullReporter;
    let options = cleanup::CleanupOptions {
        clean_roaming: true,
        backup_to_desktop: false,
        ..Default::default()
    };
    cleanup::cleanup_user_data(&options, &shown, &mut reporter).expect("清理失败");
    assert!(!expected.exists(), "声明的用户数据目录应被删除");
    assert!(sibling.exists(), "同级目录不应被碰");

    let _ = std::fs::remove_dir_all(&roaming);
}
