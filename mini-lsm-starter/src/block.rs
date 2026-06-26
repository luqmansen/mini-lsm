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

mod builder;
mod iterator;

pub use builder::BlockBuilder;
use bytes::{BufMut, Bytes, BytesMut};
pub use iterator::BlockIterator;

/// A block is the smallest unit of read and caching in LSM tree. It is a collection of sorted key-value pairs.
#[derive(Debug)]
pub struct Block {
    pub(crate) data: Vec<u8>,
    pub(crate) offsets: Vec<u16>,
}

impl Block {
    /// Encode the internal data to the data layout illustrated in the course
    /// Note: You may want to recheck if any of the expected field is missing from your output
    pub fn encode(&self) -> Bytes {
        let mut b = BytesMut::new();

        b.put_slice(self.data.as_ref());
        for &u in &self.offsets {
            b.put_u16(u);
        }

        // note to self: usize is 8 bytes, u16 is 2 bytes.
        let cnt = self.offsets.len();
        b.put_u16(cnt as u16);

        b.freeze()
    }

    // Decode from the data layout, transform the input `data` to a single `Block`
    pub fn decode(raw_data_bytes: &[u8]) -> Self {
        use bytes::Buf;

        let item_cnt = u16::from_be_bytes(
            raw_data_bytes[raw_data_bytes.len() - 2..raw_data_bytes.len()]
                .try_into()
                .unwrap(),
        );

        // dbg!(item_cnt);

        let (data, mut offsets_u8) =
            raw_data_bytes.split_at(raw_data_bytes.len() - ((item_cnt * 2) as usize + 2));

        let mut offsets = Vec::new();
        while offsets_u8.has_remaining() {
            offsets.push(offsets_u8.get_u16());
        }

        offsets.pop();

        let mut data = Vec::<u8>::new();
        let mut last_idx: usize = raw_data_bytes.len() - 2 - offsets.len() * 2;
        // dbg!(last_idx);
        // dbg!(offsets.iter().rev());

        for offset in offsets.iter().rev() {
            // dbg!([*offset as usize..last_idx]);
            let d = raw_data_bytes.get(*offset as usize..last_idx).unwrap();
            data.splice(0..0, d.iter().copied());
            last_idx -= last_idx - *offset as usize;
        }

        Self {
            data,
            offsets: offsets,
        }
    }
}
