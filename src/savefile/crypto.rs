/// Gen IV PRNG (Linear Congruential Generator) used for Pokemon data encryption.
///
/// Formula: `X[n+1] = 0x41C64E6D * X[n] + 0x6073`
pub fn lcg_next(state: u32) -> u32 {
    state.wrapping_mul(0x41C64E6D).wrapping_add(0x6073)
}

/// Decrypt the 128-byte ABCD section of a Gen IV Pokemon using the checksum as the LCG seed.
///
/// The checksum (stored at offset `0x06` in the raw Pokemon bytes) seeds the LCG.  Each
/// successive 16-bit LCG output XORs one 16-bit word of the encrypted data.
///
/// Returns the decrypted bytes in the same (still-shuffled) order they arrived.
pub fn decrypt_blocks(data: &[u8; 128], seed: u16) -> [u8; 128] {
    let mut out = [0u8; 128];
    let mut state = seed as u32;
    for i in 0..64 {
        state = lcg_next(state);
        let key = (state >> 16) as u16;
        let word = u16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
        let decrypted = word ^ key;
        out[i * 2] = decrypted as u8;
        out[i * 2 + 1] = (decrypted >> 8) as u8;
    }
    out
}

/// Block shuffle order table (Bulbapedia – *Block Order*).
///
/// `SHUFFLE_TABLE[shift][slot]` is the canonical block index (A=0, B=1, C=2, D=3)
/// that is stored in encrypted `slot` for a given `shift` value.
///
/// The shift is derived from the personality value: `shift = ((PV & 0x3E000) >> 13) % 24`.
const SHUFFLE_TABLE: [[usize; 4]; 24] = [
    [0, 1, 2, 3], // 0:  ABCD
    [0, 1, 3, 2], // 1:  ABDC
    [0, 2, 1, 3], // 2:  ACBD
    [0, 2, 3, 1], // 3:  ACDB
    [0, 3, 1, 2], // 4:  ADBC
    [0, 3, 2, 1], // 5:  ADCB
    [1, 0, 2, 3], // 6:  BACD
    [1, 0, 3, 2], // 7:  BADC
    [1, 2, 0, 3], // 8:  BCAD
    [1, 2, 3, 0], // 9:  BCDA
    [1, 3, 0, 2], // 10: BDAC
    [1, 3, 2, 0], // 11: BDCA
    [2, 0, 1, 3], // 12: CABD
    [2, 0, 3, 1], // 13: CADB
    [2, 1, 0, 3], // 14: CBAD
    [2, 1, 3, 0], // 15: CBDA
    [2, 3, 0, 1], // 16: CDAB
    [2, 3, 1, 0], // 17: CDBA
    [3, 0, 1, 2], // 18: DABC
    [3, 0, 2, 1], // 19: DACB
    [3, 1, 0, 2], // 20: DBAC
    [3, 1, 2, 0], // 21: DBCA
    [3, 2, 0, 1], // 22: DCAB
    [3, 2, 1, 0], // 23: DCBA
];

/// Unshuffle 128 decrypted bytes into canonical [A, B, C, D] block order.
///
/// Each block is 32 bytes.  The shuffle index is encoded in the personality value:
/// `shift = ((PV & 0x3E000) >> 13) % 24`.
pub fn unshuffle(data: &[u8; 128], pv: u32) -> [u8; 128] {
    let shift = ((pv & 0x3E000) >> 0xD) % 24;
    let order = &SHUFFLE_TABLE[shift as usize];
    // order[slot] = which canonical block is stored in that slot.
    // Copy each slot's 32 bytes into the canonical destination.
    let mut out = [0u8; 128];
    for slot in 0..4 {
        let block_idx = order[slot];
        let src = slot * 32;
        let dst = block_idx * 32;
        out[dst..dst + 32].copy_from_slice(&data[src..src + 32]);
    }
    out
}
