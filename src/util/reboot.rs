//! 重启延迟文件操作 —— 处理「被占用、当前删不掉」的文件的通用兜底。
//!
//! Windows 允许对已打开的文件做同卷改名（只改目录项），却不允许删除被持锁进程占用的
//! 文件。标准做法是把删除**排进下次启动队列**：登记到
//! `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\PendingFileRenameOperations`，
//! 由会话管理器（smss.exe）在开机极早期、文件尚未被任何进程加载前执行——那时删除必然成功。
//!
//! 这是 NSIS `Delete /REBOOTOK`、MSI `InstallFinalize` 背后的同一机制，属通用安装器能力，
//! 与具体应用无关。需要管理员权限（本安装/卸载器已通过 manifest 提权）。
//!
//! # 为什么有一本进程级账本
//!
//! 「有旧文件删不掉」这个事实产生在最底层的 IO 处（解压时的 `create_or_backup`、
//! 旧版遗留清理、卸载时的文件删除），那里既没有 `Step` 上下文也没有 `Reporter`；
//! 而需要它的是最顶层——完成页要提示用户重启、quiet 模式要据此**放弃自动退出**。
//! 逐层改签名会把 `need_reboot` 穿进 `extract_entry`/`extract_all` 等与之无关的 API。
//!
//! `MoveFileExW(DELAY_UNTIL_REBOOT)` 本身就是写进程外全局状态的操作，故这里用
//! [`LEDGER`] 记账只是把它已有的全局性显式化，[`run_plan`] 在收尾处一次性读取汇总。
//!
//! [`run_plan`]: crate::installer::step::run_plan

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT};

/// 一条「当前删不掉、留待重启处理」的记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingItem {
    /// 被推迟的路径（升级场景下通常是改名后的 `.old_xxxxxxxx`）。
    pub path: PathBuf,
    /// 是否成功登记到 `PendingFileRenameOperations`。
    ///
    /// `false` 表示连排队都失败了——重启也不会自动清理，属于更严重的残留，
    /// 但对用户的建议是一样的（重启后进程不再持锁，可手工删或由下次安装清掉）。
    pub scheduled: bool,
}

fn ledger() -> &'static Mutex<Vec<PendingItem>> {
    static LEDGER: OnceLock<Mutex<Vec<PendingItem>>> = OnceLock::new();
    LEDGER.get_or_init(|| Mutex::new(Vec::new()))
}

/// 取账本；忽略中毒（记账线程 panic 不该让后续记账全部失败）。
fn ledger_lock() -> std::sync::MutexGuard<'static, Vec<PendingItem>> {
    ledger().lock().unwrap_or_else(|e| e.into_inner())
}

/// 安排在下次系统启动时删除 `path`，并记入本次运行的账本。
///
/// 用于锁定文件的兜底清理：当前删不掉就登记 `MoveFileExW(path, NULL, DELAY_UNTIL_REBOOT)`，
/// 下次开机由会话管理器删除。对空目录同样有效（可删除重启时已清空的残留目录）。
///
/// 无论成功与否都会记账：调用方只需 best-effort 调用，「是否该提示用户重启」由
/// [`is_reboot_pending`] 统一回答。失败（如权限不足、路径非法）返回 `Err`，
/// 由调用方决定是否降级为普通告警——本函数不 panic、不影响主流程。
pub fn schedule_delete_on_reboot(path: &Path) -> Result<(), String> {
    // 需以 NUL 结尾的宽字符串；空目标（第二参 NULL）表示「删除」而非「移动」。
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let result = unsafe {
        MoveFileExW(
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            MOVEFILE_DELAY_UNTIL_REBOOT,
        )
    }
    .map_err(|e| format!("排定重启删除失败 {:?}: {}", path, e));

    record_pending(path, result.is_ok());
    result
}

/// 直接记一笔「当前处理不掉的残留」而不尝试排队。
///
/// 用于连改名让路都做不到、或路径本就不该进 `PendingFileRenameOperations` 的场景：
/// 事实仍需被计入「建议重启」的依据。
pub fn record_pending(path: &Path, scheduled: bool) {
    let item = PendingItem {
        path: path.to_path_buf(),
        scheduled,
    };
    let mut guard = ledger_lock();
    // 同一路径可能被多次尝试（改名失败后重试）；只保留一条，成功状态取「曾经成功过」。
    if let Some(existing) = guard.iter_mut().find(|i| i.path == item.path) {
        existing.scheduled |= scheduled;
        return;
    }
    guard.push(item);
}

