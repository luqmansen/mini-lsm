# SST (SSTable / Table) Layer — Agent Context Map

> Purpose: prebuilt map of the **SST** subsystem so an agent can answer table
> questions without re-grepping. Paths point at the **reference solution** crate
> `mini-lsm/` (fully implemented). The user implements **`mini-lsm-starter/`**,
> where `SsTableBuilder::{new,add,estimated_size,build}` and
> `SsTableIterator::{create_and_seek_to_first,seek_to_first,
> create_and_seek_to_key,seek_to_key}` are `unimplemented!()` stubs (confirmed;
> the rest of `table.rs`/`bloom.rs` is provided even in the starter). Treat
> `mini-lsm/` as the answer key, `mini-lsm-starter/` as the worksheet.
> `mini-lsm-mvcc/` adds a real `max_ts` and per-key timestamps.
>
> An SST (Sorted String Table) is an immutable on-disk file: a sequence of
> encoded **blocks** (see `BLOCK.md`) followed by metadata that indexes those
> blocks, plus a bloom filter. It is produced by flushing a memtable or by
> compaction, then read back via `SsTable::open` and walked by `SsTableIterator`
> (see `ITERATORS.md`).

## 1. Files

| File | Holds |
|------|-------|
| `mini-lsm/src/table.rs` | `SsTable`, `BlockMeta`, `FileObject`, open/read/checksum logic |
| `mini-lsm/src/table/builder.rs` | `SsTableBuilder` (the encode/flush side) |
| `mini-lsm/src/table/bloom.rs` | `Bloom` filter + `BitSlice`/`BitSliceMut` |
| `mini-lsm/src/table/iterator.rs` | `SsTableIterator` — covered in `ITERATORS.md` |

## 2. On-disk file layout (the master picture)

Built by `SsTableBuilder::build`, read by `SsTable::open`:

```
┌──────────────────────────────────────────────────────────────────────┐
│  Data Section: block_0 ‖ block_1 ‖ ... ‖ block_{n-1}                   │
│    each block is:  [ encoded Block bytes ][ CRC32 checksum (u32) ]      │
├──────────────────────────────────────────────────────────────────────┤
│  Meta Section: encode_block_meta(block_meta)  (BlockMeta[] + CRC32)     │
├──────────────────────────────────────────────────────────────────────┤
│  meta_offset (u32)   <- byte offset where Meta Section begins           │
├──────────────────────────────────────────────────────────────────────┤
│  Bloom Section: bloom.encode()  (filter bits + k + CRC32)               │
├──────────────────────────────────────────────────────────────────────┤
│  bloom_offset (u32)  <- byte offset where Bloom Section begins          │
└──────────────────────────────────────────────────────────────────────┘
```

Note the two **trailing offset pointers**: the very last `u32` is
`bloom_offset`, and the `u32` immediately before the bloom section is
`meta_offset`. `open` reads the file from the back forward: read last 4 bytes →
`bloom_offset` → decode bloom → read 4 bytes before bloom → `meta_offset` →
decode meta. This back-to-front pattern mirrors how a block stores its
`num_of_elem` at the end (see `BLOCK.md`).

## 3. BlockMeta — the in-SST index

`mini-lsm/src/table.rs`

```rust
pub struct BlockMeta {
    pub offset: usize,      // byte offset of this block within the Data Section
    pub first_key: KeyBytes,// first key in the block
    pub last_key: KeyBytes, // last key in the block
}
```

One `BlockMeta` per data block. This is the index that lets a reader pick the
right block without scanning the whole file.

- `encode_block_meta(metas, buf)`: writes `count (u32)`, then per meta
  `offset(u32) | first_key_len(u16) | first_key | last_key_len(u16) | last_key`,
  then a `CRC32` over everything written.
- `decode_block_meta(buf)`: reverses it and **verifies the CRC32** (`bail!`s on
  mismatch).
- `KeyBytes` is the owned, `Bytes`-backed key form (no lifetime) — see the key
  family in `ITERATORS.md`.

## 4. SsTable — the read handle

`mini-lsm/src/table.rs`

```rust
pub struct SsTable {
    file: FileObject,                  // the backing file (read via pread)
    block_meta: Vec<BlockMeta>,        // index, loaded fully into memory
    block_meta_offset: usize,          // where meta starts (= end of data section)
    id: usize,
    block_cache: Option<Arc<BlockCache>>,
    first_key: KeyBytes,               // = block_meta.first().first_key
    last_key: KeyBytes,                // = block_meta.last().last_key
    bloom: Option<Bloom>,
    max_ts: u64,                       // 0 in week 1-2; real value in mvcc
}
```

Key methods:

- `open(id, block_cache, file)`: the back-to-front parse in section 2; populates
  `block_meta`, `first_key`/`last_key`, `bloom`.
- `read_block(idx)`: computes the block's byte span as
  `[meta[idx].offset .. meta[idx+1].offset)` (or `block_meta_offset` for the
  last block), reads those bytes, splits off the trailing 4-byte CRC32, verifies
  it, then hands the **block bytes only** to `Block::decode` (this is why
  `BLOCK.md` says the block format itself carries no checksum).
- `read_block_cached(idx)`: same, but goes through the `moka`-based
  `BlockCache` keyed on `(sst_id, block_idx)` via `try_get_with` (load-once
  under contention). `SsTableIterator` uses the cached path.
- `find_block_idx(key)`: `partition_point` over `first_key`s, `saturating_sub(1)`
  — returns the block that *may* contain `key`. Binary search, same shape as
  `SstConcatIterator`'s level search in `ITERATORS.md`.
