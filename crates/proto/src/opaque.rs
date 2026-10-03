//! The OPAQUE cipher suite: ristretto255 with 3DH, hardened by Argon2id.

use ::argon2::{Algorithm, Argon2, Params, Version};
use opaque_ke::{CipherSuite, Ristretto255, TripleDh};

/// 64 MiB, 3 passes, one lane. Stored with the bundle so it can be raised.
pub const ARGON_MEMORY_KIB: u32 = 64 * 1024;
pub const ARGON_PASSES: u32 = 3;
pub const ARGON_LANES: u32 = 1;

pub struct Suite;

impl CipherSuite for Suite {
    type OprfCs = Ristretto255;
    type KeyExchange = TripleDh<Ristretto255, sha2::Sha512>;
    type Ksf = Argon2<'static>;
}

/// Argon2id with the given cost.
pub fn argon2_with(memory_kib: u32, passes: u32, lanes: u32) -> Argon2<'static> {
    let params = Params::new(memory_kib, passes, lanes, None).expect("valid argon2 parameters");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Argon2id at the cost this release uses.
pub fn argon2() -> Argon2<'static> {
    argon2_with(ARGON_MEMORY_KIB, ARGON_PASSES, ARGON_LANES)
}
