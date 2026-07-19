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

use bytes::Buf;

use std::sync::Arc;

use crate::key::{KeySlice, KeyVec};

use super::Block;

/// Iterates on a block.
pub struct BlockIterator {
    /// The internal `Block`, wrapped by an `Arc`
    block: Arc<Block>,
    /// The current key, empty represents the iterator is invalid
    key: KeyVec,
    /// the current value range in the block.data, corresponds to the current key
    value_range: (usize, usize),
    /// Current index of the key-value pair, should be in range of [0, num_of_elements)
    idx: usize,
    /// The first key in the block
    first_key: KeyVec,
}

impl BlockIterator {
    fn new(block: Arc<Block>) -> Self {
        Self {
            block,
            key: KeyVec::new(),
            value_range: (0, 0),
            idx: 0,
            first_key: KeyVec::new(),
        }
    }

    /// Creates a block iterator and seek to the first entry.
    pub fn create_and_seek_to_first(block: Arc<Block>) -> Self {
        let mut data = block.data.as_slice();

        let key_len = data.get_u16();
        let key = data.copy_to_bytes(key_len as usize);
        let value_len = data.get_u16();

        Self {
            block: Arc::clone(&block),
            key: KeyVec::from_vec(key.to_vec()),
            value_range: (
                (2 + key_len + 2) as usize,
                (2 + key_len + 2 + value_len) as usize,
            ),
            idx: 0,
            first_key: KeyVec::from_vec(key.to_vec()),
        }
    }

    /// Creates a block iterator and seek to the first key that >= `key`.
    pub fn create_and_seek_to_key(block: Arc<Block>, key: KeySlice) -> Self {
        let data = &mut block.data.as_slice();

        let mut curr_value_len = 0;
        let initial_len = data.as_ref().len();

        let mut first_key = KeyVec::new();
        let mut current_keyvec = KeyVec::new();

        // refactor with more idiomatic buffer handling
        while data.has_remaining() {
            // REMINDER
            // -----------------------------------------------------------------------
            // |                           Entry #1                            | ... |
            // -----------------------------------------------------------------------
            // | key_len (2B) | key (keylen) | value_len (2B) | value (varlen) | ... |
            // -----------------------------------------------------------------------
            //
            //
            // key with prefix compression shaped like this
            // | key_overlap_len (u16) | rest_key_len (u16) | key (rest_key_len) | ....

            let key_len = data.get_u16();
            let mut key_section_bytes = data.copy_to_bytes(key_len as usize);
            let value_len = data.get_u16();

            curr_value_len = value_len;

            current_keyvec = KeyVec::from_vec(key_section_bytes.to_vec());

            if first_key.is_empty() {
                first_key = current_keyvec.clone();
            } else {
                // decode the prefix
                let key_overlap_len = key_section_bytes.get_u16();
                let rest_key_len = key_section_bytes.get_u16();
                let rest_key = key_section_bytes.copy_to_bytes(rest_key_len as usize);
                // current_keyvec =
                let mut full_key = first_key.raw_ref()[0..key_overlap_len as usize].to_vec();
                full_key.extend_from_slice(rest_key.as_ref());

                current_keyvec = KeyVec::from_vec(full_key);
            }

            if current_keyvec.as_key_slice() >= key {
                break;
            }
        }

        let current_pos = initial_len - data.remaining();
        let value_range = (current_pos, current_pos + curr_value_len as usize);

        // dbg!(value_range);

        Self {
            block: Arc::clone(&block),
            value_range,
            key: current_keyvec.clone(),
            idx: 0,
            first_key: first_key.clone(),
        }
    }