- accessors: `first_key()`, `last_key()`, `num_of_blocks()`, `table_size()`,
  `sst_id()`, `max_ts()`.

`FileObject` (same file) wraps a read-only `File` and its size; `read(offset,
len)` is positioned `pread` (`read_exact_at`), so reads don't move a shared
cursor — safe for concurrent block reads.

## 5. SsTableBuilder — the encode/flush side

`mini-lsm/src/table/builder.rs`

```rust
pub struct SsTableBuilder {
    builder: BlockBuilder,   // the block currently being filled
    first_key: KeyVec,       // first key of the CURRENT block
    last_key: KeyVec,        // last key of the CURRENT block
    data: Vec<u8>,           // finished blocks (+ their checksums) accumulate here
    meta: Vec<BlockMeta>,    // one entry per finished block
    block_size: usize,
    key_hashes: Vec<u32>,    // farmhash fingerprints of every key, for the bloom
}
```

- `add(key, value)`: records the key's `farmhash::fingerprint32` into
  `key_hashes`, then tries `self.builder.add(key, value)` (the block-level
  back-pressure `bool` from `BLOCK.md`). If it returns `false` (block full),
  call `finish_block()` to seal the current block and start a fresh one, then
  re-add the pair (asserted to succeed). Maintains `first_key`/`last_key` of the
  in-progress block.
- `finish_block()`: `std::mem::replace`s the live `BlockBuilder` with a new one,
  `build().encode()`s the old block, pushes a `BlockMeta { offset = data.len(),
  first_key, last_key }`, appends the encoded block to `data`, then appends a
  `CRC32` of that block. (`std::mem::take` moves the keys out without cloning.)
- `build(id, block_cache, path)`: `finish_block()` for the tail block, then lay
  out the file per section 2 — append meta via `encode_block_meta`, append
  `meta_offset` u32, build the bloom from `key_hashes` at ~1% false-positive
  rate, append bloom, append `bloom_offset` u32, write the whole buffer through
  `FileObject::create`, and return a fully-populated `SsTable`.

## 6. Bloom filter — skip SSTs that can't hold the key

`mini-lsm/src/table/bloom.rs`

```rust
pub struct Bloom { filter: Bytes /* the bit array */, k: u8 /* # hash funcs */ }
```

What a bloom filter is, mechanically: a bit array plus `k` hash functions. To
insert a key, hash it `k` ways and set those `k` bits. To test a key, hash it
`k` ways and check those bits — if **any** is 0 the key is **definitely
absent**; if all are 1 the key is **probably present** (false positives
possible, false negatives impossible). That asymmetry is the whole point: it
lets `get` skip reading an SST that provably lacks the key.

- `BitSlice`/`BitSliceMut` are blanket-impl'd on `AsRef<[u8]>`/`AsMut<[u8]>` to
  give `get_bit`/`set_bit`/`bit_len` over raw bytes (`idx/8` selects the byte,
  `idx%8` the bit).
- `bloom_bits_per_key(entries, fpr)`: the standard sizing formula
  (`-n·ln(fpr)/ln(2)^2 / n`). `build` calls it with `fpr = 0.01`.
- `build_from_key_hashes(hashes, bits_per_key)`: derives `k ≈ bits_per_key·0.69`
  (clamped 1..=30), sizes the bit array, and for each key hash uses
  double-hashing (`delta = h.rotate_left(15)`, advance `h += delta` each round)
  to set `k` bits.
- `may_contain(h)`: same double-hash walk; returns `false` the instant a
  required bit is 0.
- `encode`/`decode`: append `filter ‖ k(u8) ‖ CRC32`; decode verifies the CRC.

How it's used in `get` (`lsm_storage.rs`, the `keep_table` closure ~L505): first
check `key_within(key, first_key, last_key)` (range prune via the SST's
first/last key), then `bloom.may_contain(farmhash::fingerprint32(key))`. Only if
both pass does it actually build an `SsTableIterator` and read blocks. This is
the day-7 read optimization.

## 7. Lifecycle summary

```
memtable flush / compaction
  └─ SsTableBuilder::add(key,val)*  → fills BlockBuilder; seals blocks on full
       └─ SsTableBuilder::build(..) → writes Data ‖ Meta ‖ Bloom + offsets to disk
            → SsTable (read handle)
                 ├─ get():  range-prune + bloom → SsTableIterator::seek_to_key
                 └─ scan(): SsTableIterator (L0) or SstConcatIterator (levels)
                      └─ read_block_cached → SsTable::read_block → Block::decode
                           → BlockIterator   (see BLOCK.md / ITERATORS.md)
```

## 8. Quick "where do I look" index

| Question about... | Read |
|---|---|
| SST struct, open, read_block, checksums, caching, find_block_idx | `src/table.rs` |
| block index entries (offset/first_key/last_key), meta encode/decode | `src/table.rs` (`BlockMeta`) |
| positioned file reads (pread) | `src/table.rs` (`FileObject`) |
| building/flushing an SST, when a block gets sealed | `src/table/builder.rs` |
| bloom filter math, bit ops, may_contain | `src/table/bloom.rs` |
| iterating an SST (seek to first/key, block→block) | `src/table/iterator.rs` (+ `ITERATORS.md`) |
| inside one block (entry format, prefix compression) | `BLOCK.md` |
| bloom/range pruning in the get path | `src/lsm_storage.rs` `get` (`keep_table`) |

Replace `src/` with `mini-lsm/src/` (answer key) or `mini-lsm-starter/src/`
(worksheet, the listed builder/iterator bodies are `unimplemented!()`).
