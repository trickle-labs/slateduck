//! SCRAM-SHA-256 server-side authentication (RFC 5802).
//!
//! Implements the server half of the SCRAM-SHA-256 exchange carried over the
//! PostgreSQL wire protocol.  The entry points are:
//!
//! 1. [`ScramState::from_client_first`] — parse the client-first-message,
//!    generate the server nonce/salt/iteration-count, and return the state
//!    object plus the server-first-message bytes to send back.
//! 2. [`ScramState::validate_client_final`] — verify the client proof and
//!    return the server-signature bytes (for `AuthenticationSASLFinal`) on
//!    success, or `None` on failure.
//!
//! Crypto primitives (`hi_sha256`, `hmac_sha256`, `sha256`, `ct_bytes_eq`)
//! are exposed as `pub` so they can be independently unit-tested.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

// ── Server-side SCRAM state ────────────────────────────────────────────────

/// Per-connection SCRAM-SHA-256 server state.
///
/// Created after the client-first-message is received, consumed once the
/// client-final-message arrives.
pub struct ScramState {
    /// The combined nonce (`client-nonce` + server suffix).
    pub nonce: String,
    /// The full server-first-message (sent as `AuthenticationSASLContinue`).
    pub server_first: String,
    /// The client-first-message-bare (stripped of the GS2 header).
    pub client_first_bare: String,
    /// SaltedPassword = Hi(password, salt, iterations).
    pub salted_password: [u8; 32],
}

impl ScramState {
    /// Parse the `client-first-message`, generate server randomness and
    /// derive the salted password.
    ///
    /// Returns `None` if the message is malformed or uses an unsupported GS2
    /// header (anything other than the `n,,` / `y,,` no-binding variants).
    pub fn from_client_first(
        client_first: &[u8],
        password: &str,
        server_nonce_suffix: &str,
    ) -> Option<Self> {
        let msg = std::str::from_utf8(client_first).ok()?;

        // Strip the GS2 header.  Accept `n,,` (no channel binding advertised)
        // and `y,,` (no channel binding used even if supported).
        let client_first_bare = if msg.starts_with("n,,") || msg.starts_with("y,,") {
            msg.get(3..)?
        } else {
            // Unknown / unsupported GS2 header.
            return None;
        };

        // Extract the client nonce from the bare message.
        let client_nonce = client_first_bare
            .split(',')
            .find_map(|part| part.strip_prefix("r="))?;

        // Server-side random salt (16 bytes from a cryptographically random UUID).
        let salt: Vec<u8> = uuid::Uuid::new_v4().as_bytes().to_vec();
        let salt_b64 = B64.encode(&salt);
        let iterations: u32 = 4096;

        let nonce = format!("{client_nonce}{server_nonce_suffix}");
        let server_first = format!("r={nonce},s={salt_b64},i={iterations}");
        let salted_password = hi_sha256(password.as_bytes(), &salt, iterations);

        Some(Self {
            nonce,
            server_first,
            client_first_bare: client_first_bare.to_string(),
            salted_password,
        })
    }

    /// Validate the `client-final-message` received from the client.
    ///
    /// On success returns the server-final-message bytes (the base64-encoded
    /// `ServerSignature` prefixed with `v=`).  Returns `None` if the proof
    /// is wrong or the message is malformed.
    pub fn validate_client_final(&self, client_final: &[u8]) -> Option<Vec<u8>> {
        let msg = std::str::from_utf8(client_final).ok()?;

        // Split at the last `,p=` to obtain client-final-without-proof.
        let proof_pos = msg.rfind(",p=")?;
        let client_final_without_proof = &msg[..proof_pos];
        let proof_b64 = msg.get(proof_pos + 3..)?;

        // The nonce in the final message must match what the server sent.
        let nonce_ok = client_final_without_proof
            .split(',')
            .any(|p| p == format!("r={}", self.nonce));
        if !nonce_ok {
            return None;
        }

        // Decode the client proof (ClientKey XOR ClientSignature).
        let client_proof = B64.decode(proof_b64).ok()?;
        if client_proof.len() != 32 {
            return None;
        }

        // Compute StoredKey = H(ClientKey).
        let client_key = hmac_sha256(&self.salted_password, b"Client Key");
        let stored_key = sha256(&client_key);

        // AuthMessage = client-first-bare + "," + server-first + "," + client-final-without-proof.
        let auth_message = format!(
            "{},{},{}",
            self.client_first_bare, self.server_first, client_final_without_proof,
        );

        // ClientSignature = HMAC(StoredKey, AuthMessage).
        let client_signature = hmac_sha256(&stored_key, auth_message.as_bytes());

        // Recover the original ClientKey from the proof.
        let mut recovered_key = [0u8; 32];
        for (i, (&p, &s)) in client_proof.iter().zip(client_signature.iter()).enumerate() {
            recovered_key[i] = p ^ s;
        }

        // Verify: H(recovered_key) == StoredKey.
        let recovered_stored = sha256(&recovered_key);
        if !ct_bytes_eq(&recovered_stored, &stored_key) {
            return None;
        }

        // Compute ServerSignature = HMAC(ServerKey, AuthMessage).
        let server_key = hmac_sha256(&self.salted_password, b"Server Key");
        let server_sig = hmac_sha256(&server_key, auth_message.as_bytes());
        let server_final = format!("v={}", B64.encode(server_sig));

        Some(server_final.into_bytes())
    }
}

// ── Crypto primitives ──────────────────────────────────────────────────────

/// PBKDF2-HMAC-SHA256 (`Hi` in RFC 5802 notation).
///
/// Derives a 32-byte key from `password`, `salt`, and an iteration count.
pub fn hi_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    // U_1 = HMAC(password, salt || 0x00000001)
    let u1 = {
        let mut mac = HmacSha256::new_from_slice(password).expect("HMAC accepts any key size");
        mac.update(salt);
        mac.update(&[0u8, 0, 0, 1]);
        let out = mac.finalize().into_bytes();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(out.as_slice());
        arr
    };

    let mut result = u1;
    let mut prev = u1;

    for _ in 1..iterations {
        let mut mac = HmacSha256::new_from_slice(password).expect("HMAC accepts any key size");
        mac.update(&prev);
        let out = mac.finalize().into_bytes();
        let mut next = [0u8; 32];
        next.copy_from_slice(out.as_slice());
        for (r, n) in result.iter_mut().zip(next.iter()) {
            *r ^= n;
        }
        prev = next;
    }

    result
}

/// HMAC-SHA256.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(data);
    let out = mac.finalize().into_bytes();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(out.as_slice());
    arr
}

/// SHA-256 hash.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(out.as_slice());
    arr
}

/// Constant-time byte-slice equality (same length and same bytes).
///
/// Evaluates every byte pair regardless of where the first mismatch occurs,
/// eliminating the data-dependent timing that short-circuit equality creates.
pub fn ct_bytes_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Generate a cryptographically random 18-character server nonce suffix.
///
/// Derives randomness from a version-4 UUID (backed by the OS CSPRNG via
/// `getrandom`), so no separate `rand` dependency is required.
pub fn random_server_nonce() -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    // Take first 18 chars of the 32-char hex UUID string.
    id[..18].to_string()
}
