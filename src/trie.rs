use hashbrown::HashTable;

pub(crate) unsafe fn debug_unreachable() -> ! {
    #[cfg(debug_assertions)]
    unreachable!();

    #[cfg(not(debug_assertions))]
    unsafe {
        core::hint::unreachable_unchecked()
    };
}

pub(crate) struct HashTableTrie<T> {
    // invariant: map.len() is always 1 less than a power of two
    map: Vec<Option<HashTable<T>>>,
    cached: HashTable<T>,
    tables: usize,
}

/// top 7 bits for hashbrown
/// spare
/// bottom 7 bits for hashbrown (with 1024 entries max)
/// we shall use `hash >> 7` as our search key, using the bottom bits.
const SHIFT: u32 = 7;

const CAP: usize = 1024 * 7 / 8;

impl<T> HashTableTrie<T> {
    pub(crate) const fn new() -> Self {
        Self {
            map: Vec::new(),
            cached: HashTable::new(),
            tables: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn trie_len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    pub(crate) fn tables(&self) -> usize {
        self.tables
    }

    pub fn allocation_size(&self) -> usize {
        self.iter().map(|t| t.allocation_size()).sum::<usize>() + core::mem::size_of_val(&*self.map)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &HashTable<T>> {
        self.map.iter().filter_map(|x| x.as_ref())
    }

    pub(crate) fn iter_mut(&mut self) -> TableIterMut<T> {
        TableIterMut {
            slice: self.map.iter_mut(),
        }
    }

    fn search_hash(&self, hash: u64) -> (usize, usize) {
        let key = hash >> SHIFT;
        self.search(key as usize)
    }

    // returns key mask and index
    fn search(&self, key: usize) -> (usize, usize) {
        // invariant 0: map.len() is always 1 less than a power of two
        debug_assert!((self.map.len() + 1).is_power_of_two());

        // invariant 1: offset is always 1 less than a power of two
        // invariant 2: offset is always less than or equal to usize::MAX
        let mut offset = 0;
        loop {
            debug_assert!(offset < self.map.len());

            // since offset and map.len() are 1 less thw a power of two, it follows that
            // invariant 3: offset * 2 < self.map.len()

            // since map.len() <= isize::MAX, it follows that
            // invariant 4: offset * 2 < isize::MAX

            // invariant 5: key <= offset
            let key = key & offset;
            // invariant 6: index <= offset * 2 < self.map.len()
            // this cannot overflow given invariant 4
            let index = offset + key;

            let mask = offset;
            // this preserves invariant 1
            offset = offset * 2 + 1;

            // Safety: invariants 6 ensures that
            // index <= offset * 2 < self.map.len()
            // therefore it is inbounds.
            if unsafe { self.map.get_unchecked(index) }.is_some() {
                return (mask, index);
            }
        }
    }

    pub(crate) fn get_table<'a>(&'a self, hash: u64) -> &'a HashTable<T> {
        if self.tables == 0 {
            return const { &HashTable::new() };
        }

        let (_, index) = self.search_hash(hash);
        self.map[index].as_ref().unwrap()
    }

    pub(crate) fn get_table_mut<'a>(&'a mut self, hash: u64) -> &'a mut HashTable<T> {
        if self.tables == 0 {
            self.map.push(Some(HashTable::new()));
            self.tables += 1;
        }

        let (_, index) = self.search_hash(hash);
        self.map[index].as_mut().unwrap()
    }

    /// Gets the hashtable corresponding with the hash.
    ///
    /// # Safety
    /// Caller must only insert 1 item.
    ///
    /// Guarantees there is capacity in the hashtable.
    pub(crate) unsafe fn get_table_for_insert<'a>(
        &'a mut self,
        hash: u64,
        hasher: impl Fn(&T) -> u64,
    ) -> &'a mut HashTable<T> {
        if self.tables == 0 {
            self.map.push(Some(HashTable::new()));
            self.tables += 1;
        }

        use polonius_the_crab::{polonius, polonius_return};
        let mut this = self;

        loop {
            let (mask, index) = this.search_hash(hash);

            polonius!(|this| -> &'polonius mut HashTable<T> {
                let table = this.map[index].as_mut().unwrap();

                let mut cap = table.capacity();
                let growth_left = cap - table.len();

                // inserting will not cause a realloc beyond CAP.
                if growth_left >= 1 || cap <= CAP / 2 {
                    if growth_left < 1 {
                        table.reserve(1, &hasher);
                        cap = table.capacity();
                        debug_assert!(cap <= CAP, "new capacity too large {cap}");
                    }

                    polonius_return!(table);
                }
            });

            // need to split.
            this.split(mask, index, &hasher);
        }
    }

    #[cold]
    #[inline(never)]
    fn split(&mut self, mask: usize, index: usize, hasher: impl Fn(&T) -> u64) {
        let next_mask = mask * 2 + 1;
        let next_len = next_mask * 2 + 1;

        let next0 = next_mask + index - mask;
        let next1 = next_mask + index + 1;
        debug_assert_ne!(next0, next1);

        if next_len > self.map.len() {
            self.map.resize_with(next_len, || None);
        }

        let mut left = HashTable::with_capacity(CAP);
        if self.cached.capacity() == 0 {
            let prev_cached = core::mem::replace(&mut self.cached, HashTable::with_capacity(CAP));
            // it's empty and has no capacity, don't include drop glue.
            core::mem::forget(prev_cached);
        }
        let mut right = core::mem::take(&mut self.cached);

        // Safety: for next0 to equal next1, we must have mask = usize::MAX.
        // mask is always < map.len().
        // map.len() <= isize::MAX, therefore they are not equal.
        // Safety: for next0 to equal index, we must have next_mask = mask, which is impossible.
        // Safety: for next1 to equal index, we must have next_mask = usize::MAX, which is impossible.
        let [table_slot, slot0, slot1] =
            unsafe { self.map.get_disjoint_unchecked_mut([index, next0, next1]) };

        // Safety: caller will insure that index is in the map.
        // We don't take here, because we need to preserve the table invariant if hasher panics.
        let table = unsafe { table_slot.as_mut().unwrap_unchecked() };

        let next_mask = mask * 2 + 1;
        for e in table.drain() {
            let hash = hasher(&e);
            let key = (hash >> SHIFT) as usize & next_mask;
            let t = if key <= mask { &mut left } else { &mut right };

            // Safety: table is guaranteed to have a length <= CAP.
            // Worst case, all entries go to the same table, but these are pre-allocated with enough capacity.
            t.insert_unique(hash, e, |_| unsafe { debug_unreachable() });
        }

        // Safety: same as above.
        let table = unsafe { table_slot.take().unwrap_unchecked() };
        core::mem::forget(core::mem::replace(&mut self.cached, table));
        core::mem::forget(core::mem::replace(slot0, Some(left)));
        core::mem::forget(core::mem::replace(slot1, Some(right)));
        self.tables += 1;
    }
}

pub(crate) struct TableIterMut<'a, T> {
    slice: core::slice::IterMut<'a, Option<HashTable<T>>>,
}

impl<'a, T> Iterator for TableIterMut<'a, T> {
    type Item = &'a mut HashTable<T>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(next) = self.slice.next()? {
                break Some(next);
            }
        }
    }
}
