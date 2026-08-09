// P0 · 系统钥匙串密钥存储
//
// 两类密钥职责完全分离（见 `docs/安全改造方案.md` 设计调整）：
//   1) 内部数据库加密密钥（ACCOUNT_ENC = "clipstack.enc"，无生物识别保护、设备解锁即可用）：
//      由程序内部生成/持有，与用户主密码无关。数据库内容加密始终使用它，且始终在内存。
//      主密码仅作为「应用锁」凭据，不再派生数据库密钥，因此修改/清除主密码都不影响落库数据。
//   2) Touch ID 解锁：不再使用「钥匙串中存储的秘密」。调用系统 `LocalAuthentication`
//      （LAContext）验证当前登录用户——有 Touch ID 时弹 Touch ID，否则回退到设备登录密码。
//      实现上启动 `/usr/bin/swift` 子进程执行脚本（macOS 26+ 的 LAContext 仅支持异步
//      reply: 版，且裸二进制经 objc2 直接调用会崩溃），解锁凭据即「当前用户身份」，
//      钥匙串中不保留任何可用于解锁的令牌，杜绝令牌泄露风险。

use rand::RngCore;

const SERVICE: &str = "com.clipstack.dbkey";
/// 解锁专用钥匙串项（旧版遗留）：曾用于存放受 BiometryCurrentSet 保护的随机令牌。
/// 新方案改用 LAContext 不再写入它；`delete_unlock_key` 仅在关闭 Touch ID / 清除密码时
/// 清理该遗留项，避免无用项残留。
const ACCOUNT: &str = "clipstack";
/// 加密专用密钥项：无生物识别保护（系统默认 WhenUnlocked，设备解锁即可用），读取不弹 Touch ID。
/// 始终在设置主密码时持久化，供「锁定态」加密存储剪贴板——写入无需交互，复制永不丢失。
const ACCOUNT_ENC: &str = "clipstack.enc";

#[cfg(target_os = "macos")]
use core_foundation::dictionary::CFDictionary;
#[cfg(target_os = "macos")]
use security_framework::access_control::SecAccessControl;

/// 构造钥匙串项字典（供 `store_enc_key` 复用）。
/// `access = Some(...)` 时写入受生物识别保护的项（解锁弹 Touch ID）；
/// `access = None` 时省略访问控制，由系统默认 `WhenUnlocked` 可访问（不强制 Touch ID）。
#[cfg(target_os = "macos")]
fn build_key_dict(key: &[u8; 32], access: Option<&SecAccessControl>, account: &str) -> CFDictionary {
    use core_foundation::base::{TCFType, ToVoid};
    use core_foundation::data::CFData;
    use core_foundation::dictionary::CFMutableDictionary;
    use core_foundation::string::CFString;
    use security_framework_sys::item::{
        kSecAttrAccessControl, kSecAttrAccount, kSecAttrService, kSecClass, kSecClassGenericPassword,
        kSecValueData,
    };
    // kSec* 常量与 wrap_under_get_rule 均为不安全操作，需在 unsafe 块内构造字典。
    unsafe {
        let mut dict = CFMutableDictionary::from_CFType_pairs(&[]);
        dict.add(
            &CFString::wrap_under_get_rule(kSecClass).to_void(),
            &CFString::wrap_under_get_rule(kSecClassGenericPassword).to_void(),
        );
        dict.add(
            &CFString::wrap_under_get_rule(kSecAttrService).to_void(),
            &CFString::new(SERVICE).to_void(),
        );
        dict.add(
            &CFString::wrap_under_get_rule(kSecAttrAccount).to_void(),
            &CFString::new(account).to_void(),
        );
        dict.add(
            &CFString::wrap_under_get_rule(kSecValueData).to_void(),
            &CFData::from_buffer(key).to_void(),
        );
        if let Some(a) = access {
            dict.add(
                &CFString::wrap_under_get_rule(kSecAttrAccessControl).to_void(),
                &a.to_void(),
            );
        }
        dict.to_immutable()
    }
}

