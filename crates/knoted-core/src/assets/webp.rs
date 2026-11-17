//! Check container and frame dimensions before allocating decoded frames.
//! Pixel decoding remains the pinned codec's responsibility.
use super::service::{DomainError, MAX_PIXELS};

type Dimensions = (u32, u32);

fn bounded((width, height): Dimensions) -> Result<Dimensions, DomainError> {
    if width == 0 || height == 0 {
        return Err(DomainError::InvalidImage);
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(DomainError::Limit);
    }
    Ok((width, height))
}

fn le24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16)
}

fn chunk<'a>(bytes: &'a [u8], offset: &mut usize) -> Result<(&'a [u8], &'a [u8]), DomainError> {
    let header = bytes
        .get(*offset..)
        .and_then(|rest| rest.get(..8))
        .ok_or(DomainError::InvalidImage)?;
    let size = u32::from_le_bytes(header[4..8].try_into().expect("four size bytes")) as usize;
    let start = offset.checked_add(8).ok_or(DomainError::InvalidImage)?;
    let end = start.checked_add(size).ok_or(DomainError::InvalidImage)?;
    let next = end.checked_add(size % 2).ok_or(DomainError::InvalidImage)?;
    if next > bytes.len() {
        return Err(DomainError::InvalidImage);
    }
    *offset = next;
    Ok((&header[..4], &bytes[start..end]))
}

fn encoded(kind: &[u8], data: &[u8]) -> Result<Option<Dimensions>, DomainError> {
    let dimensions = match kind {
        b"VP8 " => {
            if data.len() < 10 || data[0] & 1 != 0 || data[3..6] != [0x9d, 1, 0x2a] {
                return Err(DomainError::InvalidImage);
            }
            (
                u32::from(u16::from_le_bytes([data[6], data[7]]) & 0x3fff),
                u32::from(u16::from_le_bytes([data[8], data[9]]) & 0x3fff),
            )
        }
        b"VP8L" => {
            if data.len() < 5 || data[0] != 0x2f {
                return Err(DomainError::InvalidImage);
            }
            let bits = u32::from_le_bytes(data[1..5].try_into().expect("four dimension bytes"));
            ((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)
        }
        _ => return Ok(None),
    };
    bounded(dimensions).map(Some)
}

fn animation_frame(data: &[u8], canvas: Dimensions) -> Result<(), DomainError> {
    if data.len() < 16 {
        return Err(DomainError::InvalidImage);
    }
    let x = le24(&data[..3]) * 2;
    let y = le24(&data[3..6]) * 2;
    let dimensions = bounded((le24(&data[6..9]) + 1, le24(&data[9..12]) + 1))?;
    if x + dimensions.0 > canvas.0 || y + dimensions.1 > canvas.1 {
        return Err(DomainError::InvalidImage);
    }
    let chunks = &data[16..];
    let mut offset = 0;
    let mut found = false;
    while offset < chunks.len() {
        let (kind, payload) = chunk(chunks, &mut offset)?;
        if matches!(kind, b"ANMF" | b"VP8X") {
            return Err(DomainError::InvalidImage);
        }
        if let Some(inner) = encoded(kind, payload)? {
            if inner != dimensions {
                return Err(DomainError::InvalidImage);
            }
            found = true;
        }
    }
    if !found {
        return Err(DomainError::InvalidImage);
    }
    Ok(())
}

pub(super) fn check_dimensions(bytes: &[u8]) -> Result<(), DomainError> {
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WEBP") {
        return Err(DomainError::InvalidImage);
    }
    let chunks = &bytes[12..];
    let mut offset = 0;
    let mut canvas = None;
    while offset < chunks.len() {
        let (kind, payload) = chunk(chunks, &mut offset)?;
        match kind {
            b"VP8X" => {
                if payload.len() != 10 {
                    return Err(DomainError::InvalidImage);
                }
                canvas = Some(bounded((
                    le24(&payload[4..7]) + 1,
                    le24(&payload[7..10]) + 1,
                ))?);
            }
            b"ANMF" => animation_frame(payload, canvas.ok_or(DomainError::InvalidImage)?)?,
            _ => {
                if let Some(inner) = encoded(kind, payload)?
                    && canvas.is_some_and(|outer| outer != inner)
                {
                    return Err(DomainError::InvalidImage);
                }
            }
        }
    }
    Ok(())
}
