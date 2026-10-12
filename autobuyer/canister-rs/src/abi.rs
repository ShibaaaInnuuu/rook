//! Just enough of the Solidity ABI to build and read the calls `policy.rs` and `bridgecall.rs` make: static words, one level of
//! dynamic `bytes`/`string`, and a returned struct holding strings. Pure; every encoding is checked against Foundry's `cast` in
//! the modules that use it.

pub(crate) type Word = [u8; 32];

pub(crate) fn uint_word(v: u128) -> Word {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&v.to_be_bytes());
    w
}

pub(crate) fn addr_word(a: &[u8; 20]) -> Word {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}

/// Word `i` of `b`, where word 0 starts at `base`.
pub(crate) fn word_at(b: &[u8], base: usize, i: usize) -> Result<&[u8], String> {
    let s = base.checked_add(i.checked_mul(32).ok_or("ABI offset overflows")?).ok_or("ABI offset overflows")?;
    b.get(s..s + 32).ok_or_else(|| format!("ABI data ends before word {i}"))
}

/// An unsigned integer that must fit in `bits` (≤ 128): anything above is refused, never truncated.
pub(crate) fn uint_at(b: &[u8], base: usize, i: usize, bits: u32) -> Result<u128, String> {
    let w = word_at(b, base, i)?;
    if w[..16].iter().any(|x| *x != 0) {
        return Err(format!("word {i} is wider than 128 bits"));
    }
    let v = u128::from_be_bytes(w[16..].try_into().expect("16 bytes"));
    if bits < 128 && v >> bits != 0 {
        return Err(format!("word {i} is wider than {bits} bits"));
    }
    Ok(v)
}

pub(crate) fn bool_at(b: &[u8], base: usize, i: usize) -> Result<bool, String> {
    match uint_at(b, base, i, 8)? {
        0 => Ok(false),
        1 => Ok(true),
        v => Err(format!("word {i} is {v}, not a bool")),
    }
}

pub(crate) fn addr_at(b: &[u8], base: usize, i: usize) -> Result<[u8; 20], String> {
    let w = word_at(b, base, i)?;
    if w[..12].iter().any(|x| *x != 0) {
        return Err(format!("word {i} is not an address (high bytes set)"));
    }
    Ok(w[12..].try_into().expect("20 bytes"))
}

/// A dynamic `bytes`/`string` whose head word `i` (relative to `base`) holds its offset, also relative to `base`.
pub(crate) fn bytes_at(b: &[u8], base: usize, i: usize) -> Result<Vec<u8>, String> {
    let off = uint_at(b, base, i, 32)? as usize;
    let start = base.checked_add(off).ok_or("ABI offset overflows")?;
    let len = uint_at(b, start, 0, 32)? as usize;
    let data = start + 32;
    b.get(data..data.checked_add(len).ok_or("ABI length overflows")?).map(|x| x.to_vec())
        .ok_or_else(|| format!("dynamic value at word {i} runs past the end"))
}

/// `sel ‖ head ‖ tail` for a call whose arguments are static words and dynamic byte strings, in order.
pub(crate) enum Arg<'a> {
    Word(Word),
    Bytes(&'a [u8]),
}

pub(crate) fn encode_call(sel: [u8; 4], args: &[Arg]) -> Vec<u8> {
    let mut head = Vec::new();
    let mut tail = Vec::new();
    let head_len = 32 * args.len();
    for a in args {
        match a {
            Arg::Word(w) => head.extend_from_slice(w),
            Arg::Bytes(b) => {
                head.extend_from_slice(&uint_word((head_len + tail.len()) as u128));
                tail.extend_from_slice(&uint_word(b.len() as u128));
                tail.extend_from_slice(b);
                tail.resize(tail.len() + (32 - b.len() % 32) % 32, 0);
            }
        }
    }
    let mut out = sel.to_vec();
    out.extend(head);
    out.extend(tail);
    out
}

pub(crate) fn selector_of(data: &[u8]) -> Option<[u8; 4]> {
    data.get(..4).map(|s| s.try_into().expect("4 bytes"))
}
