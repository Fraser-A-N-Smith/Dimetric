//! Tile chunks.
//!
//! Tile data is stored per chunk, run-length encoded, one block per 32×32
//! region. Chunking is what gives diffs locality: painting one corner of a room
//! touches one line of the file rather than rewriting the whole layer.
//!
//! Sizing is deliberate. A 128×128-tile room across four layers is about 65,000
//! cells, which lands at 100–150 KB of run-length text typically and about
//! 460 KB pathologically. The format only breaks down somewhere around a
//! 1024×1024 contiguous map, and room-based games never get there.

use dimetric_core::{Code, Diagnostic, NodeUid, StateHasher};

/// Width and height of a chunk, in tiles.
pub const CHUNK_SIZE: i32 = 32;
/// Cells in a chunk.
pub const CHUNK_CELLS: usize = (CHUNK_SIZE * CHUNK_SIZE) as usize;
/// The tile index meaning "nothing here".
pub const EMPTY_TILE: u16 = 0;

/// Where a chunk's cells come from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ChunkData {
    /// Run-length encoded in the scene file itself.
    Inline(Box<[u16; CHUNK_CELLS]>),
    /// A path to an external chunk file.
    ///
    /// Accepted by the parser and not yet implemented, so that externalising
    /// large maps later is an additive change rather than a format break.
    /// There is deliberately no automatic threshold that switches between the
    /// two: a file that changes representation at some size limit produces
    /// baffling diffs and confuses agents.
    External(String),
}

/// One 32×32 region of one tile layer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Chunk {
    /// The `TileLayer` node this belongs to.
    pub layer: NodeUid,
    /// Chunk coordinate, in chunks rather than tiles.
    pub at: [i32; 2],
    /// The cells.
    pub data: ChunkData,
}

impl Chunk {
    /// An empty chunk.
    pub fn empty(layer: NodeUid, at: [i32; 2]) -> Chunk {
        Chunk {
            layer,
            at,
            data: ChunkData::Inline(Box::new([EMPTY_TILE; CHUNK_CELLS])),
        }
    }

    /// The cells, if this chunk stores them inline.
    pub fn cells(&self) -> Option<&[u16; CHUNK_CELLS]> {
        match &self.data {
            ChunkData::Inline(c) => Some(c),
            ChunkData::External(_) => None,
        }
    }

    /// The cells for writing, if this chunk stores them inline.
    pub fn cells_mut(&mut self) -> Option<&mut [u16; CHUNK_CELLS]> {
        match &mut self.data {
            ChunkData::Inline(c) => Some(c),
            ChunkData::External(_) => None,
        }
    }

    /// Read one cell by its offset within the chunk.
    pub fn get(&self, x: i32, y: i32) -> Option<u16> {
        let cells = self.cells()?;
        Some(cells[cell_index(x, y)?])
    }

    /// Write one cell by its offset within the chunk, returning the old value.
    pub fn set(&mut self, x: i32, y: i32, tile: u16) -> Option<u16> {
        let index = cell_index(x, y)?;
        let cells = self.cells_mut()?;
        Some(std::mem::replace(&mut cells[index], tile))
    }

    /// True when every cell is empty.
    pub fn is_empty(&self) -> bool {
        match &self.data {
            ChunkData::Inline(c) => c.iter().all(|t| *t == EMPTY_TILE),
            ChunkData::External(_) => false,
        }
    }

    /// Feed into a state hash.
    pub fn hash_state(&self, h: &mut StateHasher) {
        h.node_uid(self.layer);
        h.i32(self.at[0]).i32(self.at[1]);
        match &self.data {
            ChunkData::Inline(cells) => {
                h.tag("inline").len(cells.len());
                for t in cells.iter() {
                    h.u64(*t as u64);
                }
            }
            ChunkData::External(path) => {
                h.tag("external").str(path);
            }
        }
    }
}

/// Index of a cell within a chunk, row-major.
fn cell_index(x: i32, y: i32) -> Option<usize> {
    if (0..CHUNK_SIZE).contains(&x) && (0..CHUNK_SIZE).contains(&y) {
        Some((y * CHUNK_SIZE + x) as usize)
    } else {
        None
    }
}

/// Which chunk a tile coordinate falls in, and where inside it.
///
/// Uses floor division rather than truncation so that negative coordinates land
/// in the chunk to the left, not the one at zero.
pub fn split_coord(x: i32, y: i32) -> ([i32; 2], [i32; 2]) {
    let cx = x.div_euclid(CHUNK_SIZE);
    let cy = y.div_euclid(CHUNK_SIZE);
    (
        [cx, cy],
        [x.rem_euclid(CHUNK_SIZE), y.rem_euclid(CHUNK_SIZE)],
    )
}

/// Encode cells as `count:tile` pairs.
///
/// Runs are emitted in row-major order and never split, so the encoding of a
/// given grid is unique — which is what lets `scene fmt --check` compare tile
/// data by string equality.
pub fn encode_rle(cells: &[u16]) -> String {
    let mut out = String::new();
    let mut iter = cells.iter().copied();
    let Some(mut current) = iter.next() else {
        return out;
    };
    let mut run = 1usize;
    for tile in iter {
        if tile == current {
            run += 1;
        } else {
            push_run(&mut out, run, current);
            current = tile;
            run = 1;
        }
    }
    push_run(&mut out, run, current);
    out
}

