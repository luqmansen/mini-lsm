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

use std::vec;

use crate::key::{KeySlice, KeyVec};
use bytes::BufMut;

use super::Block;

/// Builds a block.
pub struct BlockBuilder {
    /// Offsets of each key-value entries.
    offsets: Vec<u16>,
    /// All serialized key-value pairs in the block.
    data: Vec<u8>,
    /// The expected block size.
    block_size: usize,
    /// The first key in the block
    first_key: KeyVec,
}

impl BlockBuilder {
    /// Creates a new block builder.
    pub fn new(block_size: usize) -> Self {
        Self {
            offsets: Vec::<u16>::new(),
            data: Vec::<u8>::new(),
            block_size,
            first_key: KeyVec::new(),
        }
    }

    /// Adds a key-value pair to the block. Returns false when the block is full.
    /// You may find the `bytes::BufMut` trait useful for manipulating binary data.
    #[must_use]
    pub fn add(&mut self, key: KeySlice, value: &[u8]) -> bool {
        let current_len = self.data.len() + (self.offsets.len() * 2);
        let upcoming_data_len = key.len() + value.len();
        let upcoming_offset_kv_len = 4;
        let item_cnt_len = 2;

        let is_oversize = current_len + upcoming_data_len + upcoming_offset_kv_len + item_cnt_len
            > self.block_size;

        // first key allowed to be oversize. the entire block belongs to it
        if is_oversize && !self.first_key.is_empty() {
            return false;
        }

        let pre = self.data.len();
        let mut _key = key.clone();
        let mut buff = vec![];

        if self.first_key.is_empty() {
            self.first_key = key.to_key_vec()
        } else {
            // prefix compression
            // key_overlap_len (u16) | rest_key_len (u16) | key (rest_key_len)

            let mut key_overlap_len = 0;
            let mut rest_key_len = key.len() as u16;

            let st = self.first_key.raw_ref();
            let nd = key.raw_ref();
            let mut rest_key = key.raw_ref();

            for idx in 0..st.len() {
                if idx < nd.len() {
                    if st[idx] != nd[idx] {
                        break;
                    }
                    key_overlap_len += 1;
                    rest_key_len -= 1;
                    rest_key = &rest_key[1..]
                }
            }

            buff.put_u16(key_overlap_len);
            buff.put_u16(rest_key_len);
            buff.put_slice(rest_key);

            _key = KeySlice::from_slice(&buff);
        }

        self.data.put_u16(_key.len() as u16);
        self.data.extend_from_slice(_key.raw_ref());
        self.data.put_u16(value.len() as u16);
        self.data.extend_from_slice(value);
        self.offsets.push(pre as u16);

        true
    }

    /// Check if there is no key-value pair in the block.
    pub fn is_empty(&self) -> bool {
        self.first_key.is_empty()
    }

    /// Finalize the block.
    pub fn build(self) -> Block {
        Block {
            data: self.data,
            offsets: self.offsets,
        }
    }
}
