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
use rand::RngCore;
use zeroize::ZeroizeOnDrop;

/// 32 字节主密钥。派生于主密码；内存中持有，drop 时自动清零。
#[derive(ZeroizeOnDrop)]
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
}
