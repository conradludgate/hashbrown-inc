use hashbrown::hash_table;

use crate::trie::HashTableTrie;

pub struct IncHashTable<T> {
    len: usize,
    trie: HashTableTrie<T>,
}

impl<T> IncHashTable<T> {
    pub const fn new() -> Self {
        Self {
            len: 0,
            trie: HashTableTrie::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.trie.iter_mut().for_each(|t| t.clear());
    }

    pub fn shrink_to_fit(&mut self, hasher: impl Fn(&T) -> u64) {
        // TODO: should we merge tables?
        self.trie.iter_mut().for_each(|t| t.shrink_to_fit(&hasher));
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.trie.iter().flat_map(|t| t.iter())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.trie.iter_mut().flat_map(|t| t.iter_mut())
    }

    pub fn find(&self, hash: u64, eq: impl Fn(&T) -> bool) -> Option<&T> {
        let table = self.trie.get_table(hash);
        table.find(hash, eq)
    }

    pub fn retain(&mut self, mut f: impl FnMut(&mut T) -> bool) {
        self.trie.iter_mut().for_each(|t| t.retain(|v| f(v)));
    }

    pub fn find_entry<'a>(
        &'a mut self,
        hash: u64,
        eq: impl Fn(&T) -> bool,
    ) -> Result<OccupiedEntry<'a, T>, AbsentEntry<'a, T>> {
        use polonius_the_crab::{polonius, polonius_return};

        let mut this = self;

        polonius!(
            |this| -> Result<OccupiedEntry<'polonius, T>, AbsentEntry<'a, T>> {
                let table = this.trie.get_table_mut(hash);

                if let Ok(o) = table.find_entry(hash, eq) {
                    polonius_return!(Ok(OccupiedEntry {
                        len: &mut this.len,
                        inner: o,
                    }));
                }
            }
        );

        Err(AbsentEntry { table: this })
    }

    pub fn insert_unique<'a>(
        &'a mut self,
        hash: u64,
        value: T,
        hasher: impl Fn(&T) -> u64,
    ) -> OccupiedEntry<'a, T> {
        let table = self.trie.get_table_for_insert(hash, &hasher);
        let inner = table.insert_unique(hash, value, &hasher);
        OccupiedEntry {
            len: &mut self.len,
            inner,
        }
    }

    pub fn entry<'a>(
        &'a mut self,
        hash: u64,
        eq: impl Fn(&T) -> bool,
        hasher: impl Fn(&T) -> u64,
    ) -> Entry<'a, T> {
        let table = self.trie.get_table_for_insert(hash, &hasher);
        match table.entry(hash, &eq, &hasher) {
            hash_table::Entry::Occupied(o) => Entry::Occupied(OccupiedEntry {
                inner: o,
                len: &mut self.len,
            }),
            hash_table::Entry::Vacant(v) => Entry::Vacant(VacantEntry {
                inner: v,
                len: &mut self.len,
            }),
        }
    }
}

pub enum Entry<'a, T> {
    Occupied(OccupiedEntry<'a, T>),
    Vacant(VacantEntry<'a, T>),
}

impl<'a, T> Entry<'a, T> {
    pub fn insert(self, t: T) -> OccupiedEntry<'a, T> {
        match self {
            Entry::Occupied(mut occupied_entry) => {
                *occupied_entry.get_mut() = t;
                occupied_entry
            }
            Entry::Vacant(vacant_entry) => vacant_entry.insert(t),
        }
    }

    pub fn or_insert(self, default: T) -> OccupiedEntry<'a, T> {
        match self {
            Entry::Occupied(occupied_entry) => occupied_entry,
            Entry::Vacant(vacant_entry) => vacant_entry.insert(default),
        }
    }

    pub fn or_insert_with(self, default: impl FnOnce() -> T) -> OccupiedEntry<'a, T> {
        match self {
            Entry::Occupied(occupied_entry) => occupied_entry,
            Entry::Vacant(vacant_entry) => vacant_entry.insert(default()),
        }
    }

    pub fn and_modify(mut self, f: impl FnOnce(&mut T)) -> Self {
        if let Entry::Occupied(ref mut occupied_entry) = self {
            f(occupied_entry.get_mut())
        }
        self
    }
}

pub struct OccupiedEntry<'a, T> {
    len: &'a mut usize,
    inner: hash_table::OccupiedEntry<'a, T>,
}

impl<'a, T> OccupiedEntry<'a, T> {
    pub fn remove(self) -> (T, VacantEntry<'a, T>) {
        *self.len -= 1;
        let (t, vacant) = self.inner.remove();
        (
            t,
            VacantEntry {
                len: self.len,
                inner: vacant,
            },
        )
    }

    pub fn get(&self) -> &T {
        self.inner.get()
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.inner.get_mut()
    }

    pub fn into_mut(self) -> &'a mut T {
        self.inner.into_mut()
    }
}

pub struct VacantEntry<'a, T> {
    len: &'a mut usize,
    inner: hash_table::VacantEntry<'a, T>,
}

impl<'a, T> VacantEntry<'a, T> {
    pub fn insert(self, t: T) -> OccupiedEntry<'a, T> {
        *self.len += 1;
        let occupied = self.inner.insert(t);

        OccupiedEntry {
            len: self.len,
            inner: occupied,
        }
    }
}

pub struct AbsentEntry<'a, T> {
    table: &'a mut IncHashTable<T>,
}

impl<'a, T> AbsentEntry<'a, T> {
    pub fn into_table(self) -> &'a mut IncHashTable<T> {
        self.table
    }
}

#[cfg(test)]
mod tests {
    use std::hash::{BuildHasher, RandomState};

    use super::IncHashTable;

    #[test]
    fn works() {
        let hasher = RandomState::new();
        let mut table = IncHashTable::<i32>::new();

        for i in 0..10000 {
            let entry = table.entry(hasher.hash_one(&i), |&x| x == i, |i| hasher.hash_one(i));
            let super::Entry::Vacant(e) = entry else {
                panic!()
            };
            e.insert(i);
        }

        assert_eq!(table.len(), 10000);

        // ceil(10000 / 896) = 12 tables
        // it's more likely to balance in a binary fashion though, so 16 tables.
        assert_eq!(table.trie.tables(), 16);
        // would occupy 16 slots, which means 31 entries in the triangular array
        assert_eq!(table.trie.trie_len(), 31);

        for i in 0..10000 {
            let entry = table.entry(hasher.hash_one(&i), |&x| x == i, |i| hasher.hash_one(i));
            let super::Entry::Occupied(e) = entry else {
                panic!()
            };
            if i % 2 == 0 {
                e.remove();
            }
        }

        assert_eq!(table.len(), 5000);
    }
}