    /// Returns the key of the current entry.
    pub fn key(&self) -> KeySlice<'_> {
        self.key.as_key_slice()
    }

    /// Returns the value of the current entry.
    pub fn value(&self) -> &[u8] {
        &self.block.data[self.value_range.0..self.value_range.1]
    }

    /// Returns true if the iterator is valid.
    /// Note: You may want to make use of `key`
    pub fn is_valid(&self) -> bool {
        !self.key.is_empty()
    }

    /// Seeks to the first key in the block.
    pub fn seek_to_first(&mut self) {
        let block = self.block.clone();

        let first_offset_idx = block.offsets.first().unwrap();
        let second_offset_idx = block.offsets.get(1).unwrap();

        let first_entry = &block.data[*first_offset_idx as usize..*second_offset_idx as usize];

        let key_len = u16::from_be_bytes(first_entry[0..2].try_into().unwrap());
        let key = &first_entry[2..2 + key_len as usize];
        dbg!(&key);

        let value_len_start_offset = 2 + key.len(); // 2 key len bytes + the actual key length
        let value_len = u16::from_be_bytes(
            first_entry[value_len_start_offset..value_len_start_offset + 2]
                .try_into()
                .unwrap(),
        );
        let value_offset_start = value_len_start_offset + 2;

        self.key = KeyVec::from_vec(key.to_vec());
        self.value_range = (value_offset_start, value_offset_start + value_len as usize);
    }

    /// Move to the next key in the block.
    pub fn next(&mut self) {
        let next_key_len_offset = self.value_range.1;

        // out ouf offset, return immediately
        if next_key_len_offset + 2 > self.block.data.len() {
            self.key = KeyVec::new();
            return;
        }

        let next_key_len = u16::from_be_bytes(
            self.block.data[next_key_len_offset..next_key_len_offset + 2]
                .try_into()
                .unwrap(),
        );

        let mut next_compressed_key_block = &self.block.data
            [next_key_len_offset + 2..next_key_len_offset + 2 + next_key_len as usize];

        let next_value_len_offset = next_key_len_offset + 2 + next_key_len as usize;

        let next_value_len = u16::from_be_bytes(
            self.block.data[next_value_len_offset..next_value_len_offset + 2]
                .try_into()
                .unwrap(),
        ) as usize;

        // decode the prefix
        let key_overlap_len = next_compressed_key_block.get_u16();
        let rest_key_len = next_compressed_key_block.get_u16();
        let rest_key = next_compressed_key_block.copy_to_bytes(rest_key_len as usize);
        // current_keyvec =
        let mut full_key = self.first_key.raw_ref()[0..key_overlap_len as usize].to_vec();
        full_key.extend_from_slice(rest_key.as_ref());

        self.key = KeyVec::from_vec(full_key);
        self.idx += 1;
        self.value_range = (
            next_value_len_offset + 2,
            next_value_len_offset + 2 + next_value_len,
        )
    }

    /// Seek to the first key that >= `key`.
    /// Note: You should assume the key-value pairs in the block are sorted when being added by
    /// callers.
    pub fn seek_to_key(&mut self, key: KeySlice) {
        let data = &mut self.block.data.as_slice();
        let initial_len = data.as_ref().len();
        let mut current_keyvec = KeyVec::new();
        let mut value_len: u16 = 0; // value can be 0 lenght anyway
        let mut current_pos: usize = 0;

        while data.has_remaining() {
            // REMINDER
            // -----------------------------------------------------------------------
            // |                           Entry #1                            | ... |
            // -----------------------------------------------------------------------
            // | key_len (2B) | key (keylen) | value_len (2B) | value (varlen) | ... |
            // -----------------------------------------------------------------------
            //
            //
            // key with prefix compression shaped like this
            // | key_overlap_len (u16) | rest_key_len (u16) | key (rest_key_len) | ....

            let key_len = data.get_u16();
            let mut key_section_bytes = data.copy_to_bytes(key_len as usize);
            value_len = data.get_u16();
            data.advance(value_len as usize);

            current_keyvec = KeyVec::from_vec(key_section_bytes.to_vec());
            current_pos = initial_len - data.remaining();

            if self.first_key != current_keyvec {
                let key_overlap_len = key_section_bytes.get_u16();
                let rest_key_len = key_section_bytes.get_u16();
                let rest_key = key_section_bytes.copy_to_bytes(rest_key_len as usize);
                let mut full_key = self.first_key.raw_ref()[0..key_overlap_len as usize].to_vec();
                full_key.extend_from_slice(rest_key.as_ref());

                current_keyvec = KeyVec::from_vec(full_key);
            }

            self.idx += 1;

            if current_keyvec.as_key_slice() >= key {
                break;
            }
        }

        self.key = current_keyvec;
        self.value_range = (current_pos - value_len as usize, current_pos);
    }
}
