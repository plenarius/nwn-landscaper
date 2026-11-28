// SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
// SPDX-License-Identifier: GPL-3.0-or-later

//! A canvas: a landscape larger than one area, cut into pieces that each become an area (docs/DESIGN-canvas-cut.md, question 8,
//! build step 3). This is its storage; the routes are serve/canvases.rs.
//!
//! One folder a canvas, `canvases/<name>/` in the workspace:
//!
//! ```text
//! canvas.json        the header: name, size, ground height, pieces, counters, palette, water, entrances, objects, things, settings
//! blocks/b03_02.blk  the ground of the 16 by 16 tile block at (3, 2): its heights, paint and chasm mask
//! ```
//!
//! **Blocks** are fixed: they never move with the cuts. Block (x, y) covers tiles 16x to 16x + 16 west to east and 16y to 16y + 16
//! south to north, fewer at the canvas's east and north edges: 257 by 257 samples, or 16 a tile plus one for a smaller one.
//! Neighbouring blocks repeat their shared sample row or column, as pieces do, so each block is a complete grid; a save that
//! would leave a shared row different in two blocks is refused ([`Seam`]). A block that was never saved has no file and is flat
//! ground at the header's `ground` height, unpainted (palette entry 0), without chasm. Only a missing file counts as that:
//! a block file that is there and can't be read, or is damaged, stops every load and save that needs it ([`Damaged`]) and is
//! never written over.
//!
//! **A block file** is one file, so a save replaces heights, paint and chasm together ([`crate::atomic::write`]); three files
//! could be left half replaced by a crash. Little-endian throughout:
//!
//! | Bytes | What |
//! |---|---|
//! | 4 | `LSCB` |
//! | 2 | format, 1 |
//! | 2 | flags: 1 when a chasm mask follows, 2 when a blend layer does |
//! | 2, 2 | width and height in samples |
//! | 4 a sample | heights in metres (f32), rows from the south, each west to east |
//! | 1 a sample | paint: an index into the header's `palette` |
//! | 1 a sample | the chasm mask, 0 or 1, only with flag 1 (a block without chasm has none) |
//! | 2 a sample | the blend layer (soft paint: a second palette index and its weight, see heightmap.rs), only with flag 2 |
//! | 8 | the FNV-1a hash (64 bits) of every byte before it |
//!
//! A block whose blend weights are all 0 has no layer and today's bytes: only a block that holds one is unreadable to an engine
//! older than the layer (which refuses flag 2 as damaged; it has nowhere to skip it).
//!
//! **Versions.** The header's version is the FNV-1a hash of `canvas.json`, as an area's is of its working copy; a block's is
//! the hash its file ends with, or `none` for a block with no file. Each is 16 hex digits. Every load says the versions of what
//! it returns, and every save names the version it started from (`base`) of each thing it changes: the header, and each
//! block. A save is refused whole ([`Moved`]) when any of them is no longer the one on disk, so two tabs never overwrite each
//! other. A delete names the versions of everything the tab holds.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;

/// Tiles a block covers each way.
pub const BLOCK_TILES: usize = 16;
/// Samples a tile, each way (0.625 m apart; an area is 16 a tile plus one).
pub const TILE_SAMPLES: usize = 16;
/// The largest canvas each way, in tiles (question 1: 4 full pieces of 32).
pub const MAX_TILES: usize = 128;
/// The smallest canvas each way, in tiles: one piece's least.
pub const MIN_TILES: usize = 3;
/// A piece's least and most tiles a side (an NWN area's limits, question 2).
pub const PIECE_TILES: std::ops::RangeInclusive<u64> = 3..=32;
/// The version of a block that has no file (flat ground).
pub const NONE: &str = "none";
/// The header's file in a canvas's folder.
pub const HEADER: &str = "canvas.json";

const MAGIC: &[u8; 4] = b"LSCB";
const FORMAT: u16 = 1;
/// Magic, format, flags, width, height.
const HEAD_BYTES: usize = 12;
const TRAILER_BYTES: usize = 8;

/// A canvas's name: 1 to 7 lower case letters, digits or `_`, so a piece's default name (`<canvas><two digits>`) fits an
/// area's 9 (question 6).
pub fn name_ok(name: &str) -> bool {
    (1..=7).contains(&name.len()) && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// FNV-1a, 64 bits, as serve/areas.rs `version`: stable across Rust versions, so a restart never makes a tab's version stale.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// The version of a header file's bytes.
pub fn header_version(bytes: &[u8]) -> String {
    format!("{:016x}", fnv(bytes))
}

/// A canvas's size in tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub tiles_x: usize,
    pub tiles_y: usize,
}

impl Size {
    /// Blocks each way.
    pub fn blocks(self) -> (usize, usize) {
        (self.tiles_x.div_ceil(BLOCK_TILES), self.tiles_y.div_ceil(BLOCK_TILES))
    }

    /// Whether the canvas has block `id`.
    pub fn has(self, id: BlockId) -> bool {
        let (bx, by) = self.blocks();
        id.x < bx && id.y < by
    }

    /// Block `id`'s width and height in samples (it must be one of the canvas's).
    pub fn samples(self, id: BlockId) -> (usize, usize) {
        let side = |tiles: usize, k: usize| (tiles - k * BLOCK_TILES).min(BLOCK_TILES) * TILE_SAMPLES + 1;
        (side(self.tiles_x, id.x), side(self.tiles_y, id.y))
    }

    /// Every block of the canvas, rows from the south.
    pub fn all(self) -> impl Iterator<Item = BlockId> {
        let (bx, by) = self.blocks();
        (0..by).flat_map(move |y| (0..bx).map(move |x| BlockId { x, y }))
    }
}

/// A block's place: (x, y) in blocks from the south-west corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId {
    pub x: usize,
    pub y: usize,
}

impl BlockId {
    /// Its name: `b03_02` for (3, 2), as its file is named and the editor names it.
    pub fn key(self) -> String {
        format!("b{:02}_{:02}", self.x, self.y)
    }

    /// The block named `key` (`b03_02`), if that is a block's name.
    pub fn parse(key: &str) -> Option<BlockId> {
        let (x, y) = key.strip_prefix('b')?.split_once('_')?;
        let n = |s: &str| (s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten();
        Some(BlockId { x: n(x)?, y: n(y)? })
    }

    fn file(self) -> String {
        format!("{}.blk", self.key())
    }
}

/// The ground of one block.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub width: usize,
    pub height: usize,
    /// Heights in metres, rows from the south, each west to east.
    pub heights: Vec<f32>,
    /// Paint, one palette index a sample.
    pub paint: Vec<u8>,
    /// The chasm mask, 0 or 1 a sample; None: no chasm in this block.
    pub chasm: Option<Vec<u8>>,
    /// The blend layer, [second, weight] a sample; None: no sample has a second texture.
    pub blend: Option<Vec<[u8; 2]>>,
}

