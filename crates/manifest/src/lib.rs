//! What a component says about itself, and who vouches for it — ADR-1
//! (`docs/adr/0001-app-identity-and-manifest.md`).
//!
//! Both travel inside the component as custom sections, so an app opened by a
//! bare address carries them as surely as one from the registry:
//!
//! * `nimadir-manifest` — JSON: name, version, the permissions it asks for.
//! * `nimadir-signature` — the publisher's Ed25519 public key and a signature
//!   over every byte before it. It must be the **last** section, which makes
//!   "what was signed" simply "everything up to here".
//!
//! An app's identity is `<publisher>/<name>`, the publisher being derived from
//! the key. Shared by the host, which reads and verifies, and `nimadir-pack`,
//! which writes and signs.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use ed25519_dalek::SigningKey as Key;

pub const MANIFEST_SECTION: &str = "nimadir-manifest";
pub const SIGNATURE_SECTION: &str = "nimadir-signature";

/// What an app declares about itself.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Lower-case letters, digits and `-`; with the publisher, the identity.
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// What the app asks to be allowed, e.g. `["storage", {"network":
    /// ["https://api.example.com"]}]`. Granting is the shell's business.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<Permission>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Permission {
    Named(String),
    Network { network: Vec<String> },
}

impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        let ok = !self.name.is_empty()
            && self.name.len() <= 64
            && self
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !ok {
            return Err(format!(
                "manifest name {:?} must be 1–64 of a-z, 0-9 and -",
                self.name
            ));
        }
        if self.version.is_empty() {
            return Err("manifest has no version".into());
        }
        Ok(())
    }

    /// The origins listed under `network`, all of them.
    pub fn network(&self) -> Vec<String> {
        self.permissions
            .iter()
            .filter_map(|p| match p {
                Permission::Network { network } => Some(network.iter().cloned()),
                Permission::Named(_) => None,
            })
            .flatten()
            .collect()
    }

    pub fn wants(&self, permission: &str) -> bool {
        self.permissions
            .iter()
            .any(|p| matches!(p, Permission::Named(n) if n == permission))
    }
}

/// Who vouches for a component.
#[derive(Clone, Debug, PartialEq)]
pub enum SignedBy {
    /// No signature section: it runs, as *unverified* (ADR-1).
    Nobody,
    /// Signed, and the signature holds.
    Publisher(String),
}

/// What `inspect` found.
#[derive(Clone, Debug)]
pub struct Inspected {
    pub manifest: Option<Manifest>,
    pub signed_by: SignedBy,
}

impl Inspected {
    /// `<publisher>/<name>` — only for a signed component with a manifest. An
    /// unsigned one has no identity anything persistent can be keyed by.
    pub fn identity(&self) -> Option<String> {
        match (&self.signed_by, &self.manifest) {
            (SignedBy::Publisher(p), Some(m)) => Some(format!("{p}/{}", m.name)),
            _ => None,
        }
    }
}

/// Read a component's manifest and check its signature.
///
/// `Err` is a component that must not run: a malformed manifest, a signature
/// section that is not last, or a signature that does not hold — the bytes
/// were changed after signing, or it claims a key it was not signed with.
pub fn inspect(bytes: &[u8]) -> Result<Inspected, String> {
    let sections = sections(bytes)?;

    let mut manifest = None;
    let mut signature = None;
    for (i, s) in sections.iter().enumerate() {
        match s.custom_name {
            Some(MANIFEST_SECTION) => {
                let m: Manifest = serde_json::from_slice(&bytes[s.payload.clone()])
                    .map_err(|e| format!("the app's manifest is not valid: {e}"))?;
                m.validate()?;
                manifest = Some(m);
            }
            Some(SIGNATURE_SECTION) => {
                if i + 1 != sections.len() {
                    return Err("the app's signature is not its last section".into());
                }
                signature = Some(s);
            }
            _ => {}
        }
    }

    let signed_by = match signature {
        None => SignedBy::Nobody,
        Some(s) => {
            let payload = &bytes[s.payload.clone()];
            if payload.len() != 32 + 64 {
                return Err("the app's signature section is malformed".into());
            }
            let key = VerifyingKey::from_bytes(payload[..32].try_into().expect("32 bytes"))
                .map_err(|_| "the app's signing key is not a valid key".to_string())?;
            let sig = Signature::from_bytes(payload[32..].try_into().expect("64 bytes"));
            key.verify(&bytes[..s.start], &sig).map_err(|_| {
                "the app's signature does not match its contents — it was changed \
                 after it was signed"
                    .to_string()
            })?;
            SignedBy::Publisher(publisher_id(key.as_bytes()))
        }
    };
    Ok(Inspected {
        manifest,
        signed_by,
    })
}

