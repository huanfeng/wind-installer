//! 防火墙入站规则 —— 通过 `netsh advfirewall` 添加/删除按程序放行的规则。
//!
//! 安装器/卸载器本身已是管理员进程（写 HKLM 的前提），netsh 继承其令牌，
//! 这里不再另行提权。

use std::os::windows::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::manifest::{FirewallInfo, FirewallProtocol};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 添加一条入站放行规则，返回规则所指程序的绝对路径（写回执用）。
///
/// 幂等：先按名字 + 程序删除同名旧规则，再添加。
pub fn add_rule(rule: &FirewallInfo, install_dir: &Path) -> Result<String, String> {
    validate(rule)?;
    let program = install_dir
        .join(&rule.program)
        .to_string_lossy()
        .to_string();
    let args = build_add_args(rule, &program)?;

    delete_rule(&rule.name, &program)?;
    run_netsh(&args)?;
    Ok(program)
}

/// 删除名字与程序都匹配的规则。没有匹配的规则视为成功（已被用户或上次卸载删掉）。
///
/// 先 `show rule` 判断是否存在，而不是解析 delete 的输出：输出文案随系统语言变化，
/// 退出码才是稳定信号（show 在无匹配时返回非零）。
pub fn delete_rule(name: &str, program_abs: &str) -> Result<(), String> {
    if run_netsh(&build_show_args(name, program_abs)).is_err() {
        return Ok(());
    }
    run_netsh(&build_delete_args(name, program_abs))
}

/// 入口校验：规则名与程序路径来自清单，不得用来误删全部规则或逃出安装目录。
fn validate(rule: &FirewallInfo) -> Result<(), String> {
    let name = rule.name.trim();
    if name.is_empty() || name.eq_ignore_ascii_case("all") || name.contains('"') {
        return Err(format!("Invalid firewall rule name: {:?}", rule.name));
    }
    let program = rule.program.trim();
    let p = Path::new(program);
    let escapes = p
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir));
    if program.is_empty() || p.is_absolute() || p.has_root() || escapes {
        return Err(format!("Invalid firewall program path: {:?}", rule.program));
    }
    Ok(())
}

fn build_add_args(rule: &FirewallInfo, program_abs: &str) -> Result<Vec<String>, String> {
    Ok(vec![
        "advfirewall".into(),
        "firewall".into(),
        "add".into(),
        "rule".into(),
        format!("name={}", rule.name),
        "dir=in".into(),
        "action=allow".into(),
        format!("program={}", program_abs),
        "enable=yes".into(),
        format!("profile={}", profile_arg(&rule.profiles)?),
        format!("protocol={}", protocol_arg(rule.protocol)),
    ])
}

fn build_delete_args(name: &str, program_abs: &str) -> Vec<String> {
    vec![
        "advfirewall".into(),
        "firewall".into(),
        "delete".into(),
        "rule".into(),
        format!("name={}", name),
        format!("program={}", program_abs),
    ]
}

fn build_show_args(name: &str, program_abs: &str) -> Vec<String> {
    vec![
        "advfirewall".into(),
        "firewall".into(),
        "show".into(),
        "rule".into(),
        format!("name={}", name),
        format!("program={}", program_abs),
    ]
}

fn protocol_arg(p: FirewallProtocol) -> &'static str {
    match p {
        FirewallProtocol::Any => "any",
        FirewallProtocol::Tcp => "tcp",
        FirewallProtocol::Udp => "udp",
    }
}

/// 把清单里的 profile 列表规整为 netsh 的逗号连接形式；未知取值直接报错，
/// 不静默放宽到别的网络类型。
fn profile_arg(profiles: &[String]) -> Result<String, String> {
    if profiles.is_empty() {
        return Err("Firewall rule has no profiles".into());
    }
    let mut out = Vec::new();
    for p in profiles {
        let p = p.trim().to_ascii_lowercase();
        match p.as_str() {
            "private" | "domain" | "public" => out.push(p),
            _ => return Err(format!("Unknown firewall profile: {}", p)),
        }
    }
    Ok(out.join(","))
}

/// 用 System32 下的绝对路径，避免被当前目录或 PATH 里同名 netsh.exe 劫持。
fn netsh_path() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    PathBuf::from(root).join("System32").join("netsh.exe")
}

fn run_netsh(args: &[String]) -> Result<(), String> {
    let output = Command::new(netsh_path())
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Failed to run netsh: {}", e))?;

    if output.status.success() {
        return Ok(());
    }
    // netsh 把错误信息写在 stdout
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "netsh failed ({}): {} {}",
        output.status,
        stdout.trim(),
        stderr.trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(name: &str, program: &str) -> FirewallInfo {
        FirewallInfo {
            name: name.into(),
            program: program.into(),
            protocol: FirewallProtocol::Tcp,
            profiles: vec!["private".into(), "public".into()],
        }
    }

    #[test]
    fn add_args_keep_spaces_in_single_argv_items() {
        let r = rule("My App", r"bin\app.exe");
        let args = build_add_args(&r, r"C:\Program Files\My App\bin\app.exe").unwrap();
        assert_eq!(
            args,
            [
                "advfirewall",
                "firewall",
                "add",
                "rule",
                "name=My App",
                "dir=in",
                "action=allow",
                r"program=C:\Program Files\My App\bin\app.exe",
                "enable=yes",
                "profile=private,public",
                "protocol=tcp",
            ]
        );
    }

    #[test]
    fn delete_args_carry_name_and_program() {
        assert_eq!(
            build_delete_args("My App", r"C:\P F\a.exe"),
            [
                "advfirewall",
                "firewall",
                "delete",
                "rule",
                "name=My App",
                r"program=C:\P F\a.exe"
            ]
        );
    }

    #[test]
    fn validate_rejects_dangerous_names_and_paths() {
        assert!(validate(&rule("X", "a.exe")).is_ok());
        assert!(validate(&rule("X", r"sub\a.exe")).is_ok());
        for bad in ["", "  ", "all", "ALL", "a\"b"] {
            assert!(validate(&rule(bad, "a.exe")).is_err(), "{bad}");
        }
        for bad in ["", r"..\a.exe", r"sub\..\..\a.exe", r"C:\a.exe", r"\a.exe"] {
            assert!(validate(&rule("X", bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn profiles_join_with_comma_and_normalize_case() {
        let p = vec!["Private".to_string(), "domain".to_string()];
        assert_eq!(profile_arg(&p).unwrap(), "private,domain");
    }

    #[test]
    fn unknown_or_empty_profiles_are_rejected() {
        assert!(profile_arg(&["any".to_string()]).is_err());
        assert!(profile_arg(&[]).is_err());
    }
}