impl Block {
    /// Flat ground at `ground`, unpainted and without chasm: a block that was never saved.
    pub fn flat(width: usize, height: usize, ground: f32) -> Block {
        let n = width * height;
        Block { width, height, heights: vec![ground; n], paint: vec![0; n], chasm: None, blend: None }
    }

    /// A block from its parts, checked: each part one value a sample, heights numbers, the chasm mask 0 or 1. A mask without
    /// a chasm sample is dropped, so the same ground always has the same bytes and version.
    pub fn new(width: usize, height: usize, heights: Vec<f32>, paint: Vec<u8>, chasm: Option<Vec<u8>>) -> Result<Block> {
        let n = width * height;
        ensure!(heights.len() == n, "{} heights for a {width} by {height} block: {n} expected", heights.len());
        ensure!(paint.len() == n, "{} paint values for a {width} by {height} block: {n} expected", paint.len());
        if let Some(k) = heights.iter().position(|h| !h.is_finite()) {
            bail!("the height at sample ({}, {}) is {}: heights must be numbers", k % width, k / width, heights[k]);
        }
        if let Some(c) = &chasm {
            ensure!(c.len() == n, "{} chasm values for a {width} by {height} block: {n} expected", c.len());
            ensure!(c.iter().all(|v| *v <= 1), "the chasm mask holds 0 or 1 a sample");
        }
        let chasm = chasm.filter(|c| c.contains(&1));
        Ok(Block { width, height, heights, paint, chasm, blend: None })
    }

    /// The block with blend layer `bytes` (two a sample), checked. A sample of weight 0 is stored as [0, 0] and a layer with
    /// none above 0 is dropped, so the same ground always has the same bytes and version.
    pub fn with_blend(mut self, bytes: Option<&[u8]>) -> Result<Block> {
        self.blend = match bytes {
            None => None,
            Some(b) => {
                let n = self.width * self.height;
                ensure!(b.len() == n * 2, "{} blend bytes for a {} by {} block: {} expected", b.len(), self.width, self.height, n * 2);
                let pairs: Vec<[u8; 2]> = b.as_chunks::<2>().0.iter().map(|p| if p[1] == 0 { [0, 0] } else { *p }).collect();
                pairs.iter().any(|p| p[1] > 0).then_some(pairs)
            }
        };
        Ok(self)
    }

    /// The block file's bytes (see the module comment).
    pub fn encode(&self) -> Vec<u8> {
        let n = self.width * self.height;
        let mut out = Vec::with_capacity(HEAD_BYTES + n * 6 + TRAILER_BYTES);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&FORMAT.to_le_bytes());
        let flags = u16::from(self.chasm.is_some()) | u16::from(self.blend.is_some()) << 1;
        out.extend_from_slice(&flags.to_le_bytes());
        for side in [self.width, self.height] {
            out.extend_from_slice(&u16::try_from(side).unwrap_or(u16::MAX).to_le_bytes());
        }
        for h in &self.heights {
            out.extend_from_slice(&h.to_le_bytes());
        }
        out.extend_from_slice(&self.paint);
        if let Some(c) = &self.chasm {
            out.extend_from_slice(c);
        }
        if let Some(b) = &self.blend {
            out.extend_from_slice(b.as_flattened());
        }
        let hash = fnv(&out);
        out.extend_from_slice(&hash.to_le_bytes());
        out
    }

    /// The block in a block file's `bytes`, which must be `width` by `height` samples; what is wrong with it otherwise.
    pub fn decode(bytes: &[u8], width: usize, height: usize) -> Result<Block> {
        ensure!(bytes.len() >= HEAD_BYTES + TRAILER_BYTES && &bytes[..4] == MAGIC, "not a canvas block");
        let (body, trailer) = bytes.split_at(bytes.len() - TRAILER_BYTES);
        let stored = u64::from_le_bytes(trailer.try_into().unwrap_or_default());
        ensure!(fnv(body) == stored, "its contents don't match its check sum (cut off or changed)");
        let u16_at = |k: usize| usize::from(u16::from_le_bytes([body[k], body[k + 1]]));
        ensure!(u16_at(4) == usize::from(FORMAT), "format {} (this engine reads {FORMAT})", u16_at(4));
        let flags = u16_at(6);
        ensure!(flags <= 3, "unknown flags {flags}");
        ensure!(
            (u16_at(8), u16_at(10)) == (width, height),
            "{} by {} samples where the canvas has {width} by {height}",
            u16_at(8),
            u16_at(10)
        );
        let n = width * height;
        let (has_chasm, has_blend) = (flags & 1 == 1, flags & 2 == 2);
        let want = HEAD_BYTES + n * (5 + usize::from(has_chasm) + 2 * usize::from(has_blend));
        ensure!(body.len() == want, "{} bytes, {} expected", bytes.len(), want + TRAILER_BYTES);
        let (heights, rest) = body[HEAD_BYTES..].split_at(n * 4);
        let (paint, rest) = rest.split_at(n);
        let (chasm, blend) = rest.split_at(if has_chasm { n } else { 0 });
        let heights = heights.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
        let chasm = has_chasm.then(|| chasm.to_vec());
        ensure!(chasm.as_ref().is_none_or(|c| c.contains(&1)), "a chasm mask without a chasm sample");
        let block = Block::new(width, height, heights, paint.to_vec(), chasm)?.with_blend(has_blend.then_some(blend))?;
        ensure!(!has_blend || block.blend.is_some(), "a blend layer without a weight above 0");
        Ok(block)
    }

    /// The samples along one side: `West`'s column 0, `East`'s last column, `South`'s row 0, `North`'s last row. Each as
    /// (height bits, paint, chasm, blend), so two sides compare exactly.
    fn side(&self, side: Side) -> Vec<(u32, u8, u8, [u8; 2])> {
        let at = |k: usize| {
            (
                self.heights[k].to_bits(),
                self.paint[k],
                self.chasm.as_ref().map_or(0, |c| c[k]),
                self.blend.as_ref().map_or([0, 0], |b| b[k]),
            )
        };
        let (w, h) = (self.width, self.height);
        match side {
            Side::West => (0..h).map(|j| at(j * w)).collect(),
            Side::East => (0..h).map(|j| at(j * w + w - 1)).collect(),
            Side::South => (0..w).map(at).collect(),
            Side::North => (0..w).map(|i| at((h - 1) * w + i)).collect(),
        }
    }
}

