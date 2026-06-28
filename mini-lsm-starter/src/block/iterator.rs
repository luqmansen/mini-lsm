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

use std::sync::Arc;

use crate::key::{Key, KeySlice, KeyVec};

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
        let first_offset_idx = block.offsets.get(0).unwrap();
        let second_offset_idx = block.offsets.get(1).unwrap();

        let first_entry = &block.data[*first_offset_idx as usize..*second_offset_idx as usize];

        // dbg!(first_entry);
        // dbg!(&first_entry[0..2]);

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

        // dbg!(value_len);
        // dbg!(
        //     value_len_start_offset,
        //     value_len_start_offset + value_len as usize
        // );
        // let value = &first_entry[value_offset_start..value_offset_start + value_len as usize];
        // dbg!(str::from_utf8(value).unwrap());

        Self {
            block: Arc::clone(&block),
            key: KeyVec::from_vec(key.to_vec()),
            value_range: (value_offset_start, value_offset_start + value_len as usize),
            idx: 0,
            first_key: KeyVec::from_vec(key.to_vec()),
        }
    }

    /// Creates a block iterator and seek to the first key that >= `key`.
    pub fn create_and_seek_to_key(block: Arc<Block>, key: KeySlice) -> Self {
        let first_offset_idx = block.offsets.get(0).unwrap();
        let second_offset_idx = block.offsets.get(1).unwrap();

        let mut value_start_offset: usize;
        let mut value_start_offset: usize;
        let mut value_len: usize;
        let mut current_key: KeySlice;

        let mut first_key = Key::new();
        let loop_cnt = 1;

        loop {
            dbg!("loop_count {:}\n", loop_cnt);

            let current_entry =
                &block.data[*first_offset_idx as usize..*second_offset_idx as usize];

            let key_len = usize::from_be_bytes(block.data[0..1].try_into().unwrap());
            current_key = KeySlice::from_slice(&block.data[2..key_len]);

            if first_key.is_empty() {
                first_key = current_key.to_key_vec();
            }

            value_start_offset = key_len + current_key.len();
            value_len = usize::from_be_bytes(
                block.data[value_start_offset..value_start_offset + 1]
                    .try_into()
                    .unwrap(),
            );

            if current_key >= key {
                break;
            }
        }

        Self {
            block: Arc::clone(&block),
            key: current_key.to_key_vec(),
            value_range: (value_start_offset, value_start_offset + value_len),
            idx: 0,
            first_key: current_key.to_key_vec(),
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
        unimplemented!()
    }

    /// Seeks to the first key in the block.
    pub fn seek_to_first(&mut self) {
        let block = self.block.clone();

        let first_offset_idx = block.offsets.get(0).unwrap();
        let second_offset_idx = block.offsets.get(1).unwrap();

        let first_entry = &block.data[*first_offset_idx as usize..*second_offset_idx as usize];

        // dbg!(first_entry);
        // dbg!(&first_entry[0..2]);

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

        // dbg!(value_len);
        // dbg!(
        //     value_len_start_offset,
        //     value_len_start_offset + value_len as usize
        // );
        // let value = &first_entry[value_offset_start..value_offset_start + value_len as usize];
        // dbg!(str::from_utf8(value).unwrap());

        self.key = KeyVec::from_vec(key.to_vec());
        self.value_range = (value_offset_start, value_offset_start + value_len as usize);
    }

    /// Move to the next key in the block.
    pub fn next(&mut self) {
        let next_key_len_offset = self.value_range.1;
        if next_key_len_offset + 2 > self.block.data.len() {
            return;
        }

        let next_key_len = u16::from_be_bytes(
            self.block.data[next_key_len_offset..next_key_len_offset + 2]
                .try_into()
                .unwrap(),
        );

        let next_key = &self.block.data
            [next_key_len_offset + 2..next_key_len_offset + 2 + next_key_len as usize];

        let next_value_len_offset = next_key_len_offset + 2 + next_key_len as usize;

        let next_value_len = u16::from_be_bytes(
            self.block.data[next_value_len_offset..next_value_len_offset + 2]
                .try_into()
                .unwrap(),
        ) as usize;

        // let next_value = &self.block.data
        //     [next_value_len_offset + 2 + next_value_len_offset + 2 + next_value_len];

        self.key = KeyVec::from_vec(next_key.to_vec());
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
        unimplemented!()
    }
}
