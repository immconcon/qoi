#![allow(incomplete_features)]
#![feature(explicit_tail_calls)]

use std::num::TryFromIntError;

use nom::{
    Parser,
    branch::alt,
    bytes::complete::{tag, take},
    combinator::{value, verify},
    number::{self, Endianness},
};
use thiserror::Error;

const ONE: usize = 1;

type Result<'a, O> = core::result::Result<(&'a [u8], O), Failure<nom::error::Error<&'a [u8]>>>;

type Pixel3 = [u8; 3];
type Pixel4 = [u8; 4];

#[derive(Error, Debug)]
pub enum Failure<E> {
    #[error("nom parsing failed: {0}")]
    Nom(#[from] nom::Err<E>),
    #[error("QOI header provided wrong pixel width, pixel height or channels")]
    HeaderLiedAboutSize,
    #[error("byte mask input did not match any known pattern")]
    BadPattern,
    #[error("image is bigger than the limit of {0} bytes")]
    ImageIsTooBig(u32),
    #[error("Are you on 16-bit machine? lol. Anyway, image is too big: {0}")]
    UsizeIsLessThanU32(#[from] TryFromIntError),
}

pub struct Qoi {
    pub header: Header,
    pub data: Box<[u8]>,
}

/// Decode a QOI image from raw bytes.
///
/// # Errors
///
/// - [`Failure::Nom`]: malformed or truncated input.
/// - [`Failure::ImageIsTooBig`]: decoded size exceeds `byte_limit`.
/// - [`Failure::HeaderLiedAboutSize`]: header dimensions don't match encoded data.
/// - [`Failure::UsizeIsLessThanU32`]: decoded size overflows `usize` (16-bit targets).
/// - [`Failure::BadPattern`]: unrecognized opcode in stream.
pub fn decode(input: &[u8], byte_limit: Option<u32>) -> Result<'_, Qoi> {
    let (input, header) = header(input)?;
    let img_bytes = header
        .width
        .checked_mul(header.height)
        .and_then(|len| len.checked_mul(header.channel as u32))
        .ok_or_else(|| Failure::ImageIsTooBig(byte_limit.unwrap_or(u32::MAX)))?;

    let mut buffer: Box<[u8]> = vec![0; usize::try_from(img_bytes)?].into();
    let (input, ()) = match header.channel {
        Channels::Rgba => {
            let mut pixel_map: [Pixel4; 64] = [[0; 4]; 64];
            let mut pixel: Pixel4 = [0, 0, 0, 255];
            loop_chomp_4chan(input, &mut pixel_map, &mut pixel, &mut buffer)
        }
        Channels::Rgb => {
            let mut pixel_map: [Pixel3; 64] = [[0; 3]; 64];
            let mut pixel: Pixel3 = [0, 0, 0];
            loop_chomp_3chan(input, &mut pixel_map, &mut pixel, &mut buffer)
        }
    }?;

    Ok((
        input,
        Qoi {
            header,
            data: buffer,
        },
    ))
}

/// Parse 4-channel RBGA image into `buffer`.
fn loop_chomp_4chan<'a>(
    mut input: &'a [u8],
    pixel_map: &mut [Pixel4; 64],
    pixel: &mut Pixel4,
    buffer: &mut [u8],
) -> Result<'a, ()> {
    const CHANNELS: usize = 4;
    // because QOI must be ending with 8 padding bytes,
    // we can just chomp 8 bytes and match on slices.
    let (after_img, chomp) = take(8_usize)(input)?;
    match chomp {
        // QOI end marker of 8 bytes.
        [0, 0, 0, 0, 0, 0, 0, 1] => return Ok((after_img, ())),
        &[0b1111_1111, r, g, b, a, ..] => {
            *pixel = [r, g, b, a];
            input = &input[5..];
        }
        &[0b1111_1110, r, g, b, ..] => {
            *pixel = [r, g, b, pixel[3]];
            input = &input[4..];
        }
        &[x, ..] if high2(x) == 0b11 => {
            // `run` is stored with bias -1;
            // i.e. when in reality it is 1..=62 stored value is 0..62.
            let bytes_run = (low6(x) as usize + 1) * CHANNELS;
            let (left, right) = buffer
                .split_at_mut_checked(bytes_run)
                .ok_or(Failure::HeaderLiedAboutSize)?;
            left.as_chunks_mut()
                .0
                .iter_mut()
                .for_each(|chunk| *chunk = *pixel);
            become loop_chomp_4chan(&input[1..], pixel_map, pixel, right)
        }
        &[x, y, ..] if high2(x) == 0b10 => {
            let green_diff = low6(x).wrapping_sub(32);
            // green
            pixel[1] = pixel[1].wrapping_add(green_diff);
            // red
            pixel[0] = pixel[0]
                .wrapping_add(green_diff)
                .wrapping_sub(8)
                .wrapping_add(high4(y));
            // blue
            pixel[2] = pixel[2]
                .wrapping_add(green_diff)
                .wrapping_sub(8)
                .wrapping_add(low4(y));
            input = &input[2..];
        }
        &[x, ..] if high2(x) == 0b01 => {
            let (red_diff, green_diff, blue_diff) = diff_low6(x);
            pixel[0] = pixel[0].wrapping_add(red_diff);
            pixel[1] = pixel[1].wrapping_add(green_diff);
            pixel[2] = pixel[2].wrapping_add(blue_diff);
            input = &input[1..];
        }
        &[x, ..] if high2(x) == 0b00 => {
            *pixel = pixel_map[x as usize % 64];
            input = &input[1..];
        }
        _ => return Err(Failure::BadPattern),
    }

    // Write pixel into map.
    pixel_map[hash4(*pixel)] = *pixel;
    // Write pixel into buffer.
    match buffer.split_first_chunk_mut() {
        None => Err(Failure::HeaderLiedAboutSize),
        Some((chunk, buffer_rest)) => {
            *chunk = *pixel;
            become loop_chomp_4chan(input, pixel_map, pixel, buffer_rest)
        }
    }
}