/// The publisher a key stands for: the first 16 bytes of its SHA-256, as
/// lower-case base32 — 26 characters, safe in a file name.
pub fn publisher_id(key: &[u8; 32]) -> String {
    base32(&Sha256::digest(key)[..16])
}

/// The component with its manifest replaced by `manifest` — and any old
/// manifest or signature dropped, since a signature over the old bytes would
/// no longer hold.
pub fn with_manifest(bytes: &[u8], manifest: &Manifest) -> Result<Vec<u8>, String> {
    manifest.validate()?;
    let mut out = bytes[..8].to_vec();
    for s in sections(bytes)? {
        if !matches!(s.custom_name, Some(MANIFEST_SECTION | SIGNATURE_SECTION)) {
            out.extend_from_slice(&bytes[s.start..s.end]);
        }
    }
    let json = serde_json::to_vec(manifest).map_err(|e| e.to_string())?;
    append_custom(&mut out, MANIFEST_SECTION, &json);
    Ok(out)
}

/// Sign a component: its bytes, then a final section holding the public key
/// and the signature over everything before it.
pub fn sign(bytes: &[u8], key: &SigningKey) -> Result<Vec<u8>, String> {
    if sections(bytes)?
        .iter()
        .any(|s| s.custom_name == Some(SIGNATURE_SECTION))
    {
        return Err("the component is already signed".into());
    }
    let signature = key.sign(bytes);
    let mut payload = key.verifying_key().to_bytes().to_vec();
    payload.extend_from_slice(&signature.to_bytes());
    let mut out = bytes.to_vec();
    append_custom(&mut out, SIGNATURE_SECTION, &payload);
    Ok(out)
}

/// One top-level section: where it is, and its name when it is custom.
struct Section<'a> {
    start: usize,
    end: usize,
    custom_name: Option<&'a str>,
    payload: std::ops::Range<usize>,
}

/// The top-level sections of a module or component. Only the outer layer:
/// a nested module is one section here, skipped by its size, so nothing about
/// what is inside it matters.
fn sections(bytes: &[u8]) -> Result<Vec<Section<'_>>, String> {
    if bytes.len() < 8 || &bytes[..4] != b"\0asm" {
        return Err("not a WebAssembly component".into());
    }
    let mut out = Vec::new();
    let mut at = 8;
    while at < bytes.len() {
        let start = at;
        let id = bytes[at];
        at += 1;
        let size = leb128(bytes, &mut at)? as usize;
        let end = at.checked_add(size).filter(|&e| e <= bytes.len());
        let end = end.ok_or("a section runs past the end of the component")?;
        let (custom_name, payload) = if id == 0 {
            let mut p = at;
            let len = leb128(bytes, &mut p)? as usize;
            let name_end = p.checked_add(len).filter(|&e| e <= end);
            let name_end = name_end.ok_or("a custom section's name is cut short")?;
            let name = std::str::from_utf8(&bytes[p..name_end]).ok();
            (name, name_end..end)
        } else {
            (None, at..end)
        };
        out.push(Section {
            start,
            end,
            custom_name,
            payload,
        });
        at = end;
    }
    Ok(out)
}

fn leb128(bytes: &[u8], at: &mut usize) -> Result<u32, String> {
    let mut value: u32 = 0;
    for shift in (0..35).step_by(7) {
        let byte = *bytes.get(*at).ok_or("the component ends inside a number")?;
        *at += 1;
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("a number in the component is too long".into())
}

fn append_custom(out: &mut Vec<u8>, name: &str, data: &[u8]) {
    let mut body = Vec::new();
    write_leb128(&mut body, name.len() as u32);
    body.extend_from_slice(name.as_bytes());
    body.extend_from_slice(data);
    out.push(0);
    write_leb128(out, body.len() as u32);
    out.extend_from_slice(&body);
}

fn write_leb128(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0);
    for &b in bytes {
        buffer = (buffer << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}
