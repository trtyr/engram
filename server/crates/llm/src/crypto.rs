//! API key 加密：AES-256-GCM。主密钥来自 env（AGENT_MEMORY_MASTER_KEY）。

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::types::LlmError;

/// 密钥加密器。一次构建，随处 clone（主密钥 32 字节）。
#[derive(Clone)]
pub struct KeyCipher {
    key: [u8; 32],
}

impl std::fmt::Debug for KeyCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyCipher(<redacted>)")
    }
}

impl KeyCipher {
    /// 从 hex 主密钥构建（64 hex 字符 = 32 字节）。
    pub fn from_hex_master(hex: &str) -> Result<Self, LlmError> {
        let bytes = hex_to_bytes(hex)
            .ok_or_else(|| LlmError::NotConfigured("主密钥必须是 64 个 hex 字符".into()))?;
        let key: [u8; 32] = bytes
            .try_into()
            .map_err(|_| LlmError::NotConfigured("主密钥长度错误".into()))?;
        Ok(Self { key })
    }

    /// 加密明文 → nonce(12) || ciphertext（含 GCM tag）。
    pub fn encrypt(&self, plaintext: &str) -> Result<Vec<u8>, LlmError> {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: b"agent-memory-llm-key",
                },
            )
            .map_err(|e| LlmError::Permanent(format!("加密失败: {e}")))?;
        let mut out = nonce_bytes.to_vec();
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// 解密 [`Self::encrypt`] 的产物。
    pub fn decrypt(&self, data: &[u8]) -> Result<String, LlmError> {
        if data.len() < 12 + 16 {
            return Err(LlmError::Permanent("密文格式非法".into()));
        }
        let (nonce_bytes, ct) = data.split_at(12);
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        let pt = cipher
            .decrypt(
                Nonce::from_slice(nonce_bytes),
                Payload {
                    msg: ct,
                    aad: b"agent-memory-llm-key",
                },
            )
            .map_err(|_| LlmError::Permanent("解密失败（主密钥不匹配或密文损坏）".into()))?;
        String::from_utf8(pt).map_err(|_| LlmError::Permanent("解密产物非 UTF-8".into()))
    }
}

/// sha256 hex（API key / 会话 token 哈希共用）。
pub fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let cipher = KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap();
        let secret = "sk-test-12345-长密钥也能加密";
        let enc = cipher.encrypt(secret).unwrap();
        assert_ne!(enc, secret.as_bytes());
        assert!(cipher.decrypt(&enc).unwrap() == secret);
    }

    #[test]
    fn wrong_master_key_fails() {
        let a = KeyCipher::from_hex_master(&"ab".repeat(32)).unwrap();
        let b = KeyCipher::from_hex_master(&"cd".repeat(32)).unwrap();
        let enc = a.encrypt("secret").unwrap();
        assert!(b.decrypt(&enc).is_err());
    }

    #[test]
    fn bad_master_key_rejected() {
        assert!(KeyCipher::from_hex_master("tooshort").is_err());
    }
}
