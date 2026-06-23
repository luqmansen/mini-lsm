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

use std::cmp::{self};
use std::collections::BinaryHeap;
use std::collections::binary_heap::PeekMut;

use anyhow::{Ok, Result};

use crate::key::{Key, KeySlice};

use super::StorageIterator;

struct HeapWrapper<I: StorageIterator>(pub usize, pub Box<I>);

impl<I: StorageIterator> PartialEq for HeapWrapper<I> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == cmp::Ordering::Equal
    }
}

impl<I: StorageIterator> Eq for HeapWrapper<I> {}

impl<I: StorageIterator> PartialOrd for HeapWrapper<I> {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<I: StorageIterator> Ord for HeapWrapper<I> {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.1
            .key()
            .cmp(&other.1.key())
            .then(self.0.cmp(&other.0))
            .reverse()
    }
}

/// Merge multiple iterators of the same type. If the same key occurs multiple times in some
/// iterators, prefer the one with smaller index.
pub struct MergeIterator<I: StorageIterator> {
    iter_heap: BinaryHeap<HeapWrapper<I>>,
    current_iterator: Option<HeapWrapper<I>>,
}

impl<I: StorageIterator> MergeIterator<I> {
    pub fn create(iters: Vec<Box<I>>) -> Self {
        let mut bh: BinaryHeap<HeapWrapper<I>> = BinaryHeap::new();

        for (index, iter) in iters.into_iter().enumerate() {
            if iter.is_valid() {
                bh.push(HeapWrapper(index, iter));
            }
        }
        let current = bh.pop();

        Self {
            iter_heap: bh,
            current_iterator: top,
        }
    }
}
impl<I: 'static + for<'a> StorageIterator<KeyType<'a> = KeySlice<'a>>> StorageIterator
    for MergeIterator<I>
{
    type KeyType<'a> = KeySlice<'a>;

    fn key(&self) -> KeySlice<'_> {
        self.current_iterator.as_ref().unwrap().1.key()
    }

    fn value(&self) -> &[u8] {
        self.current_iterator.as_ref().unwrap().1.value()
    }

    fn is_valid(&self) -> bool {
        self.current_iterator.is_some()
    }

    fn next(&mut self) -> Result<()> {
        let mut current_iterator = self.current_iterator.take().unwrap();
        let current_key = current_iterator.1.key();

        while let Some(mut top) = self.iter_heap.peek_mut() {
            let matches = { top.1.as_ref().key() == current_key };
            if matches {
                let res = top.1.next();
                if res.is_err() {
                    PeekMut::pop(top);
                    return res;
                }

                if !top.1.is_valid() {
                    PeekMut::pop(top);
                }
            }
        }
        if current_iterator.1.next().is_ok() && current_iterator.1.is_valid() {
            self.iter_heap.push(current_iterator);
        }

        if let Some(next_iter) = self.iter_heap.pop() {
            self.current_iterator = Some(next_iter);
        }

        Ok(())
    }
}