/// 本次运行是否留下了需要重启才能清理干净的东西。
pub fn is_reboot_pending() -> bool {
    !ledger_lock().is_empty()
}

/// 账本快照，供日志与诊断使用。
pub fn pending_items() -> Vec<PendingItem> {
    ledger_lock().clone()
}

/// 一行人类可读摘要，如 `3 个文件待重启后清理（其中 1 个未能排队）`。
/// 账本为空时返回空串。
pub fn pending_summary() -> String {
    let items = pending_items();
    if items.is_empty() {
        return String::new();
    }
    let unscheduled = items.iter().filter(|i| !i.scheduled).count();
    if unscheduled == 0 {
        format!("{} 个文件待重启后清理", items.len())
    } else {
        format!(
            "{} 个文件待重启后清理（其中 {} 个未能排入删除队列）",
            items.len(),
            unscheduled
        )
    }
}

/// 清空账本。**仅供测试**——一次真实运行内不存在「重新开始计数」的正当场景。
///
/// 之所以是 `pub` 而非 `#[cfg(test)]`：本 crate 的 `--lib` 单测二进制名含 "install"，
/// 会命中 Windows UAC 安装器检测启发式而无法在普通权限下启动（见 `ui::theme` 末注），
/// 账本的测试只能放在 `tests/` 集成测试里，那里链接的是非 test 编译的 lib。
#[doc(hidden)]
#[allow(dead_code)] // 只被 tests/ 引用；bin target 看不到那边的用法
pub fn reset_ledger() {
    ledger_lock().clear();
}

// ── 目录树的重启删除 ─────────────────────────────────────────────────────────

/// 把一棵删不掉的目录树排进重启删除队列。
///
/// `MoveFileExW` 对**非空**目录无效，故必须自底向上逐项处理：先文件、再子目录、
/// 最后目录自身。只排目录一条的话，重启时它仍非空，删除会静默失败——这正是
/// 「安装目录里有文件被占用 ⇒ 重启兜底整棵树都无效」这个缺口的成因。
///
/// 三条不变量，改动时勿破：
///
/// 1. **能当场删掉的东西不进队列。** 无占用时走完这里一条记录都不留，账本为空，
///    「需要重启」的结论不受影响；队列长度只反映真正卡住的东西（实测安装目录
///    ~100 文件 / ~40 目录，即便全卡住也远低于注册表值 1MB 的标准格式上限）。
/// 2. **删不掉的文件先改名 `.old_xxxxxxxx` 再排队。** 排进
///    `PendingFileRenameOperations` 的删除要到**下次开机**才执行，而用户完全可能
///    在此之前就重装到同一目录。排原路径 = 让上一次卸载的遗留指令删掉新装的同名
///    文件（重启后「词库莫名消失」）；排一个随机后缀名则永不与新装产物撞名。
///    这与解压让路、卸载删二进制是同一套路。
/// 3. **正在运行的自身可执行文件一概不碰。** 改名后 `current_exe()` 仍返回旧路径
///    （`GetModuleFileNameW` 在加载时固化），卸载器的自删除流程会因此复制不到自己而
///    静默失效；排队它的原路径又会在重装后删掉新的 `uninstall.exe`，把 ARP 条目
///    指向不存在的程序。它由 `uninstaller::selfdelete` 负责，这里让开。
pub fn schedule_dir_on_reboot(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        // 连列目录都做不到（权限/句柄问题）：至少把目录本身记一笔，
        // 让「需要重启」的结论不会因为这里读不到而丢失。
        record_pending(dir, false);
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        // 用 `file_type()` 而非 `path.is_dir()`：后者跟随符号链接/junction，会递归进
        // 链接目标把**目标目录**的内容删掉。`FileType::is_dir()` 对重解析点返回 false，
        // 于是链接按「文件」处理——删的是链接本身，不是它指向的东西。
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            schedule_dir_on_reboot(&path);
        } else if !is_current_exe(&path) && std::fs::remove_file(&path).is_err() {
            // 走到这里的「非目录」除了普通文件，还有指向目录的重解析点（junction /
            // 目录符号链接）——对它们 `remove_file` 恒为「拒绝访问」，而 `remove_dir`
            // 直接成功且**只摘掉链接、目标目录安然无恙**。不先试这一下的话，安装目录
            // 里只要有一个 junction，每次卸载都会白白多一条重启账、完成页无端提示
            // 「需要重启」，还留下一个改了名的链接。
            if std::fs::remove_dir(&path).is_err() {
                let _ = schedule_delete_on_reboot(&stash_aside(&path));
            }
        }
    }

    // 子项清空后目录本身通常就能删掉——能删就删，别往队列里塞无谓的条目（不变量 1）。
    if std::fs::remove_dir(dir).is_err() {
        let _ = schedule_delete_on_reboot(dir);
    }
}

