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

use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;
use std::{io::Write, mem};

use anyhow::Result;
use bytes::{BufMut, Bytes};

use super::{BlockMeta, SsTable};
use crate::{
    block::BlockBuilder,
    key::{Key, KeySlice},
    lsm_storage::BlockCache,
    table::FileObject,
};

/// Builds an SSTable from key-value pairs.
pub struct SsTableBuilder {
    builder: BlockBuilder,
    first_key: Vec<u8>,
    last_key: Vec<u8>,
    data: Vec<u8>,
    pub(crate) meta: Vec<BlockMeta>,
    block_size: usize,
}

impl SsTableBuilder {
    /// Create a builder based on target block size.
    pub fn new(block_size: usize) -> Self {
        let builder = BlockBuilder::new(block_size);
        Self {
            block_size,
            builder,
            first_key: vec![],
            last_key: vec![],
            data: vec![],
            meta: vec![],
        }
    }

    /// Adds a key-value pair to SSTable.
    ///
    /// Note: You should split a new block when the current block is full.(`std::mem::replace` may
    /// be helpful here)
    pub fn add(&mut self, key: KeySlice, value: &[u8]) {
        if self.first_key.is_empty() {
            self.first_key = key.into_inner().to_vec();
        }

        self.last_key = key.into_inner().to_vec();

        let is_full = self.builder.add(key, value);

        let first_key = Key::from_bytes(Bytes::copy_from_slice(key.raw_ref()));
        let last_key = Key::from_bytes(Bytes::copy_from_slice(key.raw_ref()));
        match &mut self.meta.last_mut() {
            Some(meta) => {
                // if it's full, meaning that last meta is owned by prev block
                if !is_full {
                    if meta.first_key.is_empty() {
                        meta.first_key = first_key;
                    }
                    meta.last_key = last_key;
                } else {
                    self.meta.push(BlockMeta {
                        offset: 0,
                        first_key,
                        last_key,
                    });
                }
            }
            None => {
                self.meta.push(BlockMeta {
                    offset: 0,
                    first_key,
                    last_key,
                });
            }
        }

        if !is_full {
            return;
        }
        // dbg!("block is full!");

        let new_builder = BlockBuilder::new(self.block_size);
        let old_builder = mem::replace(&mut self.builder, new_builder);

        let old_block = old_builder.build();
        let block_bytes = old_block.encode();

        self.meta.last_mut().map(|m| m.offset = self.data.len());
        self.data.extend_from_slice(block_bytes.as_ref());

        return;
    }

    /// Get the estimated size of the SSTable.
    ///
    /// Since the data blocks contain much more data than meta blocks, just return the size of data
    /// blocks here.
    pub fn estimated_size(&self) -> usize {
        unimplemented!()
    }

    /// Builds the SSTable and writes it to the given path. Use the `FileObject` structure to manipulate the disk objects.
    pub fn build(
        #[allow(unused_mut)] mut self,
        id: usize,
        block_cache: Option<Arc<BlockCache>>,
        path: impl AsRef<Path>,
    ) -> Result<SsTable> {
        /*
        * -------------------------------------------------------------------------------------------
        |         Block Section         |          Meta Section         |          Extra          |
        -------------------------------------------------------------------------------------------
        | data block | ... | data block |            metadata           | meta block offset (u32) |
        -------------------------------------------------------------------------------------------
        */

        // idk whether i can append to self.data in-place or i need to allocate in-case self.data
        // is still used downstream
        let mut data_to_write = vec![];
        data_to_write.extend_from_slice(self.data.as_ref());

        let block_meta_offset = self.data.len();
        BlockMeta::encode_block_meta(&self.meta, &mut data_to_write);

        data_to_write.put_u32(block_meta_offset as u32);

        let file_obj = FileObject::create(path.as_ref(), data_to_write)?;

        let sst = SsTable {
            file: file_obj,
            block_meta: self.meta,
            block_meta_offset,
            id: id,
            block_cache,
            first_key: Key::from_bytes(Bytes::from_owner(self.first_key)),
            last_key: Key::from_bytes(Bytes::from_owner(self.last_key)),
            bloom: None,
            max_ts: 0,
        };

        Ok(sst)
    }

    #[cfg(test)]
    pub(crate) fn build_for_test(self, path: impl AsRef<Path>) -> Result<SsTable> {
        self.build(0, None, path)
    }
}
