mod crypto;
mod encoding;
mod species;

use crate::team::Pokemon;

// ── HGSS save file layout ─────────────────────────────────────────────────────
//
//   Small block 1:  starts at 0x00000  (size 0xF628, footer at 0xF614)
//   Big   block 1:  starts at 0x0F700
//   Small block 2:  starts at 0x40000  (backup)
//   Big   block 2:  starts at 0x4F700  (backup)
//
// Footer layout (last 0x14 bytes of each block, verified against raw save data):
//   +0x00–0x03  Block connection ID
//   +0x04–0x07  Save number (u32 LE)  ← higher value = most recently written
//   +0x08–0x0B  Block size (u32 LE)
//   +0x0C–0x0F  K
//   +0x10–0x11  T
//   +0x12–0x13  CRC-16-CCITT checksum
//
// Source: projectpokemon.org/home/docs/gen-4/hgss-save-structure-r76/
//         + raw save file inspection.

/// Size of one HGSS small block in bytes (confirmed from the block-size footer field).
const SMALL_BLOCK_SIZE: usize = 0xF628;

/// Offset of the footer within a small block (SMALL_BLOCK_SIZE - 0x14).
const FOOTER_OFFSET: usize = SMALL_BLOCK_SIZE - 0x14;

/// Offset of the save number (u32 LE) within a small block (footer + 0x04).
const SAVE_NUMBER_OFFSET: usize = FOOTER_OFFSET + 0x04;

/// Byte offset of the second small block (backup pair) within the file.
const SMALL_BLOCK_2_START: usize = 0x40000;

// ── Offsets within the active small block ────────────────────────────────────
// Source: projectpokemon.org/home/docs/gen-4/hgss-save-structure-r76/

/// Number of Pokemon currently in the party (u8).
const PARTY_COUNT_OFFSET: usize = 0x94;

/// Start of the party Pokemon array.
const PARTY_OFFSET: usize = 0x98;

/// Size of one party Pokemon slot in bytes (136-byte box data + 100-byte battle stats).
const PARTY_POKEMON_SIZE: usize = 236;

// ── Pokemon data structure (Gen IV, Bulbapedia) ───────────────────────────────
//
//   0x00–0x03  Personality value (PV, u32 LE)
//   0x04–0x05  Sanity / origin flags
//   0x06–0x07  Checksum (u16 LE) — also the XOR seed for decrypting blocks ABCD
//   0x08–0x87  Four shuffled + encrypted 32-byte blocks (ABCD)
//   0x88–0xEB  Battle stats (encrypted with PV as seed, no shuffle; not parsed here)
//
// After decryption and unshuffling:
//   Block A (bytes 0x00–0x1F of canonical blocks):  species ID at byte 0 (u16 LE)
//   Block C (bytes 0x40–0x5F of canonical blocks):  nickname at bytes 0x00–0x15
//                                                    (11 × u16 LE, Gen IV encoding)

/// Read a u16 from `data` at `offset` (little-endian).
fn read_u16_le(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

/// Read a u32 from `data` at `offset` (little-endian).
fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Read the save number (u32 LE) from a slice starting at a small-block boundary.
///
/// Returns `0` if the slice is too short.
fn save_number(block: &[u8]) -> u32 {
    if block.len() > SAVE_NUMBER_OFFSET + 3 {
        read_u32_le(block, SAVE_NUMBER_OFFSET)
    } else {
        0
    }
}

/// Determine the byte offset of the active small block within the full save file.
///
/// The block with the higher save number is the most recently written one.
/// Wrapping arithmetic handles counter roll-over.
fn active_small_block_start(data: &[u8]) -> usize {
    if data.len() >= SMALL_BLOCK_2_START + SMALL_BLOCK_SIZE {
        let num1 = save_number(data);
        let num2 = save_number(&data[SMALL_BLOCK_2_START..]);
        // Wrapping subtraction: if result < 2^31, num2 is ahead of num1.
        if num2.wrapping_sub(num1) < 0x8000_0000 && num2 != num1 {
            return SMALL_BLOCK_2_START;
        }
    }
    0
}

/// Attempt to decode one raw party-slot byte slice into a [`Pokemon`].
///
/// Returns `None` for empty or invalid slots.
fn parse_pokemon_slot(raw: &[u8]) -> Option<Pokemon> {
    if raw.len() < PARTY_POKEMON_SIZE {
        return None;
    }

    let pv = read_u32_le(raw, 0x00);
    let checksum = read_u16_le(raw, 0x06);

    // A completely zeroed slot indicates an empty party position.
    if pv == 0 && checksum == 0 {
        return None;
    }

    // Decrypt and unshuffle the four 32-byte data blocks (bytes 0x08–0x87).
    let encrypted: [u8; 128] = raw[0x08..0x88].try_into().unwrap();
    let decrypted_shuffled = crypto::decrypt_blocks(&encrypted, checksum);
    let blocks = crypto::unshuffle(&decrypted_shuffled, pv);

    // Block A starts at canonical offset 0: species ID is the first u16.
    let species_id = u16::from_le_bytes([blocks[0], blocks[1]]);
    if species_id == 0 {
        return None;
    }

    // Block C starts at canonical offset 64 (0x40): nickname occupies the first 22 bytes.
    let nickname_bytes = &blocks[64..64 + encoding::NICKNAME_MAX_CHARS * 2];
    let nickname = encoding::decode_string(nickname_bytes);

    Some(Pokemon {
        name: species::name_from_id(species_id)
            .unwrap_or("unknown")
            .to_string(),
        nickname: if nickname.is_empty() {
            None
        } else {
            Some(nickname)
        },
    })
}

/// Parse a HeartGold / SoulSilver save file and return the active party as a
/// `Vec<Pokemon>` (up to 6 entries, empty slots omitted).
///
/// # Errors
///
/// Returns a human-readable error string if the file is too small to be a valid save.
pub fn read_party(data: &[u8]) -> Result<Vec<Pokemon>, String> {
    if data.len() < SMALL_BLOCK_SIZE {
        return Err(format!(
            "Save file too small: {} bytes (expected at least {} bytes)",
            data.len(),
            SMALL_BLOCK_SIZE
        ));
    }

    let block_start = active_small_block_start(data);
    let small_block = &data[block_start..block_start + SMALL_BLOCK_SIZE];

    let party_count = (small_block[PARTY_COUNT_OFFSET] as usize).min(6);

    let mut party = Vec::with_capacity(party_count);
    for i in 0..party_count {
        let offset = PARTY_OFFSET + i * PARTY_POKEMON_SIZE;
        if offset + PARTY_POKEMON_SIZE > small_block.len() {
            break;
        }
        if let Some(pokemon) = parse_pokemon_slot(&small_block[offset..offset + PARTY_POKEMON_SIZE])
        {
            party.push(pokemon);
        }
    }

    Ok(party)
}
