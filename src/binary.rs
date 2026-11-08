// SPDX-FileCopyrightText: 2016 Radu
// SPDX-FileCopyrightText: 2016-2022 Perfare
// SPDX-FileCopyrightText: 2021-2026 aelurum
// SPDX-FileCopyrightText: 2026 SombrAbsol
//
// SPDX-License-Identifier: MIT

//! Binary reader.
//! Derived from AssetStudioMod by aelurum:
//! https://github.com/aelurum/AssetStudioMod

use anyhow::{Result, bail};

/// Reader of a byte buffer, little-endian unless told otherwise.
pub struct Reader<'a> {
    data: &'a [u8],
    pub pos: usize,
    pub little_endian: bool,
}

impl<'a> Reader<'a> {
    /// Creates a little-endian reader.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            little_endian: true,
        }
    }

    /// Creates a big-endian reader.
    pub fn big_endian(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            little_endian: false,
        }
    }

    /// Moves to an absolute position.
    pub fn seek(&mut self, pos: usize) {
        self.pos = pos;
    }

    /// Reads exactly `n` bytes.
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n);
        match end {
            Some(end) if end <= self.data.len() => {
                let s = &self.data[self.pos..end];
                self.pos = end;
                Ok(s)
            }
            _ => bail!("unexpected end of data"),
        }
    }

    /// Reads `N` bytes, reversed for a big-endian reader.
    fn arr<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.bytes(N)?);
        if !self.little_endian {
            a.reverse();
        }
        Ok(a)
    }

    /// Reads an unsigned byte.
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    /// Reads a byte as a boolean.
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }
    /// Reads a 16-bit signed integer.
    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.arr()?))
    }
    /// Reads a 16-bit unsigned integer.
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.arr()?))
    }
    /// Reads a 32-bit signed integer.
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.arr()?))
    }
    /// Reads a 32-bit unsigned integer.
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    /// Reads a 64-bit signed integer.
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.arr()?))
    }

    /// Reads up to `count` bytes, fewer at the end of the data.
    pub fn read_bytes(&mut self, count: i32) -> Result<&'a [u8]> {
        if count < 0 {
            bail!("negative size {count}");
        }
        let start = self.pos.min(self.data.len());
        let end = start.saturating_add(count as usize).min(self.data.len());
        self.pos = end.max(self.pos);
        Ok(&self.data[start..end])
    }

    /// Reads the rest of the data.
    pub fn remaining(&mut self) -> &'a [u8] {
        let start = self.pos.min(self.data.len());
        self.pos = self.data.len().max(self.pos);
        &self.data[start..]
    }

    /// Reads a .NET string: a 7-bit encoded length, then as many bytes of
    /// UTF-8.
    pub fn dotnet_string(&mut self) -> Result<String> {
        let mut len: u32 = 0;
        let mut shift = 0;
        loop {
            let b = self.u8()?;
            len |= ((b & 0x7F) as u32) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift > 28 {
                bail!("invalid string length");
            }
        }
        let bytes = self.bytes(len as usize)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    /// Reads up to `count` UTF-8 characters, stopping at a null one.
    pub fn string_to_null(&mut self, count: usize) -> String {
        let mut s = String::new();
        let mut read = 0;
        while self.pos < self.data.len() && read < count {
            let b = self.data[self.pos];
            let n = match b {
                0x00..=0x7F => 1,
                0xC0..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF7 => 4,
                _ => 1,
            };
            let end = (self.pos + n).min(self.data.len());
            let ch = std::str::from_utf8(&self.data[self.pos..end])
                .ok()
                .and_then(|t| t.chars().next())
                .unwrap_or('\u{FFFD}');
            self.pos = if ch == '\u{FFFD}' && n > 1 {
                self.pos + 1
            } else {
                end
            };
            if ch == '\0' {
                break;
            }
            s.push(ch);
            read += 1;
        }
        s
    }

    /// Reads a null-terminated string.
    pub fn cstring(&mut self) -> Result<String> {
        let start = self.pos;
        while self.pos < self.data.len() && self.data[self.pos] != 0 {
            self.pos += 1;
        }
        if self.pos >= self.data.len() {
            bail!("unterminated string");
        }
        let s = String::from_utf8_lossy(&self.data[start..self.pos]).into_owned();
        self.pos += 1;
        Ok(s)
    }

    /// Skips to the next multiple of `n` bytes.
    pub fn align(&mut self, n: usize) {
        let m = self.pos % n;
        if m != 0 {
            self.pos += n - m;
        }
    }
}