fn push_run(out: &mut String, run: usize, tile: u16) {
    if !out.is_empty() {
        out.push(' ');
    }
    out.push_str(&format!("{run}:{tile}"));
}

/// Decode `count:tile` pairs into exactly [`CHUNK_CELLS`] cells.
pub fn decode_rle(text: &str) -> Result<Box<[u16; CHUNK_CELLS]>, Diagnostic> {
    let mut cells = Box::new([EMPTY_TILE; CHUNK_CELLS]);
    let mut written = 0usize;
    for (i, pair) in text.split_whitespace().enumerate() {
        let (count, tile) = pair.split_once(':').ok_or_else(|| {
            bad_chunk(format!("run {i} is {pair:?}; expected count:tile"))
                .with_field("run", i as i64)
        })?;
        let count: usize = count
            .parse()
            .map_err(|_| bad_chunk(format!("run {i} has a non-numeric count {count:?}")))?;
        let tile: u16 = tile
            .parse()
            .map_err(|_| bad_chunk(format!("run {i} has a non-numeric tile index {tile:?}")))?;
        if written + count > CHUNK_CELLS {
            return Err(
                bad_chunk(format!("runs describe more than {CHUNK_CELLS} cells"))
                    .with_field("cells", (written + count) as i64),
            );
        }
        cells[written..written + count].fill(tile);
        written += count;
    }
    if written != CHUNK_CELLS {
        return Err(bad_chunk(format!(
            "runs describe {written} cells, but a chunk is {CHUNK_CELLS}"
        ))
        .with_field("cells", written as i64));
    }
    Ok(cells)
}

fn bad_chunk(message: String) -> Diagnostic {
    Diagnostic::new(Code::BAD_CHUNK_DATA, message)
}

// -- Reading and writing a layer's grid -----------------------------------
//
// One implementation, two callers: the authoring commands in `dimetric-host`
// and the simulation's own deferred writes. The write logic used to live only
// in `Command::SetTiles`, so giving scripts tiles meant either a second copy of
// chunk allocation and run-length handling, or lifting it here. A second copy
// is how two paths disagree about what an out-of-range write does.

/// The tile at a cell, or [`EMPTY_TILE`] where no chunk has been allocated.
///
/// An unallocated chunk reads as empty rather than as an error: a grid is
/// conceptually infinite and sparsely stored, so "nothing there" is the honest
/// answer for a cell nobody has painted. A script generating a floor reads
/// outside its own bounds constantly — checking the neighbours of an edge cell
/// does it — and an error for that would mean bounds-checking every read.
pub fn tile_at(chunks: &[Chunk], layer: NodeUid, x: i32, y: i32) -> u16 {
    let (chunk_at, cell) = split_coord(x, y);
    chunks
        .iter()
        .find(|c| c.layer == layer && c.at == chunk_at)
        .and_then(|c| c.get(cell[0], cell[1]))
        .unwrap_or(EMPTY_TILE)
}

/// Write one tile, allocating its chunk if needed, and return what was there.
///
/// The previous value is returned because that is what makes the write
/// invertible: `Command::SetTiles` builds its undo from these.
pub fn set_tile_in(
    chunks: &mut Vec<Chunk>,
    layer: NodeUid,
    x: i32,
    y: i32,
    tile: u16,
) -> Result<u16, Diagnostic> {
    let (chunk_at, cell) = split_coord(x, y);
    let index = match chunks
        .iter()
        .position(|c| c.layer == layer && c.at == chunk_at)
    {
        Some(i) => i,
        None => {
            chunks.push(Chunk::empty(layer, chunk_at));
            // Kept in a defined order rather than appended wherever: the chunk
            // list is hashed, and a list whose order depended on which cell a
            // script happened to touch first would hash differently for the
            // same grid (I4).
            chunks.sort_by_key(|c| (c.layer, c.at));
            chunks
                .iter()
                .position(|c| c.layer == layer && c.at == chunk_at)
                .expect("just inserted")
        }
    };
    chunks[index].set(cell[0], cell[1], tile).ok_or_else(|| {
        Diagnostic::new(
            Code::BAD_CHUNK_DATA,
            format!("chunk at {chunk_at:?} stores its cells externally and cannot be edited yet"),
        )
    })
}

/// The tile extent of a layer, as `[x, y, width, height]`.
///
/// Chunk granularity, not cell granularity: it reports the region that has
/// storage, which is what a generator wants to iterate. `None` when the layer
/// has no chunks at all.
pub fn layer_bounds(chunks: &[Chunk], layer: NodeUid) -> Option<[i32; 4]> {
    let mut found = false;
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for chunk in chunks.iter().filter(|c| c.layer == layer) {
        found = true;
        x0 = x0.min(chunk.at[0] * CHUNK_SIZE);
        y0 = y0.min(chunk.at[1] * CHUNK_SIZE);
        x1 = x1.max((chunk.at[0] + 1) * CHUNK_SIZE);
        y1 = y1.max((chunk.at[1] + 1) * CHUNK_SIZE);
    }
    found.then(|| [x0, y0, x1 - x0, y1 - y0])
}
