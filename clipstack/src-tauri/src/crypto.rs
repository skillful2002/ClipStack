// P0 · 加密核心（应用层内容加密）
//
// 设计（见 `docs/安全改造方案.md` §4）：
//   - 仅加密 `content_text` / `content_blob` 两列，其余字段（hash / 索引 / 时间）保持明文，
//     以便去重与查询无需解密。
//   - 内容用 AES-256-GCM 加密（`nonce(12B) || ciphertext`），`content_text` 以 base64 存 TEXT、
//     `content_blob` 以原始字节存 BLOB。
//   - 数据库内容加密密钥为内部固定密钥（`Key` 包裹，drop 时清零），独立于用户主密码；
//     主密码仅经 Argon2id 计算 `pw_verifier` 用于应用锁校验，不再派生任何加密密钥。

use std::fmt;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use hmac::Hmac;
use pbkdf2::pbkdf2;
use rand::RngCore;
use sha2::{Digest, Sha256};
use zeroize::ZeroizeOnDrop;

/// 32 字节主密钥。派生于主密码；内存中持有，drop 时自动清零。
#[derive(Clone, ZeroizeOnDrop)]
pub struct Key(pub [u8; 32]);

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Key(***)")
    }
}

/// 统一的 Argon2id 参数：64 MiB 内存、3 轮、4 并行度。派生密钥与校验哈希共用，保证一致。
fn argon2() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(64 * 1024, 3, 4, None).expect("valid argon2 params"),
    )
}

/// 生成 16 字节随机盐（十六进制存储于 settings 的 `pw_salt`）。
pub fn random_salt() -> [u8; 16] {
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    salt
}

/// 计算主密码的 Argon2 哈希（存储为 `pw_verifier`，含盐与参数），用于后续校验。
pub fn hash_password(pwd: &str, salt: &[u8; 16]) -> String {
    let a = argon2();
    let salt_str = SaltString::encode_b64(salt).expect("valid salt");
    let hash = a
        .hash_password(pwd.as_bytes(), &salt_str)
        .expect("argon2 hash failed");
    hash.to_string()
}

/// 校验主密码是否匹配已存储的 `pw_verifier`。
pub fn verify_password(pwd: &str, verifier: &str) -> bool {
    let a = argon2();
    match PasswordHash::new(verifier) {
        Ok(ph) => a.verify_password(pwd.as_bytes(), &ph).is_ok(),
        Err(_) => false,
    }
}

/// 加密：返回 `nonce(12B) || ciphertext`。
pub fn encrypt(key: &Key, pt: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new_from_slice(&key.0).expect("valid key length");
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), pt)
        .expect("aes-gcm encrypt failed");
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

/// 解密：`sealed = nonce(12B) || ciphertext`。失败（密钥错误 / 数据损坏）返回 None。
pub fn decrypt(key: &Key, sealed: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < 12 {
        return None;
    }
    let (nonce, ct) = sealed.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(&key.0).ok()?;
    cipher.decrypt(Nonce::from_slice(nonce), ct).ok()
}

// ===== 局域网共享：PSK 派生（替换全量设计的 ECDH） =====
//
// 设计（见 `clipstack-lan-sync-design.md` §六）：对称密钥由「共享组 + 密钥」派生，
// `sym_key = PBKDF2-HMAC-SHA256(share_key, salt = SHA256(share_group), rounds = 100_000)`。
// 与内部数据库密钥（`Key`）共用同一封装，drop 时自动清零。

/// 分组匹配指纹：仅广播前 8 字节，绝不泄露密钥原文（设计 §3.3）。
pub fn group_fingerprint(share_group: &str, share_key: &str) -> String {
    let mut h = Sha256::new();
    h.update(share_group.as_bytes());
    h.update("::".as_bytes());
    h.update(share_key.as_bytes());
    let digest = h.finalize();
    digest[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 由「共享组 + 密钥」派生对称密钥（PSK）。
pub fn derive_psk(share_group: &str, share_key: &str) -> Key {
    let salt = sha256(share_group.as_bytes());
    let mut out = [0u8; 32];
    pbkdf2::<Hmac<Sha256>>(share_key.as_bytes(), &salt, 100_000, &mut out)
        .expect("pbkdf2 derivation failed");
    Key(out)
}

/// 内部：返回 SHA256 原始 32 字节（作 PBKDF2 salt）。
fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    let d = h.finalize();
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&d);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_and_verify_roundtrip() {
        let salt = random_salt();
        let verifier = hash_password("hunter2", &salt);
        assert!(verify_password("hunter2", &verifier));
        assert!(!verify_password("wrong", &verifier));
    }

    #[test]
    fn aes_roundtrip() {
        let key = Key([0u8; 32]);
        let pt = b"secret clipboard content";
        let sealed = encrypt(&key, pt);
        assert_ne!(&sealed[..], pt);
        let out = decrypt(&key, &sealed).expect("decrypt");
        assert_eq!(out, pt);
    }

    #[test]
    fn wrong_key_cannot_decrypt() {
        let k1 = Key([1u8; 32]);
        let k2 = Key([2u8; 32]);
        let sealed = encrypt(&k1, b"data");
        assert!(decrypt(&k2, &sealed).is_none());
    }

    #[test]
    fn key_zeroized_on_drop() {
        // 仅验证 Key 可正常构造与 drop（zeroize 在 drop 时执行，无 panic 即可）。
        let key = Key([0u8; 32]);
        assert_eq!(key.0.len(), 32);
        drop(key);
    }

    #[test]
    fn group_fingerprint_consistent_and_secret() {
        let fp1 = group_fingerprint("home", "s3cret");
        let fp2 = group_fingerprint("home", "s3cret");
        let fp3 = group_fingerprint("home", "other");
        let fp4 = group_fingerprint("office", "s3cret");
        assert_eq!(fp1, fp2);
        assert_ne!(fp1, fp3); // 密钥不同 -> 指纹不同
        assert_ne!(fp1, fp4); // 组不同 -> 指纹不同
        assert_eq!(fp1.len(), 16); // 8 字节十六进制
        assert!(!fp1.contains("s3cret")); // 不含密钥原文
    }

    #[test]
    fn psk_derivation_roundtrip() {
        let k1 = derive_psk("home", "s3cret");
        let k2 = derive_psk("home", "s3cret");
        let k3 = derive_psk("home", "wrong");
        assert_eq!(k1.0, k2.0);
        assert_ne!(k1.0, k3.0);
        // 派生密钥可加解密
        let sealed = encrypt(&k1, b"lan payload");
        assert_eq!(decrypt(&k2, &sealed).unwrap(), b"lan payload");
        assert!(decrypt(&k3, &sealed).is_none());
    }
}
