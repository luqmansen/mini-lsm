# Iterator Architecture — Agent Context Map

> Purpose: a prebuilt map of the iterator subsystem so an agent can answer
> iterator questions without re-grepping the whole tree. Paths below point at
> the **reference solution** crate `mini-lsm/` (fully implemented). The user
> implements the **`mini-lsm-starter/`** crate, where every method body shown
> here is a `unimplemented!()` stub (confirmed: all `MergeIterator`,
> `TwoMergeIterator`, `SstConcatIterator`, `LsmIterator`, `FusedIterator`
> method bodies in the starter are stubs). Treat `mini-lsm/` as the answer key,
> `mini-lsm-starter/` as the worksheet. There is also `mini-lsm-mvcc/` (later
> week-3 MVCC variant) with the same shapes plus timestamps.

## 1. The one trait everything implements

`mini-lsm/src/iterators.rs`

```rust
pub trait StorageIterator {
    type KeyType<'a>: PartialEq + Eq + PartialOrd + Ord where Self: 'a;
    fn value(&self) -> &[u8];
    fn key(&self) -> Self::KeyType<'_>;
    fn is_valid(&self) -> bool;
    fn next(&mut self) -> anyhow::Result<()>;
    fn num_active_iterators(&self) -> usize { 1 } // default
}
```

Mechanical notes for explaining this to the user:

- `StorageIterator` is the **contract / checklist**, NOT a runtime value. It has
  no single in-memory form. Default usage in this codebase is the static-generic
  form (`I: StorageIterator`), where the concrete type is welded in at compile
  time, the value sits inline on the stack/in the struct, and `next()/key()` are
  direct jumps — there is **no fat pointer**. The `Box<I>` in `MergeIterator`
  boxes the value for storage in the heap/`BinaryHeap`, but it is `Box<I>` (one
  concrete type), not `Box<dyn StorageIterator>`, so calls are still direct, not
  vtable lookups.
- `type KeyType<'a>` is a **generic associated type (GAT)** — an associated type
  that itself takes a lifetime. Grounding: `key()` hands back a borrow that
  points into bytes the iterator already owns; the `'a` is the compile-time
  "lease tag" stamping how long that borrow is valid. `KeySlice<'short>` and
  `KeySlice<'long>` are *different types* (same fields, different tag). This is
  why the return type "depends on how long you hold the borrow."
- `where Self: 'a` reads as: "the borrow `'a` cannot outlive the iterator
  itself." You cannot hand out a key that lives longer than the thing that owns
  the bytes.

## 2. The key family (drives all the lifetime talk)

`mini-lsm/src/key.rs`

```rust
pub struct Key<T: AsRef<[u8]>>(T);   // newtype wrapper over some byte container
pub type KeySlice<'a> = Key<&'a [u8]>;  // borrowed view  -> has a lifetime tag
pub type KeyVec       = Key<Vec<u8>>;   // owns its bytes  -> no lifetime
pub type KeyBytes     = Key<Bytes>;     // ref-counted own -> no lifetime
```

- `KeySlice<'a>` is the borrowed form returned by `key()`. It is `Copy`.
- `KeyVec` / `KeyBytes` own their bytes, so they carry no lifetime — they pass
  the `'static` gate by owning (or, for `Bytes`, by `Arc`-style sharing).
- Conversions: `KeyVec::as_key_slice()` -> `KeySlice<'_>`,
  `KeySlice::from_slice(&[u8])`, `KeySlice::raw_ref() -> &[u8]`,
  `KeyVec::from_vec`, `KeySlice::to_key_vec()`.
- In week 1+2 `TS_ENABLED = false`; ordering is plain byte ordering via the
  `impl Ord for Key<T>`. Week 3 (mvcc crate) adds a timestamp to the key.

## 3. The full iterator stack (leaves -> root)

This is the type that `scan()` ultimately builds. From `lsm_iterator.rs`:

```rust
type LsmIteratorInner = TwoMergeIterator<
    TwoMergeIterator<
        MergeIterator<MemTableIterator>,   // all memtables (mutable + frozen)
        MergeIterator<SsTableIterator>,    // L0 SSTs (overlapping key ranges)
    >,
    MergeIterator<SstConcatIterator>,      // L1+ levels (sorted, non-overlapping)
>;
```

Layered view:

```
FusedIterator<LsmIterator>          <- public scan() return; guards invalid/error use
└─ LsmIterator                      <- skips deletes (empty value) + applies end_bound
   └─ TwoMergeIterator              <- merge {memtables+L0} with {lower levels}
      ├─ TwoMergeIterator           <- merge memtables with L0
      │  ├─ MergeIterator<MemTableIterator>
      │  └─ MergeIterator<SsTableIterator>
      └─ MergeIterator<SstConcatIterator>
```

Precedence rule throughout: **smaller index / iterator A wins** on equal keys,
because newer data is placed first (memtable before L0 before lower levels).