/// 调用系统 `LocalAuthentication` 验证当前登录用户，用于 Touch ID 解锁。
///
/// 策略 `DeviceOwnerAuthentication`：有 Touch ID / Face ID 时直接弹生物识别；
/// 无生物识别或校验失败时自动回退到设备登录密码。这是纯系统级用户身份验证，
/// **不在钥匙串中存储任何可用于解锁的秘密**——解锁凭据即「当前用户身份」。
///
/// 实现说明：macOS 26+ 的 LAContext 已**移除同步版** `evaluatePolicy:localizedReason:error:`
/// （只保留异步版 `evaluatePolicy:localizedReason:reply:`），直接用 objc2 绑定调用会在
/// 运行时抛 `unrecognized selector` 崩溃。因此改为启动系统自带的 `/usr/bin/swift`
/// 子进程（Apple 签名、独立进程、崩溃不影响主程序）执行一段脚本，在脚本内调用
/// `LAContext.evaluatePolicy(_:localizedReason:reply:)` 完成 Touch ID / 登录密码验证，
/// 通过 stdout 输出结果。
#[cfg(target_os = "macos")]
pub fn authenticate_user(reason: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // 转义 reason 中的反斜杠与双引号，避免破坏 Swift 字符串字面量。
    let escaped = reason.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"import LocalAuthentication
import Foundation

let ctx = LAContext()
let sem = DispatchSemaphore(value: 0)
var outcome: (ok: Bool, msg: String) = (false, "CANCEL")
ctx.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: "{escaped}") {{ ok, err in
    if ok {{
        outcome.ok = true
    }} else if let e = err {{
        outcome.msg = e.localizedDescription
    }} else {{
        outcome.msg = "CANCEL"
    }}
    sem.signal()
}}
_ = sem.wait(timeout: .distantFuture)
if outcome.ok {{ print("OK") }} else {{ print("ERR:" + outcome.msg) }}
"#
    );

    let mut child = Command::new("/usr/bin/swift")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 Touch ID 验证进程失败: {e}"))?;

    child
        .stdin
        .take()
        .ok_or("无法获取子进程输入".to_string())?
        .write_all(script.as_bytes())
        .map_err(|e| format!("写入验证脚本失败: {e}"))?;

    let output = child
        .wait_with_output()
        .map_err(|e| format!("等待 Touch ID 验证结果失败: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout == "OK" {
        Ok(())
    } else if let Some(msg) = stdout.strip_prefix("ERR:") {
        Err(msg.to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            "用户取消或验证失败".to_string()
        } else {
            stderr
        })
    }
}

/// 持久化「加密专用」密钥项到钥匙串（无生物识别保护）。
///
/// 已不再被 `load_or_create_internal_key` 使用（改为文件系统存储）；
/// 保留供未来可能的钥匙串迁移场景使用。
#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub fn store_enc_key(key: &[u8; 32]) -> Result<(), String> {
    use security_framework::item::add_item;
    let _ = delete_enc_key();
    add_item(build_key_dict(key, None, ACCOUNT_ENC))
        .map_err(|e| format!("写入系统钥匙串（加密密钥）失败: {e:?}"))
}

/// 读取「加密专用」密钥项（无生物识别保护，不弹 Touch ID），供锁定态加密存储。
#[cfg(target_os = "macos")]
pub fn load_enc_key() -> Result<[u8; 32], String> {
    use security_framework::item::{ItemClass, ItemSearchOptions, SearchResult};
    let results = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(SERVICE)
        .account(ACCOUNT_ENC)
        .load_data(true)
        .limit(1i64)
        .search()
        .map_err(|e| format!("读取加密密钥失败: {e:?}"))?;
    let bytes = match results.into_iter().next() {
        Some(SearchResult::Data(d)) => d,
        _ => return Err("钥匙串中未找到加密密钥项（可能尚未设置或已删除）".into()),
    };
    if bytes.len() == 32 {
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(out)
    } else {
        Err(format!("加密密钥长度非法（应为 32 字节，实际 {}）", bytes.len()))
    }
}

