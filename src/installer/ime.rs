use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows::core::s;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

use super::acl;
use crate::manifest::ImeInfo;
use crate::meta;
use crate::util::reboot;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 取清单中的 ime 段；无则视为无输入法（各函数空操作）。
fn ime_cfg() -> Option<&'static ImeInfo> {
    meta::manifest().ime.as_ref()
}

/// 注册结果：**已注册成功的 DLL** + 失败信息。
///
/// 不用 `Result<(), String>` 是因为回执必须记下「已经做成的部分」：x64 成功、x86 失败时
/// 若整体返回 Err 而丢掉 x64 那条，卸载后 x64 的 CLSID 就永久滞留、且指向已删除的 DLL。
pub struct ComRegistration {
    /// `(系统目录副本的绝对路径, 是否用 SysWOW64 的 regsvr32)`
    pub registered: Vec<(PathBuf, bool)>,
    pub errors: Vec<String>,
}

/// 注册 COM 组件：把 TSF DLL 复制到**系统目录**，对系统副本跑 regsvr32（无窗口）。
///
/// 为什么必须对系统副本注册：`DllRegisterServer` 内部用 `GetModuleFileName` 取被加载
/// 模块的路径写进 `InprocServer32`——对哪个副本跑 regsvr32，注册就指向哪个副本。而
/// Win11 输入栈（GIP 路径，游戏聊天框等上下文）激活 in-proc TSF IME 前，msctf 会把
/// DLL 不在系统目录的 COM 服务器静默筛除（连 `DllGetClassObject` 都不调）。对安装
/// 目录副本注册的输入法因此在游戏等场景不可用；复制到系统目录再对副本注册，
/// `InprocServer32` 自然指向系统路径。
///
/// - x64 → `%WINDIR%\System32`
/// - x86 → `%WINDIR%\SysWOW64`（用 SysWOW64 的 regsvr32，失败仅计入告警，语义同前）
pub fn register_com(install_dir: &Path) -> ComRegistration {
    let mut out = ComRegistration {
        registered: Vec::new(),
        errors: Vec::new(),
    };

    let Some(ime) = ime_cfg() else {
        return out;
    };

    let x64 = install_dir.join(&ime.dll_x64);
    if x64.exists() {
        let target = system_deploy_path(&ime.dll_x64, false);
        match deploy_and_register(&x64, &target, false) {
            Ok(()) => out.registered.push((target, false)),
            Err(e) => out.errors.push(e),
        }
    }

    if !ime.dll_x86.is_empty() {
        let x86 = install_dir.join(&ime.dll_x86);
        if x86.exists() {
            let target = system_deploy_path(&ime.dll_x86, true);
            match deploy_and_register(&x86, &target, true) {
                Ok(()) => out.registered.push((target, true)),
                Err(e) => out.errors.push(e),
            }
        }
    }

    out
}

/// 复制到系统目录 → 授予 AppContainer 读权限 → 对系统副本注册。
///
/// 授权放在注册前：走到注册这一步产物才算「做成」并进回执；顺序反过来的话，注册
/// 成功而授权失败会让这条从回执里丢失，卸载时就撤销不掉已成立的注册。
fn deploy_and_register(src: &Path, target: &Path, wow64: bool) -> Result<(), String> {
    copy_to_system_dir(src, target)?;
    acl::grant_app_packages_rx(target)?;
    run_regsvr32(target, wow64, false)
}

/// 系统目录部署位置：x64 → System32，x86 → SysWOW64，文件名取自清单。
fn system_deploy_path(file_name: &str, wow64: bool) -> PathBuf {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_string());
    let sub = if wow64 { "SysWOW64" } else { "System32" };
    Path::new(&windir).join(sub).join(file_name)
}

/// 复制 DLL 到系统目录，覆盖旧副本（升级时安装目录与系统目录都可能已有旧版本）。
///
/// 系统目录里的旧副本可能仍被宿主进程（如 ctfmon）加载为映像，原地截断写入会被拒：
/// 改名让路 + 排重启删除，再重试复制——与解压让路是同一套路。删不掉的事实经 `reboot`
/// 账本记账，否则「需要重启」的结论会漏判。
fn copy_to_system_dir(src: &Path, dst: &Path) -> Result<(), String> {
    match std::fs::copy(src, dst) {
        Ok(_) => Ok(()),
        Err(e) => {
            if !dst.exists() {
                return Err(format!("复制 {:?} 到 {:?} 失败: {}", src, dst, e));
            }
            let _ = reboot::schedule_delete_on_reboot(&reboot::stash_aside(dst));
            std::fs::copy(src, dst)
                .map(|_| ())
                .map_err(|e| format!("复制 {:?} 到 {:?} 失败: {}", src, dst, e))
        }
    }
}

/// 删除系统目录里的 DLL 副本。仍被加载删不掉时改名让路 + 排重启删（记账），
/// 保证卸载/升级后系统目录不滞留旧副本，且「需要重启」不漏判。
fn remove_system_copy(path: &Path) {
    if path.exists() && std::fs::remove_file(path).is_err() {
        let _ = reboot::schedule_delete_on_reboot(&reboot::stash_aside(path));
    }
}

