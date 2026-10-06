#![allow(incomplete_features)]
#![feature(explicit_tail_calls)]

use nom::{
    Parser,
    branch::alt,
    bytes::complete::{tag, take},
    combinator::{value, verify},
    number::{self, Endianness},
};
use thiserror::Error;

/// QOI end marker of 8 bytes.
const QOI_END: [u8; 8] = [0, 0, 0, 0, 0, 0, 0, 1];

const ONE: usize = 1;

type Result<'a, O> = core::result::Result<(&'a [u8], O), Failure<nom::error::Error<&'a [u8]>>>;

type Pixel3 = [u8; 3];
type Pixel4 = [u8; 4];

#[derive(Error, Debug)]
pub enum Failure<E> {
    #[error("nom failed {0}")]
    Nom(#[from] nom::Err<E>),
    #[error("QOI file provided wrong width, height or channels")]
    InsufficientBuffer,
    #[error("input was malformed")]
    MalformedInput,
}

struct Qoi {
    header: Header,
    data: Box<[u8]>,
}

pub fn decode(input: &[u8]) -> Result<'_, Qoi> {
    let (mut input, header) = header(input)?;
    // TODO: return error instead of throwing panic on >4gb image.
    let img_bytes = header
        .width
        .checked_mul(header.height)
        .and_then(|len| len.checked_mul(header.channel as u32))
        .expect("image should be less than 4gb");

    let mut buffer: Box<[u8]> =
        vec![
            0;
            usize::try_from(img_bytes).expect("unlucky, `usize` is not big enough for this image")
        ]
        .into();
    //let mut pixel_map: [Pixel; 64] = [[0; 4]; 64];
    //let mut prev_pixel: Pixel = [0, 0, 0, 255];
    todo!();

    Ok((
        input,
        Qoi {
            header,
            data: buffer,
        },
    ))
}

fn loop_chomp_3chan<'a>(
    input: &'a [u8],
    pixel_map: &mut [Pixel3; 64],
    pixel: &mut Pixel3,
    mut buffer: &mut [u8],
) -> Result<'a, ()> {
    const CHANNELS: usize = 3;
    todo!()
}

fn loop_chomp_4chan<'a>(
    mut input: &'a [u8],
    pixel_map: &mut [Pixel4; 64],
    pixel: &mut Pixel4,
    mut buffer: &mut [u8],
) -> Result<'a, ()> {
    const CHANNELS: usize = 4;
    // because QOI must be ending with 8 padding bytes,
    // we can just chomp 8 bytes and match on slices.
    let (_, chomp) = take(8_usize)(input)?;
    match chomp {
        [0, 0, 0, 0, 0, 0, 0, 1] => return Ok((&input[8..], ())),
        &[0b1111_1111, r, g, b, a, ..] => {
            *pixel = [r, g, b, a];
            input = &input[5..];
        }
        &[0b1111_1110, r, g, b, ..] => {
            *pixel = [r, g, b, pixel[3]];
            input = &input[4..];
        }
        &[x, ..] if high2(x) == 0b11 => {
            let bytes_run = low6(x) as usize * CHANNELS;
            // `run` is stored with bias -1;
            // i.e. when in reality it is 1..=62 stored value is 0..62
            let (left, right) = buffer.split_at_mut(bytes_run);
            buffer = right;
            input = &input[1..];
            left.as_chunks_mut()
                .0
                .iter_mut()
                .for_each(|chunk| *chunk = *pixel);
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
            let (red_diff, green_diff, blue_diff) = diff6(x);
            pixel[0] += red_diff;
            pixel[1] += green_diff;
            pixel[2] += blue_diff;
            input = &input[1..];
        }
        &[x, ..] if high2(x) == 0b00 => {
            *pixel = pixel_map[x as usize % 64];
            input = &input[1..];
        }
        _ => return Err(Failure::MalformedInput),
    }

    pixel_map[hash4(*pixel)] = *pixel;

    match buffer.split_first_chunk_mut() {
        None => Err(Failure::InsufficientBuffer),
        Some((chunk, buffer_rest)) => {
            *chunk = *pixel;
            become loop_chomp_4chan(input, pixel_map, pixel, buffer_rest)
        }
    }
}

#[inline(always)]
// Two MSB bits.
const fn high2(x: u8) -> u8 {
    (x & 0b1100_0000) >> 6
}
#[inline(always)]
const fn low6(x: u8) -> u8 {
    x & 0b0011_1111
}
#[inline(always)]
const fn high4(x: u8) -> u8 {
    x >> 4
}
#[inline(always)]
const fn low4(x: u8) -> u8 {
    x & 0b1111
}
#[inline(always)]
const fn diff6(x: u8) -> (u8, u8, u8) {
    let x = low6(x);
    (x >> 4, x >> 2 & 0b11, x & 0b11)
}

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
struct Header {
    width: u32,
    height: u32,
    channel: Channels,
    colorspace: ColorSpace,
}

#[derive(Copy, Clone)]
#[repr(C)]
enum Channels {
    Rgb = 3,
    Rgba = 4,
}

#[derive(Copy, Clone)]
#[repr(C)]
enum ColorSpace {
    SRGBWithLinearAlpha = 0,
    AllChannelLinear = 1,
}

#[inline]
const fn hash3(p: Pixel3) -> usize {
    ((p[2]
        .wrapping_mul(7)
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