/// 将密钥写入文件系统（权限 600），macOS 与非 macOS 共用。
fn write_key_file(path: &std::path::Path, key: &[u8; 32]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|e| format!("创建内部密钥文件失败: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    use std::io::Write;
    f.write_all(key).map_err(|e| format!("写入内部密钥失败: {e}"))?;
    Ok(())
}

/// 内部数据库加密密钥：始终存在，与用户主密码完全无关。
///
/// macOS 优先从文件系统读取（不触发钥匙串、不弹任何系统密码框）；仅在文件不存在时
/// 尝试从旧版钥匙串迁移（一次性），迁移成功后写入文件系统，后续启动不再触碰钥匙串。
/// 非 macOS 平台同样使用文件系统（权限 600）。
pub fn load_or_create_internal_key() -> Result<[u8; 32], String> {
    let home = std::env::var("HOME").map_err(|_| "无法确定用户主目录".to_string())?;
    let path = std::path::Path::new(&home).join(".clipstack").join("dbkey.dat");

    // 1. 优先从文件系统读取（无钥匙串访问、不弹系统密码框）。
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() == 32 {
            let mut out = [0u8; 32];
            out.copy_from_slice(&bytes);
            return Ok(out);
        }
    }

    // 2. 尝试从旧版钥匙串迁移（仅 macOS、仅一次）。
    //    迁移成功后写入文件系统，后续启动走第 1 步，不再触碰钥匙串。
    #[cfg(target_os = "macos")]
    {
        if let Ok(key) = load_enc_key() {
            let _ = write_key_file(&path, &key);
            return Ok(key);
        }
    }

    // 3. 文件系统与钥匙串均无密钥（全新安装或迁移失败）：生成新密钥并写入文件系统。
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    write_key_file(&path, &key)?;
    Ok(key)
}

/// 从系统钥匙串删除加密专用密钥项。
#[cfg(target_os = "macos")]
pub fn delete_enc_key() -> Result<(), String> {
    let _ = std::process::Command::new("/usr/bin/security")
        .args(["delete-generic-password", "-a", ACCOUNT_ENC, "-s", SERVICE])
        .output();
    Ok(())
}

/// 仅删除「解锁专用」(Touch ID) 密钥项，保留「加密专用」项（锁定态加密存储仍依赖它）。
/// 关闭 Touch ID 解锁时调用，确保禁用后无法再用旧钥匙串项绕过指纹校验直接解锁。
#[cfg(target_os = "macos")]
pub fn delete_unlock_key() -> Result<(), String> {
    let _ = std::process::Command::new("/usr/bin/security")
        .args(["delete-generic-password", "-a", ACCOUNT, "-s", SERVICE])
        .output();
    Ok(())
}

/// 从系统钥匙串删除密钥项（解锁专用 + 加密专用）。
#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub fn delete_key() -> Result<(), String> {
    let _ = std::process::Command::new("/usr/bin/security")
        .args(["delete-generic-password", "-a", ACCOUNT, "-s", SERVICE])
        .output();
    let _ = std::process::Command::new("/usr/bin/security")
        .args(["delete-generic-password", "-a", ACCOUNT_ENC, "-s", SERVICE])
        .output();
    Ok(())
}

// ===== 非 macOS 平台：无 Touch ID，解锁统一走主密码路径 =====
// 内部数据库加密密钥改为落盘于数据目录（与数据库同目录，权限 600），使「数据库始终加密」
// 在非 macOS 平台上同样成立。

#[cfg(not(target_os = "macos"))]
pub fn authenticate_user(_reason: &str) -> Result<(), String> {
    Err("当前平台不支持 Touch ID，请使用主密码解锁".into())
}

#[cfg(not(target_os = "macos"))]
#[allow(dead_code)]
pub fn delete_key() -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn delete_unlock_key() -> Result<(), String> {
    Ok(())
}

/// 十六进制编码（仅用于测试与旧项兼容解码验证；生产存储已改用原始字节）。
#[allow(dead_code)]
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// 十六进制解码为 32B；长度或字符非法返回错误。
/// 仅被 `#[cfg(test)]` 用例使用（生产路径已由 LAContext 取代旧 hex 令牌迁移）。
#[allow(dead_code)]
fn hex_decode(s: &str) -> Result<[u8; 32], String> {
    let s = s.trim();
    if s.len() != 64 {
        return Err(format!("密钥长度非法（应为 64 位十六进制，实际 {0}）", s.len()));
    }
    let mut out = [0u8; 32];
    let bytes = s.as_bytes();
    for i in 0..32 {
        let hi = hex_val(bytes[i * 2])?;
        let lo = hex_val(bytes[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

#[allow(dead_code)]
fn hex_val(c: u8) -> Result<u8, String> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(format!("非法十六进制字符: {c}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let key = [0xABu8; 32];
        let hex = hex_encode(&key);
        assert_eq!(hex.len(), 64);
        let back = hex_decode(&hex).unwrap();
        assert_eq!(back, key);
    }

    #[test]
    fn hex_decode_rejects_bad() {
        assert!(hex_decode("zz").is_err()); // 非法字符
        assert!(hex_decode(&"a".repeat(63)).is_err()); // 长度不足 64
        assert!(hex_decode(&"a".repeat(65)).is_err()); // 长度超过 64
    }
}