#[derive(Clone, Copy)]
enum Side {
    West,
    East,
    South,
    North,
}

/// A file of a canvas that is there but can't be read, or is damaged: every load and save that needs it stops, and it is never
/// written over. Names the file as the builder knows it (`canvases/rivl/blocks/b03_02.blk`).
#[derive(Debug)]
pub struct Damaged {
    pub file: String,
    pub why: String,
}

impl std::fmt::Display for Damaged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is damaged ({}): nothing was changed; restore it from a backup, or ask an admin", self.file, self.why)
    }
}

impl std::error::Error for Damaged {}

/// A save or delete that started from versions that are no longer the ones on disk: another tab or computer saved since. Holds
/// the versions there now of what moved (the header as `canvas.json`, each block by its name).
#[derive(Debug)]
pub struct Moved {
    pub name: String,
    /// The header's version there now.
    pub version: String,
    /// What moved, and its version there now.
    pub moved: BTreeMap<String, String>,
}

impl std::fmt::Display for Moved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what: Vec<&str> = self.moved.keys().map(String::as_str).collect();
        write!(
            f,
            "{} was saved from another tab or computer since you loaded it ({}): nothing was changed; load those again",
            self.name,
            what.join(", ")
        )
    }
}

impl std::error::Error for Moved {}

/// A canvas the workspace hasn't got.
#[derive(Debug)]
pub struct NoCanvas(pub String);

impl std::fmt::Display for NoCanvas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "there is no canvas called {}", self.0)
    }
}

impl std::error::Error for NoCanvas {}

/// A save that would leave two neighbouring blocks with different samples on the row or column they share.
#[derive(Debug)]
pub struct Seam(pub String);

impl std::fmt::Display for Seam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: send both blocks in one save", self.0)
    }
}

impl std::error::Error for Seam {}

/// One canvas's folder, with the name it is shown by.
pub struct Canvas {
    pub name: String,
    pub dir: PathBuf,
}

/// The canvas header's fields the engine reads and keeps.
#[derive(Clone, Debug)]
pub struct Header {
    pub json: Value,
    pub size: Size,
    pub ground: f32,
    /// Its version ([`header_version`]).
    pub version: String,
}

impl Header {
    /// The header in `json`, for canvas `name`: its name, size (3 to 128 tiles each way) and ground height checked, and its
    /// pieces ([`check_pieces`]).
    pub fn parse(name: &str, json: Value, version: String) -> Result<Header> {
        ensure!(json.is_object(), "a canvas header is a JSON object");
        ensure!(json["name"].as_str() == Some(name), "that isn't canvas {name}");
        let tiles = |k: &str, or: Option<u64>| -> Result<usize> {
            let t = json[k].as_u64().or(or).with_context(|| format!("{k}: the canvas's size in tiles"))?;
            ensure!((MIN_TILES as u64..=MAX_TILES as u64).contains(&t), "a canvas is {MIN_TILES} to {MAX_TILES} tiles each way, not {t}");
            Ok(t as usize)
        };
        let tiles_x = tiles("tiles", None)?;
        let tiles_y = tiles("tilesY", Some(tiles_x as u64))?;
        let ground = match &json["ground"] {
            Value::Null => 0.0,
            v => v.as_f64().filter(|g| g.is_finite() && g.abs() < 1e6).context("ground: a height in metres")? as f32,
        };
        let size = Size { tiles_x, tiles_y };
        check_pieces(&json, size)?;
        Ok(Header { json, size, ground, version })
    }

    fn created(&self) -> Option<&Value> {
        self.json.get("created").filter(|c| !c.is_null() && c.as_str() != Some(""))
    }

    /// The pieces' ids and the counter they come from.
    fn piece_ids(&self) -> (Vec<u64>, u64) {
        let ids: Vec<u64> = self.json["pieces"].as_array().into_iter().flatten().filter_map(|p| p["id"].as_u64()).collect();
        let counter = self.json["pieceNextId"].as_u64().unwrap_or(1).max(ids.iter().map(|i| i + 1).max().unwrap_or(1));
        (ids, counter)
    }
}

/// The pieces in `header` (`pieces: [{ id, name, x, y, w, h }]`, tiles from the south-west corner): each id a whole number from
/// 1, each name an area's (1 to 9 lower case letters, digits or `_`), neither used twice; each rectangle 3 to 32 tiles a side,
/// on the canvas, overlapping no other. Gaps are allowed (question 2).
pub fn check_pieces(header: &Value, size: Size) -> Result<()> {
    let list = match &header["pieces"] {
        Value::Null => return Ok(()),
        v => v.as_array().context("pieces: a list")?,
    };
    let mut rects: Vec<(u64, [u64; 4])> = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    for p in list {
        let id = p["id"].as_u64().filter(|i| *i >= 1).with_context(|| format!("a piece's id is {}: a whole number from 1", p["id"]))?;
        let name = p["name"].as_str().unwrap_or_default();
        ensure!(
            !name.is_empty() && name.len() <= 9 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "piece {id}'s name {name:?}: 1 to 9 lower case letters, digits or _"
        );
        ensure!(names.insert(name.to_string()), "two pieces are called {name}");
        let f = |k: &str| p[k].as_u64().with_context(|| format!("piece {name}: {k} in tiles"));
        let r = [f("x")?, f("y")?, f("w")?, f("h")?];
        ensure!(PIECE_TILES.contains(&r[2]) && PIECE_TILES.contains(&r[3]), "piece {name} is {} by {} tiles: 3 to 32 a side", r[2], r[3]);
        ensure!(
            r[0] + r[2] <= size.tiles_x as u64 && r[1] + r[3] <= size.tiles_y as u64,
            "piece {name} goes past the canvas's {} by {} tiles",
            size.tiles_x,
            size.tiles_y
        );
        if let Some((other, _)) = rects.iter().find(|(o, _)| *o == id) {
            bail!("two pieces have the id {other}");
        }
        if let Some((other, _)) =
            rects.iter().find(|(_, q)| r[0] < q[0] + q[2] && q[0] < r[0] + r[2] && r[1] < q[1] + q[3] && q[1] < r[1] + r[3])
        {
            bail!("piece {name} overlaps piece {other}");
        }
        rects.push((id, r));
    }
    Ok(())
}

impl Canvas {
    /// Canvas `name` in the folder of canvases `root`.
    pub fn at(root: &Path, name: &str) -> Canvas {
        Canvas { name: name.to_string(), dir: root.join(name) }
    }