/// Parse 3-channel RBG image into `buffer`.
fn loop_chomp_3chan<'a>(
    mut input: &'a [u8],
    pixel_map: &mut [Pixel3; 64],
    pixel: &mut Pixel3,
    buffer: &mut [u8],
) -> Result<'a, ()> {
    const CHANNELS: usize = 3;
    // because QOI must be ending with 8 padding bytes,
    // we can just chomp 8 bytes and match on slices.
    let (after_img, chomp) = take(8_usize)(input)?;
    match chomp {
        // QOI end marker of 8 bytes.
        [0, 0, 0, 0, 0, 0, 0, 1] => return Ok((after_img, ())),
        &[0b1111_1111, r, g, b, _a, ..] => {
            *pixel = [r, g, b];
            input = &input[5..];
        }
        &[0b1111_1110, r, g, b, ..] => {
            *pixel = [r, g, b];
            input = &input[4..];
        }
        &[x, ..] if high2(x) == 0b11 => {
            // `run` is stored with bias -1;
            // i.e. when in reality it is 1..=62 stored value is 0..62.
            let bytes_run = (low6(x) as usize + 1) * CHANNELS;
            let (left, right) = buffer
                .split_at_mut_checked(bytes_run)
                .ok_or(Failure::HeaderLiedAboutSize)?;
            left.as_chunks_mut()
                .0
                .iter_mut()
                .for_each(|chunk| *chunk = *pixel);
            become loop_chomp_3chan(&input[1..], pixel_map, pixel, right)
        }
        &[x, y, ..] if high2(x) == 0b10 => {
            let green_diff = low6(x).wrapping_sub(32);
            // green
            pixel[1] = pixel[1].wrapping_add(green_diff);
            // red
            pixel[0] = pixel[0]
                .wrapping_add(green_diff)
                .wrapping_sub(8)
                .wrapping_add(high4(y));
            // blue
            pixel[2] = pixel[2]
                .wrapping_add(green_diff)
                .wrapping_sub(8)
                .wrapping_add(low4(y));
            input = &input[2..];
        }
        &[x, ..] if high2(x) == 0b01 => {
            let (red_diff, green_diff, blue_diff) = diff_low6(x);
            pixel[0] = pixel[0].wrapping_add(red_diff);
            pixel[1] = pixel[1].wrapping_add(green_diff);
            pixel[2] = pixel[2].wrapping_add(blue_diff);
            input = &input[1..];
        }
        &[x, ..] if high2(x) == 0b00 => {
            *pixel = pixel_map[x as usize % 64];
            input = &input[1..];
        }
        _ => return Err(Failure::BadPattern),
    }

    // Write pixel into map.
    pixel_map[hash3(*pixel)] = *pixel;
    // Write pixel into buffer.
    match buffer.split_first_chunk_mut() {
        None => Err(Failure::HeaderLiedAboutSize),
        Some((chunk, buffer_rest)) => {
            *chunk = *pixel;
            become loop_chomp_3chan(input, pixel_map, pixel, buffer_rest)
        }
    }
}