/// 调用 regsvr32 注册/反注册单个 DLL，检查退出码。
fn run_regsvr32(dll: &Path, wow64: bool, unregister: bool) -> Result<(), String> {
    let program = if wow64 {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".to_string());
        Path::new(&windir).join("SysWOW64").join("regsvr32.exe")
    } else {
        PathBuf::from("regsvr32")
    };

    let mut args: Vec<&str> = Vec::new();
    if unregister {
        args.push("/u");
    }
    args.push("/s");
    let dll_str = dll.to_string_lossy();
    args.push(&dll_str);

    let output = Command::new(&program)
        .args(&args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("无法运行 {:?}: {}", program, e))?;

    if !output.status.success() {
        return Err(format!(
            "regsvr32{} 失败 {:?}: {}",
            if unregister { " /u" } else { "" },
            dll,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(())
}

/// 反注册旧 COM 组件（无窗口）：对**系统目录**里的现有副本跑 regsvr32 /u，成功后
/// 删除该副本，为随后的 `RegisterCom` 部署新副本让路。
///
/// 只认系统副本——`InprocServer32` 指向的是上次 regsvr32 实际加载的那份 DLL，即系统
/// 副本（见 [`register_com`]）；安装目录旧副本不再用于反注册，它由解压正向覆盖。
/// 尽力而为、不报错：个别失败留给随后的注册覆盖与残留清扫兜底。
pub fn unregister_old_com(_install_dir: &Path) -> Result<(), String> {
    let Some(ime) = ime_cfg() else {
        return Ok(());
    };

    let x64 = system_deploy_path(&ime.dll_x64, false);
    if x64.exists() && run_regsvr32(&x64, false, true).is_ok() {
        remove_system_copy(&x64);
    }

    if !ime.dll_x86.is_empty() {
        let x86 = system_deploy_path(&ime.dll_x86, true);
        if x86.exists() && run_regsvr32(&x86, true, true).is_ok() {
            remove_system_copy(&x86);
        }
    }

    Ok(())
}

/// TSF profile 字符串：`<lang_id>:<clsid><profile_guid>`
fn profile_string(ime: &ImeInfo) -> String {
    format!("{}:{}{}", ime.lang_id, ime.clsid, ime.profile_guid)
}

// ── 回执支持：记录产物 / 按产物撤销（不查清单）──────────────────────────────

/// 当前清单的 TSF profile 字符串，供安装步骤写入回执。
pub fn profile_id() -> Option<String> {
    ime_cfg().map(profile_string)
}

/// 按绝对路径反注册 COM DLL，成功后删除该 DLL 文件。回执驱动——不读清单，故升级
/// 换了 DLL 名也能撤销旧的。
///
/// 注册的目标是系统目录副本（见 [`register_com`]），它不在安装目录里、
/// `DeleteInstallFiles` 够不着，不在此删就会永久滞留系统目录。对旧回执里安装目录
/// 路径的条目同样安全：那本就是要随 `DeleteInstallFiles` 删除的文件，提前一步无害。
///
/// 检查 regsvr32 退出码：反注册失败必须让 `UndoReceipt` 汇报，否则用户以为卸载干净了，
/// 实际 COM 仍注册着——此时不删文件，留给下次安装的注册覆盖。
pub fn unregister_com_path(dll: &Path, wow64: bool) -> Result<(), String> {
    if !dll.exists() {
        return Ok(());
    }
    run_regsvr32(dll, wow64, true)?;
    remove_system_copy(dll);
    Ok(())
}

/// 按 profile 字符串反注册输入法。回执驱动——不读清单。
pub fn unregister_profile(profile: &str) -> Result<(), String> {
    call_install_layout_or_tip(profile, 0x0000_0001)
}

/// 注册系统输入法 — 直接调用 input.dll!InstallLayoutOrTip，无 PowerShell 窗口
pub fn register_input_method() -> Result<(), String> {
    let Some(ime) = ime_cfg() else {
        return Ok(());
    };
    call_install_layout_or_tip(&profile_string(ime), 0)
}

// 反注册输入法请用 `unregister_profile`：它按回执记录的 profile 字符串撤销，
// 不依赖当前清单。

/// 直接通过 input.dll FFI 调用 InstallLayoutOrTip。
/// 等同于 NSIS `System::Call 'input::InstallLayoutOrTip(t, i)'`，无任何子进程/窗口开销。
fn call_install_layout_or_tip(profile: &str, flags: u32) -> Result<(), String> {
    type Fn = unsafe extern "system" fn(*const u16, u32) -> bool;

    unsafe {
        let lib_name: Vec<u16> = "input.dll\0".encode_utf16().collect();
        let hmod = LoadLibraryW(windows::core::PCWSTR(lib_name.as_ptr()))
            .map_err(|e| format!("Failed to load input.dll: {}", e))?;

        let proc = GetProcAddress(hmod, s!("InstallLayoutOrTip"));
        match proc {
            Some(f) => {
                let f: Fn = std::mem::transmute(f);
                let wide: Vec<u16> = profile.encode_utf16().chain(std::iter::once(0)).collect();
                if !f(wide.as_ptr(), flags) {
                    eprintln!(
                        "Warning: InstallLayoutOrTip returned false for flags={:#010x}",
                        flags
                    );
                }
            }
            None => {
                return Err("InstallLayoutOrTip not found in input.dll".into());
            }
        }
        // input.dll 是系统 IME 核心库，保持加载到进程退出即可
    }
    Ok(())
}