### Leaf iterators (the concrete types that fill the generic slots)

| Type | File | Owns | `KeyType` |
|------|------|------|-----------|
| `BlockIterator` | `block/iterator.rs` | `Arc<Block>` | (inherent `key()->KeySlice`, NOT a `StorageIterator` impl) |
| `SsTableIterator` | `table/iterator.rs` | `Arc<SsTable>` + a `BlockIterator` | `KeySlice<'a>` |
| `SstConcatIterator` | `iterators/concat_iterator.rs` | `Vec<Arc<SsTable>>` + current `SsTableIterator` | `KeySlice<'a>` |
| `MemTableIterator` | `mem_table.rs` | `Arc<SkipMap>` (self-referential!) | `KeySlice<'a>` |

`BlockIterator` is the bottom turtle: it decodes prefix-compressed entries out
of `block.data`, rebuilding each key from `overlap_len` bytes of `first_key`
plus the stored suffix. `value()` returns a slice into `block.data` via a stored
`(start,end)` range. It does NOT implement `StorageIterator`; it exposes plain
inherent methods that `SsTableIterator` wraps.

## 4. MemTableIterator — the self-referential one (week 1 day 2)

`mini-lsm/src/mem_table.rs`. This is the trickiest piece and the one most likely
to generate borrow-checker questions.

```rust
#[self_referencing]
pub struct MemTableIterator {
    map: Arc<SkipMap<Bytes, Bytes>>,        // the library
    #[borrows(map)]
    #[not_covariant]
    iter: SkipMapRangeIter<'this>,          // a bookmark INTO map, same struct
    item: (Bytes, Bytes),                   // current (key,value) snapshot
}
```

Mechanical story (this is the "iterator carries the library in its backpack"
case):

- A normal Rust range iterator borrows the map: `map.range(..)` returns
  something tagged with the map's lifetime. If you returned it from a function,
  the map (a local) would die and the bookmark would dangle.
- The fix: store BOTH the `Arc<SkipMap>` (owner) and the range iterator
  (borrower) in the *same struct*. `'this` is the self-referential lifetime: the
  `iter` field borrows from the `map` field of the same instance. `ouroboros`
  (`#[self_referencing]`) generates the unsafe glue so this is sound and gives
  accessors: `borrow_item()`, `with_iter_mut(...)`, `with_mut(...)`, and the
  `MemTableIteratorBuilder { map, iter_builder, item }.build()` constructor.
- `Arc` is the manual equivalent of Go's GC pointer: it keeps the skipmap alive
  as long as the iterator (or anyone) holds a handle, satisfying `'static`
  by *sharing ownership* rather than by borrowing.
- `item` holds the *current* key/value as owned `Bytes` so `key()`/`value()` can
  return a borrow into stable storage without touching the live cursor.
- `next()` (`with_iter_mut`) advances the real skipmap cursor, snapshots the new
  entry into `item` (`with_mut`). `scan()` calls `next()` once at build time to
  prime `item` with the first element.
- `#[not_covariant]` is a variance restriction ouroboros requires here; if asked,
  ground it as "the compiler may not silently shrink/extend the `'this` tag on
  the stored borrow."

## 5. MergeIterator — k-way merge via a binary heap (week 1 day 4)

`mini-lsm/src/iterators/merge_iterator.rs`

- Shape: `MergeIterator<I> { iters: BinaryHeap<HeapWrapper<I>>, current: Option<HeapWrapper<I>> }`.
- `HeapWrapper<I>(usize /*index*/, Box<I>)`. Ordering is `key().cmp(...)` then
  `index.cmp(...)`, all `.reverse()`d — because Rust's `BinaryHeap` is a
  **max**-heap and we want the **smallest** key (and smallest index on ties) on
  top. Smaller index = higher priority = newer data.
- `current` holds the iterator currently positioned at the output key; the heap
  holds the rest.
- The impl block bound is the puzzle worth knowing:

  ```rust
  impl<I: 'static + for<'a> StorageIterator<KeyType<'a> = KeySlice<'a>>>
      StorageIterator for MergeIterator<I> { ... }
  ```

  - `I: 'static` — the concrete type in the slot must not hold any short-lived
    borrow; it owns or `Arc`-shares its data. (All four leaf iterators qualify.)
  - `for<'a> StorageIterator<KeyType<'a> = KeySlice<'a>>` is a **higher-ranked
    trait bound**: read it as "for *every* lifetime `'a`, this type's `KeyType`
    is `KeySlice<'a>`." Grounding: the merge logic compares keys borrowed for
    arbitrary, caller-chosen durations, so the constraint must hold for all
    lease lengths, not one fixed `'a`. It also pins `KeyType` to specifically
    `KeySlice` so the heap can compare keys across the sub-iterators.
