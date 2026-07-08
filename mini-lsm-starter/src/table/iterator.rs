// Copyright (c) 2022-2025 Alex Chi Z
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![allow(unused_variables)] // TODO(you): remove this lint after implementing this mod
#![allow(dead_code)] // TODO(you): remove this lint after implementing this mod

use std::{cmp, mem, sync::Arc, usize};

use anyhow::Result;

use super::SsTable;
use crate::{block::BlockIterator, iterators::StorageIterator, key::KeySlice};

/// An iterator over the contents of an SSTable.
pub struct SsTableIterator {
    table: Arc<SsTable>,
    blk_iter: BlockIterator,
    blk_idx: usize,
}

impl SsTableIterator {
    /// Create a new iterator and seek to the first key-value pair in the first data block.
    pub fn create_and_seek_to_first(table: Arc<SsTable>) -> Result<Self> {
        let blk_iter = BlockIterator::create_and_seek_to_first(table.read_block(0).unwrap());

        Ok(Self {
            table,
            blk_iter,
            blk_idx: 0,
        })
    }

    /// Seek to the first key-value pair in the first data block.
    pub fn seek_to_first(&mut self) -> Result<()> {
        let sst_iter = SsTableIterator::create_and_seek_to_first(self.table.clone()).unwrap();

        _ = mem::replace(self, sst_iter);

        Ok(())
    }

    /// Create a new iterator and seek to the first key-value pair which >= `key`.
    pub fn create_and_seek_to_key(table: Arc<SsTable>, key: KeySlice) -> Result<Self> {
        //   1st  ---- last  |  1st ---- last |
        let idx = table.block_meta.binary_search_by(|block_meta| {
            // println!(
            //     "{:},{:},{:}",
            //     String::from_utf8(block_meta.first_key.raw_ref().to_vec()).unwrap(),
            //     String::from_utf8(block_meta.last_key.raw_ref().to_vec()).unwrap(),
            //     String::from_utf8(key.raw_ref().to_vec()).unwrap()
            // );
            // whole block is before the key
            if block_meta.last_key.as_key_slice() < key {
                return cmp::Ordering::Less;
            // block contains key
            } else if key < block_meta.first_key.as_key_slice() {
                return cmp::Ordering::Greater;
            // block after key
            } else if key <= block_meta.last_key.as_key_slice()
                && key >= block_meta.first_key.as_key_slice()
            {
                return cmp::Ordering::Equal;
            } else {
                panic!("impossible!")
            }
        });

        // dbg!(idx);

        // if idx.is_err() {
        //     dbg!(&table.block_meta);
        //     return Err(Error::msg(format!("not found {:}", idx.err().unwrap())));
        // }
        //

        let new_idx = match idx {
            Ok(v) => v,
            Err(v) => {
                if v > table.block_meta.len() - 1 {
                    table.block_meta.len() - 1
                } else {
                    v
                }
            }
        };

        let mut blk_iter =
            BlockIterator::create_and_seek_to_first(table.read_block(new_idx).unwrap());

        blk_iter.seek_to_key(key);

        Ok(Self {
            table,
            blk_iter,
            blk_idx: new_idx,
        })
    }

    /// Seek to the first key-value pair which >= `key`.
    /// Note: You probably want to review the handout for detailed explanation when implementing
    /// this function.
    pub fn seek_to_key(&mut self, key: KeySlice) -> Result<()> {
        let sst_iter = SsTableIterator::create_and_seek_to_key(self.table.clone(), key).unwrap();
        _ = mem::replace(self, sst_iter);

        Ok(())
    }
}

impl StorageIterator for SsTableIterator {
    type KeyType<'a> = KeySlice<'a>;

    /// Return the `key` that's held by the underlying block iterator.
    fn key(&self) -> KeySlice<'_> {
        self.blk_iter.key()
    }

    /// Return the `value` that's held by the underlying block iterator.
    fn value(&self) -> &[u8] {
        self.blk_iter.value()
    }

    /// Return whether the current block iterator is valid or not.
    fn is_valid(&self) -> bool {
        self.blk_iter.is_valid()
    }

    /// Move to the next `key` in the block.
    /// Note: You may want to check if the current block iterator is valid after the move.
    fn next(&mut self) -> Result<()> {
        self.blk_iter.next();
        if self.blk_iter.is_valid() {
            return Ok(());
        }

        if self.blk_idx + 1 > self.table.num_of_blocks() {
            return Ok(());
        }

        let next_block = self.table.read_block(self.blk_idx + 1)?;
        let blk_iter = BlockIterator::create_and_seek_to_first(next_block);

        if !blk_iter.is_valid() {
            return Ok(());
        }

        self.blk_iter = blk_iter;
        self.blk_idx += 1;

        Ok(())
    }
}
