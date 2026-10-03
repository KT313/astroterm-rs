//! Explicit wire encodings for Rust enums and optional values; no enum discriminants or padding are persisted.
use crate::catalog::Designation;
pub(crate) fn encode_designation(value: Option<Designation>) -> [u8; 16] {
    let mut bytes = [0; 16];
    match value {
        None => {}
        Some(Designation::Bayer {
            letter,
            component,
            constellation,
        }) => {
            bytes[0] = 1;
            bytes[1] = letter;
            bytes[2] = component;
            bytes[3..6].copy_from_slice(&constellation);
        }
        Some(Designation::Flamsteed { number, constellation }) => {
            bytes[0] = 2;
            bytes[1..3].copy_from_slice(&number.to_le_bytes());
            bytes[3..6].copy_from_slice(&constellation);
        }
        Some(Designation::Hr(n)) => {
            bytes[0] = 3;
            bytes[1..5].copy_from_slice(&n.to_le_bytes());
        }
        Some(Designation::Hip(n)) => {
            bytes[0] = 4;
            bytes[1..5].copy_from_slice(&n.to_le_bytes());
        }
        Some(Designation::Tycho {
            region,
            number,
            component,
        }) => {
            bytes[0] = 5;
            bytes[1..3].copy_from_slice(&region.to_le_bytes());
            bytes[3..5].copy_from_slice(&number.to_le_bytes());
            bytes[5] = component;
        }
        Some(Designation::Gaia(n)) => {
            bytes[0] = 6;
            bytes[1..9].copy_from_slice(&n.to_le_bytes());
        }
    }
    bytes
}
pub(crate) fn decode_designation(bytes: [u8; 16]) -> Option<Option<Designation>> {
    let con: [u8; 3] = bytes[3..6].try_into().ok()?;
    let value = match bytes[0] {
        0 => None,
        1 if bytes[1] < 24 => Some(Designation::Bayer {
            letter: bytes[1],
            component: bytes[2],
            constellation: con,
        }),
        2 => Some(Designation::Flamsteed {
            number: u16::from_le_bytes(bytes[1..3].try_into().ok()?),
            constellation: con,
        }),
        3 => Some(Designation::Hr(u32::from_le_bytes(bytes[1..5].try_into().ok()?))),
        4 => Some(Designation::Hip(u32::from_le_bytes(bytes[1..5].try_into().ok()?))),
        5 => Some(Designation::Tycho {
            region: u16::from_le_bytes(bytes[1..3].try_into().ok()?),
            number: u16::from_le_bytes(bytes[3..5].try_into().ok()?),
            component: bytes[5],
        }),
        6 => Some(Designation::Gaia(u64::from_le_bytes(bytes[1..9].try_into().ok()?))),
        _ => return None,
    };
    (encode_designation(value) == bytes).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constellation_bytes_preserve_the_existing_loader_contract() {
        for constellation in [*b"Ori", *b"123", *"星".as_bytes().first_chunk::<3>().unwrap()] {
            let value = Some(Designation::Bayer {
                letter: 0,
                component: 1,
                constellation,
            });
            assert_eq!(decode_designation(encode_designation(value)), Some(value));
        }
    }
}