    /// Whether the canvas's folder is there.
    pub fn exists(&self) -> bool {
        self.dir.exists()
    }

    fn shown(&self, file: &str) -> String {
        format!("canvases/{}/{file}", self.name)
    }

    fn damaged(&self, file: &str, why: impl std::fmt::Display) -> anyhow::Error {
        Damaged { file: self.shown(file), why: why.to_string() }.into()
    }

    /// The header. [`NoCanvas`] when there is no such canvas; [`Damaged`] when its folder is there but its header is missing,
    /// can't be read or doesn't parse.
    pub fn header(&self) -> Result<Header> {
        let bytes = match crate::atomic::read(&self.dir.join(HEADER)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !self.dir.exists() => return Err(NoCanvas(self.name.clone()).into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(self.damaged(HEADER, "it is missing")),
            Err(e) => return Err(self.damaged(HEADER, format!("it can't be read: {e}"))),
        };
        let json: Value = serde_json::from_slice(&bytes).map_err(|e| self.damaged(HEADER, format!("it doesn't parse: {e}")))?;
        Header::parse(&self.name, json, header_version(&bytes)).map_err(|e| self.damaged(HEADER, format!("{e:#}")))
    }

    fn block_path(&self, id: BlockId) -> PathBuf {
        self.dir.join("blocks").join(id.file())
    }

    /// Block `id` and its version: flat ground and [`NONE`] when it has no file; [`Damaged`] when its file can't be read or
    /// isn't a whole block of the right size.
    pub fn block(&self, header: &Header, id: BlockId) -> Result<(Block, String)> {
        let (w, h) = header.size.samples(id);
        let file = format!("blocks/{}", id.file());
        match crate::atomic::read(&self.block_path(id)) {
            Ok(bytes) => {
                let block = Block::decode(&bytes, w, h).map_err(|e| self.damaged(&file, format!("{e:#}")))?;
                Ok((block, trailer_version(&bytes)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((Block::flat(w, h, header.ground), NONE.to_string())),
            Err(e) => Err(self.damaged(&file, format!("it can't be read: {e}"))),
        }
    }

    /// The bytes of block `id`'s file as they are, and its version; None for a block with no file. Checked as [`Canvas::block`].
    pub fn block_bytes(&self, header: &Header, id: BlockId) -> Result<Option<(Vec<u8>, String)>> {
        let (w, h) = header.size.samples(id);
        let file = format!("blocks/{}", id.file());
        match crate::atomic::read(&self.block_path(id)) {
            Ok(bytes) => {
                Block::decode(&bytes, w, h).map_err(|e| self.damaged(&file, format!("{e:#}")))?;
                let v = trailer_version(&bytes);
                Ok(Some((bytes, v)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(self.damaged(&file, format!("it can't be read: {e}"))),
        }
    }

    /// The versions of the blocks that have a file, by name. Read from each file's last 8 bytes, with its size checked
    /// against the block's (with or without a chasm mask), not its whole contents: a block's own load checks those.
    pub fn block_versions(&self, header: &Header) -> Result<BTreeMap<String, String>> {
        use std::io::{Read, Seek, SeekFrom};
        let mut out = BTreeMap::new();
        for id in header.size.all() {
            let file = format!("blocks/{}", id.file());
            let mut f = match crate::atomic::open(&self.block_path(id)) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(self.damaged(&file, format!("it can't be read: {e}"))),
            };
            let (w, h) = header.size.samples(id);
            let len = f.metadata().map_err(|e| self.damaged(&file, e))?.len();
            // (Without and with a chasm mask, then the same with a blend layer.)
            let sizes = [5, 6, 7, 8].map(|b| HEAD_BYTES + w * h * b + TRAILER_BYTES);
            if !sizes.iter().any(|s| *s as u64 == len) {
                return Err(self.damaged(&file, format!("{len} bytes, one of {sizes:?} expected")));
            }
            let mut tail = [0u8; TRAILER_BYTES];
            f.seek(SeekFrom::End(-(TRAILER_BYTES as i64))).and_then(|_| f.read_exact(&mut tail)).map_err(|e| self.damaged(&file, e))?;
            out.insert(id.key(), format!("{:016x}", u64::from_le_bytes(tail)));
        }
        Ok(out)
    }
}

/// The version a block file's bytes end with.
fn trailer_version(bytes: &[u8]) -> String {
    let tail: [u8; TRAILER_BYTES] = bytes[bytes.len() - TRAILER_BYTES..].try_into().unwrap_or_default();
    format!("{:016x}", u64::from_le_bytes(tail))
}

/// The lock of one canvas (its folder): saves, the canvas's making and its delete take turns, so a check of versions and the
/// writes after it are never split by another save.
pub fn lock(dir: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    type Locks = std::collections::HashMap<PathBuf, std::sync::Arc<std::sync::Mutex<()>>>;
    static LOCKS: std::sync::Mutex<Option<Locks>> = std::sync::Mutex::new(None);
    let mut g = LOCKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    g.get_or_insert_with(Default::default).entry(dir.to_path_buf()).or_default().clone()
}

/// What one save changes: the header (with the version it started from), and blocks (each with the version it started from).
#[derive(Default)]
pub struct Save {
    pub header: Option<(Value, String)>,
    pub blocks: Vec<(BlockId, String, Block)>,
}

/// The versions a save, a load or the canvas's making answers with: the header's, and each block's it saved (a save) or that
/// has a file (a load).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Versions {
    pub version: String,
    pub blocks: BTreeMap<String, String>,
}

/// What a save will write, checked, before anything is written: the bytes of each file, and how many more bytes the canvas
/// takes after it (for a team server's quota).
pub struct Planned {
    writes: Vec<(PathBuf, Vec<u8>)>,
    pub adding: u64,
    header: Header,
    blocks: BTreeMap<String, String>,
}

impl Canvas {
    /// Checks `save` against what is on disk (the caller holds [`lock`]): every base is the version there now, else [`Moved`]
    /// with all that moved; the header's name, size, ground and `created` don't change and no piece id comes back; and every
    /// shared row a saved block changes matches its neighbour's as it will be ([`Seam`]). Nothing is written.
    pub fn plan(&self, save: Save) -> Result<Planned> {
        let old = self.header()?;
        let mut moved = BTreeMap::new();
        let mut writes = Vec::new();
        let mut header_write = None;
        let mut adding: i64 = 0;
        let header = match save.header {
            Some((json, base)) => {
                if base != old.version {
                    moved.insert(HEADER.to_string(), old.version.clone());
                }
                let mut new = Header::parse(&self.name, json, String::new())?;
                ensure!(
                    new.size == old.size && new.ground.to_bits() == old.ground.to_bits(),
                    "a canvas keeps its size and ground height ({} by {} tiles at {} m)",
                    old.size.tiles_x,
                    old.size.tiles_y,
                    old.ground
                );
                ensure!(old.created().is_none_or(|c| new.created() == Some(c)), "that is another canvas called {}", self.name);
                // A piece's number is given once (question 6): one the canvas had before and took out never comes back.
                let (was, counter) = old.piece_ids();
                let (ids, next) = new.piece_ids();
                if let Some(id) = ids.iter().find(|i| **i < counter && !was.contains(i)) {
                    bail!("piece id {id} was a piece's that has been taken out: a new piece needs a new id ({counter} or more)");
                }
                new.json["pieceNextId"] = next.max(counter).into();
                let bytes = serde_json::to_vec(&new.json)?;
                new.version = header_version(&bytes);
                adding += bytes.len() as i64 - std::fs::metadata(self.dir.join(HEADER)).map_or(0, |m| m.len() as i64);
                header_write = Some((self.dir.join(HEADER), bytes));
                new
            }
            None => old.clone(),
        };
        let mut saving: BTreeMap<BlockId, Block> = BTreeMap::new();
        let mut before: BTreeMap<BlockId, Block> = BTreeMap::new();
        let mut versions = BTreeMap::new();
        for (id, base, block) in save.blocks {
            ensure!(old.size.has(id), "the canvas has no block {} ({} by {} blocks)", id.key(), old.size.blocks().0, old.size.blocks().1);
            ensure!(!saving.contains_key(&id), "block {} is in the save twice", id.key());
            let (w, h) = old.size.samples(id);
            ensure!(
                (block.width, block.height) == (w, h),
                "block {} is {w} by {h} samples, not {} by {}",
                id.key(),
                block.width,
                block.height
            );
            // (Read whole, not only its version: a damaged block is never written over.)
            let (was, there) = self.block(&old, id)?;
            if base != there {
                moved.insert(id.key(), there);
            }
            before.insert(id, was);
            saving.insert(id, block);
        }
        if !moved.is_empty() {
            return Err(Moved { name: self.name.clone(), version: old.version, moved }.into());
        }
        // Shared rows: each side a saved block changes, against its neighbour as it will be. (A side it leaves as it was isn't
        // this save's to answer for: a crash between two blocks' writes could have left it different, and that must not
        // stop every later save of the block.)
        let neighbour = |id: BlockId| -> Result<Option<Block>> {
            if !old.size.has(id) {
                return Ok(None);
            }
            match saving.get(&id) {
                Some(b) => Ok(Some(b.clone())),
                None => self.block(&old, id).map(|(b, _)| Some(b)),
            }
        };
        for (id, block) in &saving {
            let sides = [
                (id.x.checked_sub(1).map(|x| BlockId { x, y: id.y }), Side::West, Side::East, "column"),
                (Some(BlockId { x: id.x + 1, y: id.y }), Side::East, Side::West, "column"),
                (id.y.checked_sub(1).map(|y| BlockId { x: id.x, y }), Side::South, Side::North, "row"),
                (Some(BlockId { x: id.x, y: id.y + 1 }), Side::North, Side::South, "row"),
            ];
            for (other, mine, theirs, what) in sides {
                let Some(other) = other else { continue };
                if before[id].side(mine) == block.side(mine) {
                    continue;
                }
                if let Some(n) = neighbour(other)?
                    && block.side(mine) != n.side(theirs)
                {
                    bail!(Seam(format!("blocks {} and {} differ on the sample {what} they share", id.key(), other.key())));
                }
            }
        }
        for (id, block) in saving {
            let bytes = block.encode();
            adding += bytes.len() as i64 - std::fs::metadata(self.block_path(id)).map_or(0, |m| m.len() as i64);
            versions.insert(id.key(), trailer_version(&bytes));
            writes.push((self.block_path(id), bytes));
        }
        // (Blocks first, the header last.)
        writes.extend(header_write);
        Ok(Planned { writes, adding: adding.max(0) as u64, header, blocks: versions })
    }

    /// Writes what [`Canvas::plan`] checked, each file whole ([`crate::atomic::write`]); the versions now.
    pub fn write(&self, plan: Planned) -> Result<Versions> {
        for (path, bytes) in &plan.writes {
            crate::atomic::write(path, bytes)?;
        }
        Ok(Versions { version: plan.header.version, blocks: plan.blocks })
    }

    /// Makes the canvas from `header` (the caller holds [`lock`] and has checked the name is free): its folder is made beside
    /// the others under a temporary name and renamed into place, so no one ever sees a canvas without its header. No block has
    /// a file yet: every block is flat ground at `ground`.
    pub fn create(&self, header: Value) -> Result<Versions> {
        let mut h = Header::parse(&self.name, header, String::new())?;
        let (_, counter) = h.piece_ids();
        h.json["pieceNextId"] = counter.into();
        let bytes = serde_json::to_vec(&h.json)?;
        let root = self.dir.parent().context("a canvas's folder")?;
        std::fs::create_dir_all(root)?;
        ensure!(!self.dir.exists(), "there is already a canvas called {}", self.name);
        let tmp = root.join(format!(".{}.new-{}", self.name, crate::pid()));
        let _ = std::fs::remove_dir_all(&tmp);
        let made = (|| -> Result<()> {
            std::fs::create_dir_all(tmp.join("blocks"))?;
            crate::atomic::write(&tmp.join(HEADER), &bytes)?;
            std::fs::rename(&tmp, &self.dir)?;
            Ok(())
        })();
        if made.is_err() {
            let _ = std::fs::remove_dir_all(&tmp);
        }
        made.with_context(|| format!("making canvas {}", self.name))?;
        Ok(Versions { version: header_version(&bytes), blocks: BTreeMap::new() })
    }

    /// Checks that `base` (the header's version) and `blocks` (each block's, a block left out counting as [`NONE`]) are the
    /// versions on disk now, as a delete needs (the caller holds [`lock`]): [`Moved`] with what differs otherwise.
    pub fn check_unchanged(&self, base: &str, blocks: &BTreeMap<String, String>) -> Result<()> {
        let header = self.header()?;
        let there = self.block_versions(&header)?;
        let mut moved = BTreeMap::new();
        if base != header.version {
            moved.insert(HEADER.to_string(), header.version.clone());
        }
        for id in header.size.all() {
            let key = id.key();
            let now = there.get(&key).map_or(NONE, String::as_str);
            if blocks.get(&key).map_or(NONE, String::as_str) != now {
                moved.insert(key, now.to_string());
            }
        }
        if let Some(k) = blocks.keys().find(|k| BlockId::parse(k).is_none_or(|id| !header.size.has(id))) {
            bail!("the canvas has no block {k}");
        }
        if !moved.is_empty() {
            return Err(Moved { name: self.name.clone(), version: header.version, moved }.into());
        }
        Ok(())
    }
}

/// The canvases in the folder `root`, by name: (name, the header or why it can't be read, when it was last saved in Unix
/// seconds).
pub fn list(root: &Path) -> Vec<(String, Result<Header>, u64)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let Some(name) = e.file_name().to_str().filter(|n| name_ok(n)).map(str::to_string) else { continue };
        let c = Canvas::at(root, &name);
        let last = newest(&c.dir);
        out.push((name, c.header(), last));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// When anything in `dir` (its header, its blocks) was last written, in Unix seconds.
fn newest(dir: &Path) -> u64 {
    let secs = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs())
    };
    let blocks = std::fs::read_dir(dir.join("blocks")).into_iter().flatten().flatten().map(|e| secs(&e.path())).max().unwrap_or(0);
    secs(&dir.join(HEADER)).max(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ls-canvas-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn header(name: &str, tiles: u64, tiles_y: u64) -> Value {
        serde_json::json!({ "name": name, "tiles": tiles, "tilesY": tiles_y, "ground": 2.5, "palette": ["grassMedium"], "created": "c1" })
    }

    /// Block `id` of `size` with every height `f(i, j)` on canvas samples (so neighbours agree on their shared rows).
    fn block_from(size: Size, id: BlockId, f: impl Fn(usize, usize) -> f32) -> Block {
        let (w, h) = size.samples(id);
        let (x0, y0) = (id.x * BLOCK_TILES * TILE_SAMPLES, id.y * BLOCK_TILES * TILE_SAMPLES);
        let heights = (0..h).flat_map(|j| (0..w).map(move |i| (i, j))).map(|(i, j)| f(x0 + i, y0 + j)).collect();
        Block::new(w, h, heights, vec![0; w * h], None).unwrap()
    }

    #[test]
    fn blocks_cover_the_canvas_and_repeat_their_shared_rows() {
        let s = Size { tiles_x: 100, tiles_y: 40 };
        assert_eq!(s.blocks(), (7, 3));
        assert_eq!(s.samples(BlockId { x: 0, y: 0 }), (257, 257));
        // 100 = 6 * 16 + 4; 40 = 2 * 16 + 8.
        assert_eq!(s.samples(BlockId { x: 6, y: 2 }), (65, 129));
        // Together the blocks hold the canvas's samples, each shared column once more per cut.
        let width: usize = (0..7).map(|x| s.samples(BlockId { x, y: 0 }).0).sum::<usize>() - 6;
        assert_eq!(width, 100 * 16 + 1);
        let big = Size { tiles_x: 128, tiles_y: 128 };
        assert_eq!((big.blocks(), big.all().count()), ((8, 8), 64));
        assert!(big.has(BlockId { x: 7, y: 7 }) && !big.has(BlockId { x: 8, y: 0 }));
        assert_eq!(big.samples(BlockId { x: 7, y: 7 }), (257, 257));
    }

    #[test]
    fn block_names_read_back_and_nothing_else_reads_as_one() {
        let id = BlockId { x: 3, y: 12 };
        assert_eq!(id.key(), "b03_12");
        assert_eq!(BlockId::parse("b03_12"), Some(id));
        for bad in ["", "b3_2", "b03-02", "b03_02.blk", "c03_02", "b0a_02", "b+3_02", "b03_002", "../b03_02"] {
            assert_eq!(BlockId::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_block_file_reads_back_and_its_version_is_its_check_sum() {
        let mut chasm = vec![0u8; 17 * 33];
        chasm[40] = 1;
        let b = Block::new(17, 33, (0..17 * 33).map(|k| k as f32 * 0.5).collect(), vec![3; 17 * 33], Some(chasm)).unwrap();
        let bytes = b.encode();
        assert_eq!(bytes.len(), HEAD_BYTES + 17 * 33 * 6 + TRAILER_BYTES);
        assert_eq!(Block::decode(&bytes, 17, 33).unwrap(), b);
        assert_eq!(trailer_version(&bytes), format!("{:016x}", fnv(&bytes[..bytes.len() - 8])));
        // No chasm sample: no mask, and the same bytes as a block that never had one.
        let plain = Block::new(17, 33, b.heights.clone(), b.paint.clone(), Some(vec![0; 17 * 33])).unwrap();
        assert_eq!(plain.chasm, None);
        assert_eq!(plain.encode().len(), HEAD_BYTES + 17 * 33 * 5 + TRAILER_BYTES);
        assert_eq!(plain.encode(), Block::new(17, 33, b.heights.clone(), b.paint.clone(), None).unwrap().encode());
        // A blend layer, with and without the chasm mask: after them, read back; weight-0 samples stored as [0, 0].
        let mut layer = [9u8, 0].repeat(17 * 33);
        layer[2 * 50..2 * 50 + 2].copy_from_slice(&[1, 200]);
        for chasm in [b.chasm.clone(), None] {
            let blended = Block::new(17, 33, b.heights.clone(), b.paint.clone(), chasm.clone()).unwrap().with_blend(Some(&layer)).unwrap();
            let bytes = blended.encode();
            assert_eq!(bytes.len(), HEAD_BYTES + 17 * 33 * (7 + usize::from(chasm.is_some())) + TRAILER_BYTES);
            let back = Block::decode(&bytes, 17, 33).unwrap();
            assert_eq!(back, blended);
            let bl = back.blend.unwrap();
            assert_eq!((bl[50], bl[51]), ([1, 200], [0, 0]));
        }
        // All weights 0: no layer, today's bytes.
        let zero = plain.clone().with_blend(Some(&[9u8, 0].repeat(17 * 33))).unwrap();
        assert_eq!(zero.encode(), plain.encode());
        assert!(plain.clone().with_blend(Some(&[1, 2, 3])).is_err());
    }

    #[test]
    fn a_damaged_block_file_is_never_read_as_ground() {
        let b = Block::flat(17, 17, 1.0).encode();
        let flipped = {
            let mut x = b.clone();
            x[100] ^= 1;
            x
        };
        let mut nan = Block::flat(17, 17, 1.0);
        nan.heights[5] = f32::NAN;
        let nan = {
            // (Encoded by hand: `Block::new` refuses it.)
            let mut x = nan.encode();
            let n = x.len();
            x.truncate(n - 8);
            let h = fnv(&x);
            x.extend_from_slice(&h.to_le_bytes());
            x
        };
        for (bytes, why) in [
            (&b[..b.len() - 1], "check sum"),
            (&flipped[..], "check sum"),
            (&b"LSCB"[..], "not a canvas block"),
            (&nan[..], "heights must be numbers"),
            (&[0u8; 2000][..], "not a canvas block"),
        ] {
            let e = format!("{:#}", Block::decode(bytes, 17, 17).unwrap_err());
            assert!(e.contains(why), "{why}: {e}");
        }
        let e = format!("{:#}", Block::decode(&b, 17, 33).unwrap_err());
        assert!(e.contains("17 by 17 samples where the canvas has 17 by 33"), "{e}");
    }

    #[test]
    fn headers_keep_to_the_canvas_limits() {
        assert!(Header::parse("rivl", header("rivl", 128, 128), String::new()).is_ok());
        assert!(Header::parse("rivl", header("rivl", 3, 3), String::new()).is_ok());
        for (t, ty) in [(129, 128), (128, 2), (0, 10)] {
            let e = format!("{:#}", Header::parse("rivl", header("rivl", t, ty), String::new()).unwrap_err());
            assert!(e.contains("3 to 128 tiles"), "{t} {ty}: {e}");
        }
        assert!(Header::parse("rivl", header("other", 10, 10), String::new()).is_err());
        assert!(name_ok("rivl") && name_ok("a_1") && name_ok("abcdefg"));
        for bad in ["", "abcdefgh", "Rivl", "a-b", "a.b", "../x"] {
            assert!(!name_ok(bad), "{bad:?}");
        }
    }

    #[test]
    fn pieces_are_rectangles_of_3_to_32_tiles_that_never_overlap() {
        let size = Size { tiles_x: 100, tiles_y: 100 };
        let with = |pieces: Value| check_pieces(&serde_json::json!({ "pieces": pieces }), size);
        use serde_json::json;
        let p = |id: u64, name: &str, x: u64, y: u64, w: u64, h: u64| json!({ "id": id, "name": name, "x": x, "y": y, "w": w, "h": h });
        assert!(
            with(serde_json::json!([p(1, "rivl01", 0, 0, 32, 32), p(2, "rivl02", 32, 0, 32, 25), p(4, "rivl04", 90, 90, 10, 10)])).is_ok()
        );
        for (bad, why) in [
            (vec![p(1, "a", 0, 0, 33, 10)], "3 to 32 a side"),
            (vec![p(1, "a", 0, 0, 2, 10)], "3 to 32 a side"),
            (vec![p(1, "a", 90, 0, 11, 10)], "goes past"),
            (vec![p(1, "a", 0, 0, 10, 10), p(2, "b", 9, 9, 10, 10)], "overlaps piece 1"),
            (vec![p(1, "a", 0, 0, 10, 10), p(1, "b", 20, 0, 10, 10)], "two pieces have the id 1"),
            (vec![p(1, "a", 0, 0, 10, 10), p(2, "a", 20, 0, 10, 10)], "two pieces are called a"),
            (vec![p(0, "a", 0, 0, 10, 10)], "from 1"),
            (vec![p(1, "Bad", 0, 0, 10, 10)], "lower case"),
        ] {
            let e = format!("{:#}", with(Value::Array(bad)).unwrap_err());
            assert!(e.contains(why), "{why}: {e}");
        }
    }

    #[test]
    fn a_save_checks_every_base_and_the_shared_rows_before_it_writes() {
        let root = scratch("save");
        let c = Canvas::at(&root, "rivl");
        let made = c.create(header("rivl", 40, 20)).unwrap();
        let h = c.header().unwrap();
        assert_eq!((h.version.clone(), h.size.blocks()), (made.version.clone(), (3, 2)));
        // A block never saved: flat at the ground height.
        let (b, v) = c.block(&h, BlockId { x: 2, y: 1 }).unwrap();
        assert_eq!((b.width, b.height, v.as_str(), b.heights[0]), (129, 65, NONE, 2.5));
        // A hill across the cut between (0, 0) and (1, 0), saved in both blocks.
        // (Within 100 samples of (256, 100): no other block's edge.)
        let hill = |i: usize, j: usize| 2.5 + (10.0 - (i as f32 - 256.0).hypot(j as f32 - 100.0) / 10.0).max(0.0);
        let (a, b) = (BlockId { x: 0, y: 0 }, BlockId { x: 1, y: 0 });
        let size = h.size;
        let saved = c
            .write(
                c.plan(Save {
                    header: None,
                    blocks: vec![(a, NONE.into(), block_from(size, a, hill)), (b, NONE.into(), block_from(size, b, hill))],
                })
                .unwrap(),
            )
            .unwrap();
        assert_eq!(saved.version, made.version, "the header didn't change");
        assert_eq!(saved.blocks.keys().collect::<Vec<_>>(), ["b00_00", "b01_00"]);
        assert_eq!(c.block_versions(&h).unwrap(), saved.blocks);
        // The hill's west half alone: its shared column no longer matches the block beside it.
        let lone = block_from(size, a, |i, j| hill(i, j) + 1.0);
        let e = c.plan(Save { header: None, blocks: vec![(a, saved.blocks["b00_00"].clone(), lone)] }).err().unwrap();
        assert!(e.downcast_ref::<Seam>().is_some_and(|s| s.0.contains("b00_00 and b01_00")), "{e:#}");
        // (0, 1), north of the hill's block: its shared row must stay the hill's.
        let north = BlockId { x: 0, y: 1 };
        let e = c.plan(Save { header: None, blocks: vec![(north, NONE.into(), block_from(size, north, |_, _| 7.0))] }).err().unwrap();
        assert!(e.downcast_ref::<Seam>().is_some(), "{e:#}");
        // A blend layer on the shared column in one block only: a seam too; inside the block, saved and read back.
        let blended = |k: usize| {
            let b = block_from(size, a, hill);
            let mut layer = vec![0u8; b.width * b.height * 2];
            layer[2 * k..2 * k + 2].copy_from_slice(&[0, 90]);
            b.with_blend(Some(&layer)).unwrap()
        };
        let edge = blended(size.samples(a).0 - 1);
        let e = c.plan(Save { header: None, blocks: vec![(a, saved.blocks["b00_00"].clone(), edge)] }).err().unwrap();
        assert!(e.downcast_ref::<Seam>().is_some(), "{e:#}");
        let inner = blended(5);
        let again =
            c.write(c.plan(Save { header: None, blocks: vec![(a, saved.blocks["b00_00"].clone(), inner.clone())] }).unwrap()).unwrap();
        assert_eq!(c.block(&h, a).unwrap(), (inner, again.blocks["b00_00"].clone()));
        assert_eq!(c.block_versions(&h).unwrap()["b00_00"], again.blocks["b00_00"]);
        // (Back to the hill alone: the same bytes, so the same version as before.)
        let back = c.plan(Save { header: None, blocks: vec![(a, again.blocks["b00_00"].clone(), block_from(size, a, hill))] }).unwrap();
        assert_eq!(c.write(back).unwrap().blocks["b00_00"], saved.blocks["b00_00"]);
        // A stale base for one block refuses the whole save, and says the version there now.
        let e = c
            .plan(Save {
                header: Some((header("rivl", 40, 20), made.version.clone())),
                blocks: vec![(a, NONE.into(), block_from(size, a, hill)), (b, saved.blocks["b01_00"].clone(), block_from(size, b, hill))],
            })
            .err()
            .unwrap();
        let m = e.downcast_ref::<Moved>().unwrap();
        assert_eq!(m.moved.len(), 1);
        assert_eq!(m.moved["b00_00"], saved.blocks["b00_00"]);
        // A pair a crash left different (one block of a save written, not the other): a save that leaves that side alone goes
        // through, so the builder isn't stuck; one that changes it must match.
        let lone = block_from(size, a, |i, j| hill(i, j) + 1.0);
        crate::atomic::write(&c.block_path(a), &lone.encode()).unwrap();
        let mut inside = lone.clone();
        inside.heights[300] += 1.0;
        let base = trailer_version(&lone.encode());
        assert!(c.plan(Save { header: None, blocks: vec![(a, base, inside)] }).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_header_save_keeps_the_size_the_creator_and_every_piece_number() {
        let root = scratch("header");
        let c = Canvas::at(&root, "rivl");
        let made = c.create(header("rivl", 64, 64)).unwrap();
        let piece = |id: u64, x: u64| serde_json::json!({ "id": id, "name": format!("rivl{id:02}"), "x": x, "y": 0, "w": 32, "h": 32 });
        let save = |h: Value, base: &str| c.plan(Save { header: Some((h, base.to_string())), blocks: vec![] }).and_then(|p| c.write(p));
        let mut h = header("rivl", 64, 64);
        h["pieces"] = serde_json::json!([piece(1, 0), piece(2, 32)]);
        let v = save(h.clone(), &made.version).unwrap().version;
        assert_eq!(c.header().unwrap().json["pieceNextId"], 3);
        // Piece 2 joined into 1; then a new piece may not take number 2 again.
        h["pieces"] = serde_json::json!([piece(1, 0)]);
        let v = save(h.clone(), &v).unwrap().version;
        h["pieces"] = serde_json::json!([piece(1, 0), piece(2, 32)]);
        assert!(format!("{:#}", save(h.clone(), &v).unwrap_err()).contains("piece id 2 was a piece's"));
        h["pieces"] = serde_json::json!([piece(1, 0), piece(3, 32)]);
        let v = save(h.clone(), &v).unwrap().version;
        for (k, val, why) in [
            ("tiles", serde_json::json!(65), "keeps its size"),
            ("ground", serde_json::json!(0), "keeps its size"),
            ("created", serde_json::json!("c2"), "another canvas"),
        ] {
            let mut other = h.clone();
            other[k] = val;
            assert!(format!("{:#}", save(other, &v).unwrap_err()).contains(why), "{k}");
        }
        assert_eq!(c.header().unwrap().version, v, "refusals changed nothing");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_damaged_header_or_block_stops_everything_that_needs_it() {
        let root = scratch("damaged");
        let c = Canvas::at(&root, "rivl");
        let made = c.create(header("rivl", 20, 20)).unwrap();
        let h = c.header().unwrap();
        let id = BlockId { x: 1, y: 1 };
        std::fs::write(c.block_path(id), b"LSCB nonsense").unwrap();
        let e = format!("{:#}", c.block_versions(&h).unwrap_err());
        assert!(e.contains("canvases/rivl/blocks/b01_01.blk is damaged") && !e.contains(&*root.to_string_lossy()), "{e}");
        let fresh = Block::flat(65, 65, 2.5);
        assert!(c.plan(Save { header: None, blocks: vec![(id, NONE.into(), fresh)] }).err().unwrap().downcast_ref::<Damaged>().is_some());
        assert_eq!(std::fs::read(c.block_path(id)).unwrap(), b"LSCB nonsense", "never written over");
        // A neighbour's save that changes their shared column needs it too: refused, not taken as flat ground.
        let west = BlockId { x: 0, y: 1 };
        let raised = Block::flat(257, 65, 3.0);
        assert!(
            c.plan(Save { header: None, blocks: vec![(west, NONE.into(), raised)] }).err().unwrap().downcast_ref::<Damaged>().is_some()
        );
        // A header that doesn't parse; one that is missing from its folder.
        std::fs::write(c.dir.join(HEADER), b"{\"name\": \"rivl\",").unwrap();
        assert!(format!("{:#}", c.header().unwrap_err()).contains("canvases/rivl/canvas.json is damaged (it doesn't parse"));
        std::fs::remove_file(c.dir.join(HEADER)).unwrap();
        assert!(format!("{:#}", c.header().unwrap_err()).contains("canvas.json is damaged (it is missing)"));
        assert!(c.check_unchanged(&made.version, &BTreeMap::new()).is_err());
        assert!(Canvas::at(&root, "none").header().unwrap_err().downcast_ref::<NoCanvas>().is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_delete_needs_every_version_the_tab_holds() {
        let root = scratch("delete");
        let c = Canvas::at(&root, "rivl");
        let made = c.create(header("rivl", 16, 16)).unwrap();
        assert!(c.check_unchanged(&made.version, &BTreeMap::new()).is_ok());
        let id = BlockId { x: 0, y: 0 };
        let saved = c.write(c.plan(Save { header: None, blocks: vec![(id, NONE.into(), Block::flat(257, 257, 3.0))] }).unwrap()).unwrap();
        // The tab that loaded before the save holds `none` for the block.
        let e = c.check_unchanged(&made.version, &BTreeMap::new()).unwrap_err();
        assert_eq!(e.downcast_ref::<Moved>().unwrap().moved["b00_00"], saved.blocks["b00_00"]);
        assert!(c.check_unchanged(&saved.version, &saved.blocks).is_ok());
        let mut extra = saved.blocks.clone();
        extra.insert("b05_00".into(), NONE.into());
        assert!(c.check_unchanged(&saved.version, &extra).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
