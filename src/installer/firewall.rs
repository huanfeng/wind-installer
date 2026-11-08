//! 防火墙入站规则 —— 通过 `netsh advfirewall` 添加/删除按程序放行的规则。
//!
//! 安装器/卸载器本身已是管理员进程（写 HKLM 的前提），netsh 继承其令牌，
//! 这里不再另行提权。

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use crate::manifest::{FirewallInfo, FirewallProtocol};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 添加一条入站放行规则，返回规则所指程序的绝对路径（写回执用）。
///
/// 幂等：先按名字 + 程序删除同名旧规则（不存在时 netsh 报错，忽略），再添加。
pub fn add_rule(rule: &FirewallInfo, install_dir: &Path) -> Result<String, String> {
    let profile = profile_arg(&rule.profiles)?;
    let program = install_dir
        .join(&rule.program)
        .to_string_lossy()
        .to_string();

    let _ = delete_rule(&rule.name, &program);

    run_netsh(&[
        "advfirewall".into(),
        "firewall".into(),
        "add".into(),
        "rule".into(),
        format!("name={}", rule.name),
        "dir=in".into(),
        "action=allow".into(),
        format!("program={}", program),
        "enable=yes".into(),
        format!("profile={}", profile),
        format!("protocol={}", protocol_arg(rule.protocol)),
    ])?;
    Ok(program)
}

/// 删除名字与程序都匹配的规则。
pub fn delete_rule(name: &str, program_abs: &str) -> Result<(), String> {
    run_netsh(&[
        "advfirewall".into(),
        "firewall".into(),
        "delete".into(),
        "rule".into(),
        format!("name={}", name),
        format!("program={}", program_abs),
    ])
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

fn run_netsh(args: &[String]) -> Result<(), String> {
    let output = Command::new("netsh")
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
