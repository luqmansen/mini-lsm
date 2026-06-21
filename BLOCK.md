# Block Layer — Agent Context Map

> Purpose: prebuilt map of the **block** subsystem so an agent can answer block
> questions without re-grepping. Paths point at the **reference solution** crate
> `mini-lsm/` (fully implemented). The user implements **`mini-lsm-starter/`**,
> where `BlockBuilder::{new,add,is_empty,build}` and all of
> `BlockIterator::{create_and_seek_to_first,create_and_seek_to_key,key,value,
> is_valid,seek_to_first,next,seek_to_key}` are `unimplemented!()` stubs
> (confirmed). Treat `mini-lsm/` as the answer key, `mini-lsm-starter/` as the
> worksheet. `mini-lsm-mvcc/` is the same shape plus a timestamp baked into each
> key.
>
> A block is the smallest unit of read and caching in the LSM tree: one
> contiguous chunk of sorted key/value pairs (default target ~4KB). It is built
> in memory (`BlockBuilder`), serialized to bytes (`Block::encode`), stored
> inside an SST, later read back and decoded (`Block::decode`), then walked with
> `BlockIterator`. For block↔SST wiring and the iterator's role in the merge
> stack, see `SST.md` and `ITERATORS.md`.

## 1. Files

| File | Holds |
|------|-------|
| `mini-lsm/src/block.rs` | `Block` struct, `encode`/`decode`, `SIZEOF_U16`, `get_first_key` |
| `mini-lsm/src/block/builder.rs` | `BlockBuilder`, `compute_overlap` |
| `mini-lsm/src/block/iterator.rs` | `BlockIterator` (decode side) — also covered in `ITERATORS.md` |

## 2. The in-memory struct

`mini-lsm/src/block.rs`

```rust
pub struct Block {
    pub(crate) data: Vec<u8>,     // all encoded entries, back to back
    pub(crate) offsets: Vec<u16>, // byte offset of each entry's start within `data`
}
pub(crate) const SIZEOF_U16: usize = 2;
```

Two parallel arrays: `data` is the raw entry bytes; `offsets[i]` is where the
i-th entry begins inside `data`. `offsets.len()` == number of entries. Both are
`u16`-scale, which is why a block is bounded (~64KB max addressable, target 4KB).

## 3. On-disk / encoded layout

This is the single most important picture. `encode()` lays out:

```
┌─────────────────────────────────────────────┬──────────────────────┬───────────┐
│              Data Section                    │   Offset Section     │  Extra    │
│  entry_0 | entry_1 | ... | entry_{n-1}       │ off_0 off_1 .. off_{n-1} │ num_of_elem │
│           (variable length)                  │   (n * u16)          │  (u16)    │
└─────────────────────────────────────────────┴──────────────────────┴───────────┘
```

- Each offset is `u16`; the trailing `num_of_elem` (`u16`) is how `decode`
  knows where the offset section starts.
- One **entry** (one key/value pair) inside the data section:

```
┌────────────┬──────────┬──────────────┬───────────┬──────────────┐
│ overlap_len│ rest_len │ key_rest     │ value_len │ value        │
│   u16      │   u16    │ rest_len B   │   u16     │ value_len B   │
└────────────┴──────────┴──────────────┴───────────┴──────────────┘
```

  - `overlap_len` = how many leading bytes this key shares with the block's
    **first key** (prefix compression).
  - `rest_len` = `key.len() - overlap_len` = length of the suffix actually
    stored.
  - `key_rest` = the suffix bytes; the full key is
    `first_key[..overlap_len] ++ key_rest`.
  - `value_len` + `value` = the value, stored in full.

## 4. Prefix compression (the clever bit)

`compute_overlap(first_key, key)` in `block/builder.rs` walks both byte slices
and counts the shared leading prefix. Because keys in a block are sorted, keys
near each other usually share a long prefix, so storing only the suffix saves
space.

Mechanical decode story (in `BlockIterator::seek_to_offset`,
`block/iterator.rs`): to rebuild a key, take `overlap_len` bytes from the stored
`first_key` (the iterator keeps `first_key: KeyVec`), then append the stored
`key_rest`. Only the **first** entry in a block stores its full key
(`overlap_len == 0`), and the builder captures it via `block.get_first_key()`
(`block.rs`) when an iterator is created.

`value()` is cheaper: it returns a slice straight into `block.data` using the
stored `value_range: (start, end)` — no copy, just two indices.

## 5. BlockBuilder — the encode side

`mini-lsm/src/block/builder.rs`

```rust
pub struct BlockBuilder {
    offsets: Vec<u16>,   // accumulates entry offsets
    data: Vec<u8>,       // accumulates encoded entries
    block_size: usize,   // target cap (soft)
    first_key: KeyVec,   // first key added; basis for prefix compression
}
```

- `add(key, value) -> bool` (`#[must_use]`): appends one entry, returns `false`
  if adding it would exceed `block_size` **and** the block is non-empty (so a
  single oversized entry is still allowed into an empty block — never produce an
  empty block).
  - Order of operations: push `data.len()` as the new offset → compute overlap
    vs `first_key` → write `overlap_len`, `rest_len`, `key_rest`, `value_len`,
    `value` → if `first_key` empty, set it to this key.
- `estimated_size()` = trailing `num_of_elem` u16 + offsets array + data; used
  by `add` to decide fullness. The fullness check adds `key.len() + value.len()
  + SIZEOF_U16 * 3` (the two length prefixes + the new offset slot).
- `is_empty()` = `offsets.is_empty()`.
- `build()` → `Block { data, offsets }`; panics if empty.

The `bool` return is the back-pressure signal the SST builder watches: when
`add` returns `false`, the SST layer finalizes this block and starts a new one
(see `SST.md`).

## 6. decode — the read side

`Block::decode(data)` (`block.rs`) reverses `encode`:

1. Read the last `u16` → `entry_offsets_len` (number of entries).
2. `data_end = len - 2 - entry_offsets_len*2` marks the boundary between the
   data section and the offset section.
3. Slice `[data_end .. len-2]` as the offsets, `chunks(2).map(get_u16)`.
4. Slice `[0 .. data_end]` as the entry data.

Note: `decode` here takes the **block bytes only**. The per-block CRC32 checksum
is stripped one layer up by `SsTable::read_block` before `decode` is called (see
`SST.md`), so the block format itself carries no checksum.

## 7. Quick "where do I look" index

| Question about... | Read |
|---|---|
| block struct / encode / decode layout | `src/block.rs` |
| prefix compression, fullness/back-pressure, building | `src/block/builder.rs` |
| decoding entries, rebuilding keys, seek within a block | `src/block/iterator.rs` (+ `ITERATORS.md`) |
| how blocks sit inside an SST, checksums, caching | `SST.md` |

Replace `src/` with `mini-lsm/src/` (answer key) or `mini-lsm-starter/src/`
(worksheet, bodies are `unimplemented!()`).