/// 把删不掉的文件改名让路，返回**实际要排队的路径**；改名失败则原路返回。
///
/// 同卷改名只改目录项，对已被打开/已加载为映像的文件同样成立；只有被以不含
/// `FILE_SHARE_DELETE` 方式打开的文件会失败，那时退回排原路径（见不变量 2）。
/// 调用方有两处：卸载删除安装目录二进制，以及 IME 系统副本的覆盖/删除让路
/// （`installer::ime`）——后者不在安装目录里，`delete_install_files` 够不着。
///
/// **已经是 `.old_xxxxxxxx` 的名字原样返回、不再改一次。** 这不是洁癖：卸载时
/// `delete_install_files` 的 binaries 循环会先把锁定的 `app.exe` 改成
/// `app.exe.old_aaaaaaaa` 并排队，随后 `remove_dir_all` 失败、递归兜底又碰上同一个
/// 文件。再改一次名的后果是队列里出现**两条**——先排的那条指向已不存在的路径，成了
/// 空转指令，而 `pending_summary()` 报给用户的「N 个文件待重启后清理」也随之偏大。
/// 那个随机后缀本来就是防撞的，套第二层没有任何收益。
pub(crate) fn stash_aside(path: &Path) -> PathBuf {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return path.to_path_buf();
    };
    if is_stashed_name(&name) {
        return path.to_path_buf();
    }
    let stashed = path.with_file_name(format!("{}.old_{:08x}", name, rand::random::<u32>()));
    if std::fs::rename(path, &stashed).is_ok() {
        stashed
    } else {
        path.to_path_buf()
    }
}

/// 名字是否已是本模块（或 `delete_install_files`）产出的让路名：`.old_` + 8 位十六进制。
fn is_stashed_name(name: &str) -> bool {
    match name.rsplit_once(".old_") {
        Some((stem, suffix)) => {
            !stem.is_empty() && suffix.len() == 8 && suffix.bytes().all(|b| b.is_ascii_hexdigit())
        }
        None => false,
    }
}

/// 该路径是否就是当前进程的可执行文件（见不变量 3）。
///
/// 比较走 `canonicalize` 而非字符串：两边一个来自 `GetModuleFileNameW`、一个由
/// `install_dir.join(名字)` 拼出，可能在 8.3 短名、junction 挂载点、大小写上写法不同，
/// 而**判漏的代价是自删除静默失效**——那种失败没有任何日志能看出来。
///
/// **文件名快速通道的名字必须取自 `canonicalize` 之后的路径**，不能取 `current_exe()`
/// 的原始返回值：进程若是以 8.3 短路径启动的（`install directory` 这类带空格的目录
/// 必有短名），`current_exe()` 原样返回 `...\INSTAL~1\UNINST~1.EXE`，而 `read_dir`
/// 永远给磁盘长名 `uninstall.exe`——两边比不上，函数会**在 canonicalize 之前就返回
/// `false`**，自删除随即静默失效（实测复现过）。canonicalize 后两边都是磁盘长名。
///
/// 取不到自身路径时返回 `false`：那只是让自身 exe 与其他文件同等对待（改名+排队），
/// 不会影响其余条目的处理；反过来返回 `true` 又会放过一个同名文件。两害相权取其轻。
///
/// `pub` 的理由同 [`reset_ledger`]：本 crate 的 `--lib` 单测跑不起来（二进制名含
/// "install"，命中 UAC 安装器启发式），这条不变量只能在 `tests/` 里断言。
#[doc(hidden)]
pub fn is_current_exe(path: &Path) -> bool {
    static SELF_EXE: OnceLock<Option<(std::ffi::OsString, PathBuf)>> = OnceLock::new();
    let Some((self_name, self_real)) = SELF_EXE
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            let real = std::fs::canonicalize(&exe).unwrap_or(exe);
            let name = real.file_name()?.to_os_string();
            Some((name, real))
        })
        .as_ref()
    else {
        return false;
    };

    // 先比文件名（无 IO）——绝大多数条目在这里就否掉了，不必为每个文件做一次
    // `canonicalize` 系统调用。
    if path.file_name().map(|n| n.eq_ignore_ascii_case(self_name)) != Some(true) {
        return false;
    }
    std::fs::canonicalize(path).as_deref().unwrap_or(path) == self_real.as_path()
}