- `next()` logic: pop from the heap every sibling whose key *equals* the current
  key (advancing each; drop the ones that error or go invalid) — this is the
  dedup that makes "smallest index wins." Then advance `current`. If `current`
  went invalid, replace it with the heap top. Otherwise, if `current` now sorts
  after the heap top, `swap` them so `current` again holds the minimum.
- `create()` handles the all-empty and all-invalid edge cases (keeps one invalid
  iterator as `current` so the struct stays well-formed).

## 6. TwoMergeIterator — merge two *different* types (week 2 day 1)

`mini-lsm/src/iterators/two_merge_iterator.rs`

- Shape: `TwoMergeIterator<A, B> { a: A, b: B, choose_a: bool }`.
- Why a separate type from `MergeIterator`: `MergeIterator<I>` merges N
  iterators of the *same* `I`. `TwoMergeIterator` merges exactly two iterators
  of *possibly different* concrete types A and B — that's how the stack glues a
  `MergeIterator<MemTableIterator>` to a `MergeIterator<SsTableIterator>`.
- Bound:
  `B: 'static + for<'a> StorageIterator<KeyType<'a> = A::KeyType<'a>>` — B's key
  type must match A's at every lifetime, so the two are comparable.
- Rule: on equal keys prefer **A**; `skip_b()` advances B past any key equal to
  A's so the same key is never emitted twice. `choose_a` is recomputed after
  every move (`a.key() < b.key()`, with invalid-iterator fallbacks).

## 7. SstConcatIterator — concatenate sorted, non-overlapping SSTs (week 2 day 2)

`mini-lsm/src/iterators/concat_iterator.rs`

- For L1+ levels where SSTs are globally sorted and key ranges don't overlap, so
  at most one SST is "current" at a time — cheaper than heap-merging them.
- Shape: `{ current: Option<SsTableIterator>, next_sst_idx: usize, sstables: Vec<Arc<SsTable>> }`.
- Lazily opens the next SST only when the current one drains
  (`move_until_valid`), to avoid the seek cost of opening all of them up front.
- `create_and_seek_to_key` uses `partition_point` (binary search) over
  `first_key()` to jump to the right SST.
- `check_sst_valid` asserts the non-overlap / sorted invariant in debug.

## 8. LsmIterator + FusedIterator — the top (week 2 day 1)

`mini-lsm/src/lsm_iterator.rs`

- `LsmIterator` wraps `LsmIteratorInner` and adds two behaviors:
  1. **Delete tombstones**: a key with an *empty value* means deleted;
     `move_to_non_delete()` skips those so `scan` users never see them.
  2. **Upper bound**: `end_bound: Bound<Bytes>` stops iteration at the scan's
     upper limit (`Included`/`Excluded`/`Unbounded`).
  - Its `KeyType<'a> = &'a [u8]` (raw bytes, not `KeySlice`) — it unwraps the key
    via `inner.key().raw_ref()` because it is the boundary to the public API.
- `FusedIterator<I>` is a safety shell: once the inner iterator errors or goes
  invalid, it refuses further `next()`/`key()`/`value()` (panics or returns the
  tainted error) so a misbehaving caller can't poke a dead iterator. `scan()`
  returns `FusedIterator<LsmIterator>`.

## 9. Where it all gets wired together

`mini-lsm/src/lsm_storage.rs`

- `LsmStorageInner::scan(lower, upper)` (~L732) is the constructor for the whole
  stack: snapshot state under the read lock, build `MergeIterator` over memtable
  scans, build `MergeIterator` over L0 `SsTableIterator`s, build
  `MergeIterator<SstConcatIterator>` over the levels, then fold them with
  `TwoMergeIterator::create`, wrap in `LsmIterator::new`, then `FusedIterator`.
- `LsmStorageInner::get` (~L500) builds a similar but seek-to-key stack to fetch
  a single key (point lookup) and reads the top entry.
- `MemTable::scan` (`mem_table.rs` ~L137) builds the `MemTableIterator` and
  primes it with one `next()`.

## 10. Quick "where do I look" index

| Question about... | Read |
|---|---|
| the trait / `KeyType` GAT | `src/iterators.rs` |
| key types & lifetimes | `src/key.rs` |
| self-referential iterator / ouroboros / `'this` | `src/mem_table.rs` |
| heap k-way merge, `for<'a>` HRTB, dedup-by-index | `src/iterators/merge_iterator.rs` |
| merging two different iterator types, prefer-A | `src/iterators/two_merge_iterator.rs` |
| non-overlapping level scan, lazy SST open | `src/iterators/concat_iterator.rs` |
| block decode, prefix compression, value range | `src/block/iterator.rs` |
| SST -> block iteration | `src/table/iterator.rs` |
| tombstones, end bound, fuse | `src/lsm_iterator.rs` |
| assembling the full stack in scan/get | `src/lsm_storage.rs` |

Replace `src/` with `mini-lsm/src/` (answer key) or `mini-lsm-starter/src/`
(worksheet, bodies are `unimplemented!()`).