const fn high2(x: u8) -> u8 {
    x >> 6
}
const fn low6(x: u8) -> u8 {
    x & 0b0011_1111
}
const fn high4(x: u8) -> u8 {
    x >> 4
}
const fn low4(x: u8) -> u8 {
    x & 0b1111
}
/// Returns `(red_diff, green_diff, blue_diff)` with +2 bias already accounted for.
const fn diff_low6(x: u8) -> (u8, u8, u8) {
    let x = low6(x);
    (
        (x >> 4).wrapping_sub(2),
        ((x >> 2) & 0b11).wrapping_sub(2),
        (x & 0b11).wrapping_sub(2),
    )
}

/// Parse QOI header.
fn header(input: &[u8]) -> Result<'_, Header> {
    let channel = alt((
        value(Channels::Rgb, verify(take(ONE), |s: &[u8]| s == [3])),
        value(Channels::Rgba, verify(take(ONE), |s: &[u8]| s == [4])),
    ));
    let colorspace = alt((
        value(
            ColorSpace::SRGBWithLinearAlpha,
            verify(take(ONE), |s: &[u8]| s == [0]),
        ),
        value(
            ColorSpace::AllChannelLinear,
            verify(take(ONE), |s: &[u8]| s == [1]),
        ),
    ));

    let (rest, (_magic, width, height, channel, colorspace)) = (
        tag(b"qoif".as_slice()),
        number::u32(Endianness::Big),
        number::u32(Endianness::Big),
        channel,
        colorspace,
    )
        .parse(input)?;

    Ok((
        rest,
        Header {
            width,
            height,
            channel,
            colorspace,
        },
    ))
}

#[repr(C)]
pub struct Header {
    pub width: u32,
    pub height: u32,
    pub channel: Channels,
    pub colorspace: ColorSpace,
}

#[derive(Copy, Clone)]
#[repr(u8)]
pub enum Channels {
    Rgb = 3,
    Rgba = 4,
}

#[derive(Copy, Clone)]
#[repr(u8)]
pub enum ColorSpace {
    SRGBWithLinearAlpha = 0,
    AllChannelLinear = 1,
}

#[inline]
const fn hash3(p: Pixel3) -> usize {
    // Alpha is always 255 for RGB images (never encoded in pixels).
    const ALPHA_HASH: u8 = 255_u8.wrapping_mul(11) % 64; // 53
    ((ALPHA_HASH
        .wrapping_add(p[2].wrapping_mul(7))
        .wrapping_add(p[1].wrapping_mul(5))
        .wrapping_add(p[0].wrapping_mul(3)))
        % 64) as usize
}
#[inline]
const fn hash4(p: Pixel4) -> usize {
    ((p[3]
        .wrapping_mul(11)
        .wrapping_add(p[2].wrapping_mul(7))
        .wrapping_add(p[1].wrapping_mul(5))
        .wrapping_add(p[0].wrapping_mul(3)))
        % 64) as usize
}
