use core::fmt;

use hashbrown::{
    HashTable,
    hash_table::{self, DrainingTable},
};

use crate::trie::debug_unreachable;

pub struct IncHashTable<T> {
    old: hash_table::DrainingTable<T>,
    curr: HashTable<T>,
}

impl<T> IncHashTable<T> {
    pub const fn new() -> Self {
        Self {
            old: DrainingTable::empty(),
            curr: HashTable::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            old: DrainingTable::empty(),
            curr: HashTable::with_capacity(capacity),
        }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.curr.capacity()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.old.len() + self.curr.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn clear(&mut self) {
        self.old = DrainingTable::empty();
        self.curr.clear();
    }

    /// Shrinks the capacity of the table as much as possible. It will drop
    /// down as much as possible while maintaining the internal rules
    /// and possibly leaving some space in accordance with the resize policy.
    ///
    /// # Non-Incremental Warning
    ///
    /// This implementation is explicitly not incremental as we shrink the table
    /// as much as possible, which will force a rehash.
    ///
    /// `hasher` is called if entries need to be moved or copied to a new table.
    /// This must return the same hash value that each entry was inserted with.
    pub fn shrink_to_fit(&mut self, hasher: impl Fn(&T) -> u64) {
        self.retain_rehash_old(DrainingTable::empty(), |_| true, &hasher);
        self.curr.shrink_to_fit(hasher);
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.old.iter().chain(self.curr.iter())
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.old.iter_mut().chain(self.curr.iter_mut())
    }

    pub fn find(&self, hash: u64, eq: impl Fn(&T) -> bool) -> Option<&T> {
        self.old
            .find(hash, &eq)
            .or_else(|| self.curr.find(hash, eq))
    }

    /// Drains elements which are true under the given predicate,
    /// and returns an iterator over the removed items.
    ///
    /// In other words, move all elements `e` such that `f(&e)` returns `true` out
    /// into another iterator.
    ///
    /// If the returned `ExtractIf` is not exhausted, e.g. because it is dropped without iterating
    /// or the iteration short-circuits, then the remaining elements will be retained.
    /// Use [`retain()`] with a negated predicate if you do not need the returned iterator.
    ///
    /// [`retain()`]: IncHashTable2::retain
    pub fn extract_if<F>(&mut self, f: F) -> ExtractIf<'_, T, F>
    where
        F: FnMut(&mut T) -> bool,
    {
        let Self { old, curr } = self;

        ExtractIf {
            next: Some(curr),
            extract: Some(old.extract_if(f)),
        }
    }

    /// Clears the set, returning all elements in an iterator.
    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        let Self { old, curr } = self;

        let old = core::mem::replace(old, DrainingTable::empty());
        old.chain(curr.drain())
    }

    /// Retains only the elements specified by the predicate.
    ///
    /// In other words, remove all elements `e` such that `f(&e)` returns `false`.
    pub fn retain(&mut self, mut f: impl FnMut(&mut T) -> bool) {
        self.curr.retain(|v| f(v));
        // would be nice if we could move these into current from here, but we don't have the hasher available :(
        self.old.retain(f);
    }

    pub(crate) fn retain_rehash(
        &mut self,
        mut f: impl FnMut(&mut T) -> bool,
        hasher: impl Fn(&T) -> u64,
    ) {
        self.curr.retain(|v| f(v));
        self.retain_rehash_old(DrainingTable::empty(), f, hasher);
    }

    /// Returns an `OccupiedEntry` for an entry in the table with the given hash
    /// and which satisfies the equality function passed.
    ///
    /// This can be used to remove the entry from the table.
    ///
    /// This method will call `eq` for all entries with the given hash, but may
    /// also call it for entries with a different hash. `eq` should only return
    /// true for the desired entry, at which point the search is stopped.
    pub fn find_entry(
        &mut self,
        hash: u64,
        eq: impl Fn(&T) -> bool,
    ) -> Result<OccupiedEntry<'_, T>, AbsentEntry<'_, T>> {
        use polonius_the_crab::{polonius, polonius_return};

        let mut this = self;

        polonius!(
            |this| -> Result<OccupiedEntry<'polonius, T>, AbsentEntry<'_, T>> {
                if let Ok(inner) = this.old.find_entry(hash, &eq) {
                    let inner = this
                        .curr
                        .insert_unique(hash, inner.remove().0, |_| unreachable!());

                    polonius_return!(Ok(OccupiedEntry { inner }));
                }
                if let Ok(inner) = this.curr.find_entry(hash, eq) {
                    polonius_return!(Ok(OccupiedEntry { inner }));
                }
            }
        );

        Err(AbsentEntry { table: this })
    }

    #[inline]
    fn retain_rehash_old(
        &mut self,
        replace: DrainingTable<T>,
        mut f: impl FnMut(&mut T) -> bool,
        hasher: impl Fn(&T) -> u64,
    ) {
        let old = core::mem::replace(&mut self.old, replace);
        old.for_each(|mut e| {
            if f(&mut e) {
                // Safety: we always allocate enough space in curr for all old entries
                self.curr
                    .insert_unique(hasher(&e), e, |_| unsafe { debug_unreachable() });
            }
        });
    }

    #[inline]
    fn rehash_one(&mut self, hasher: impl Fn(&T) -> u64) -> bool {
        let Some(e) = self.old.next() else {
            return false;
        };

        // Safety: we always allocate enough space in curr for all old entries
        self.curr
            .insert_unique(hasher(&e), e, |_| unsafe { debug_unreachable() });

        if self.old.len() == 0 {
            // drop the allocation.
            self.old = DrainingTable::empty();
        }

        true
    }

    #[inline]
    fn find_old_entry_and_rehash(
        &mut self,
        hash: u64,
        eq: impl Fn(&T) -> bool,
        hasher: impl Fn(&T) -> u64,
    ) -> Result<hash_table::OccupiedEntry<'_, T>, hash_table::DrainingAbsentEntry<'_, T>> {
        let inner = self.old.find_entry(hash, &eq)?;
        let (e, _) = inner.remove();
        Ok(self
            .curr
            .insert_unique(hasher(&e), e, |_| unsafe { debug_unreachable() }))
    }

    #[inline(never)]
    fn reserve_realloc(&mut self, additional: usize, hasher: impl Fn(&T) -> u64) {
        debug_assert_eq!(
            self.old.len(),
            0,
            "due to the doubling nature of hashbrown, the draining should be full empty before the new table fills."
        );

        fn new_cap(old: usize, len: usize, additional: usize) -> Option<usize> {
            // must always be at least double the current length for our incremental requirements.
            // but always reserve more if additional is larger.
            let additional = usize::max(additional, len);
            // we need to have some extra for any straggling old entries.
            len.checked_add(additional)?.checked_add(old)
        }

        let new_cap = new_cap(self.old.len(), self.curr.len(), additional)
            .expect("new capacity should not overflow usize");

        // we always allocate a new table here to avoid any large rehash jobs.
        let next = HashTable::with_capacity(new_cap);
        assert!(next.capacity() >= new_cap);
        let curr = core::mem::replace(&mut self.curr, next).into_drain();

        // usually old is empty.
        // might be slow if a really large additional is requested
        self.retain_rehash_old(curr, |_| true, hasher);
    }

    /// Reserves enough capacity in the table for `additional` new entries to
    /// be inserted.
    ///
    /// # Warning
    /// While this table is incremental, if reserving additional elements needs to re-alloc,
    /// then we might be forced to rehash the remaining entries in the old map.
    pub fn reserve(&mut self, additional: usize, hasher: impl Fn(&T) -> u64) {
        // cannot underflow as we always allocate enough capacity to fit the old len.
        let growth_left = self.curr.capacity() - self.curr.len() - self.old.len();

        if additional > growth_left {
            self.reserve_realloc(additional, &hasher);

            let growth_left = self.curr.capacity() - self.curr.len();
            if additional > growth_left {
                dbg!(additional, growth_left);
                // safety: we just reserved it :)
                unsafe { debug_unreachable() }
            }
        }
    }

    /// Inserts an element into the `HashTable` with the given hash value, but
    /// without checking whether an equivalent element already exists within the
    /// table.
    ///
    /// `hasher` is called if entries need to be moved or copied to a new table.
    /// This must return the same hash value that each entry was inserted with.
    pub fn insert_unique(
        &mut self,
        hash: u64,
        value: T,
        hasher: impl Fn(&T) -> u64,
    ) -> OccupiedEntry<'_, T> {
        self.rehash_one(&hasher);
        self.reserve(1, hasher);
        let inner = self.curr.insert_unique(
            hash,
            value,
            // Safety: we reserved 1 entry.
            |_| unsafe { debug_unreachable() },
        );
        OccupiedEntry { inner }
    }

    /// Returns an `Entry` for an entry in the table with the given hash
    /// and which satisfies the equality function passed.
    ///
    /// This can be used to remove the entry from the table, or insert a new
    /// entry with the given hash if one doesn't already exist.
    ///
    /// This method will call `eq` for all entries with the given hash, but may
    /// also call it for entries with a different hash. `eq` should only return
    /// true for the desired entry, at which point the search is stopped.
    ///
    /// This method may grow the table in preparation for an insertion. Call
    /// [`IncHashTable2::find_entry`] if this is undesirable.
    ///
    /// `hasher` is called if entries need to be moved or copied to a new table.
    /// This must return the same hash value that each entry was inserted with.
    pub fn entry(
        &mut self,
        hash: u64,
        eq: impl Fn(&T) -> bool,
        hasher: impl Fn(&T) -> u64,
    ) -> Entry<'_, T> {
        use polonius_the_crab::{polonius, polonius_return};

        let mut this = self;

        polonius!(|this| -> Entry<'polonius, T> {
            if let Ok(inner) = this.find_old_entry_and_rehash(hash, &eq, &hasher) {
                polonius_return!(Entry::Occupied(OccupiedEntry { inner }));
            }
        });

        this.rehash_one(&hasher);
        this.reserve(1, hasher);
        let entry = this.curr.entry(
            hash,
            eq,
            // Safety: we reserved 1 entry.
            |_| unsafe { debug_unreachable() },
        );
        match entry {
            hash_table::Entry::Occupied(inner) => Entry::Occupied(OccupiedEntry { inner }),
            hash_table::Entry::Vacant(inner) => Entry::Vacant(VacantEntry { inner }),
        }
    }

    pub fn allocation_size(&self) -> usize {
        self.curr.allocation_size() + self.old.allocation_size()
    }
}

impl<T> IntoIterator for IncHashTable<T> {
    type Item = T;

    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            old: self.old,
            curr: self.curr.into_iter(),
        }
    }
}

pub struct IntoIter<T> {
    old: DrainingTable<T>,
    curr: hash_table::IntoIter<T>,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.old.next().or_else(|| self.curr.next())
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
    inner: hash_table::OccupiedEntry<'a, T>,
}

impl<T: fmt::Debug> fmt::Debug for OccupiedEntry<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OccupiedEntry")
            .field("value", self.get())
            .finish()
    }
}

impl<'a, T> OccupiedEntry<'a, T> {
    pub fn remove(self) -> (T, VacantEntry<'a, T>) {
        let (t, vacant) = self.inner.remove();
        (t, VacantEntry { inner: vacant })
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
    inner: hash_table::VacantEntry<'a, T>,
}

impl<T: fmt::Debug> fmt::Debug for VacantEntry<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VacantEntry")
    }
}

impl<'a, T> VacantEntry<'a, T> {
    pub fn insert(self, t: T) -> OccupiedEntry<'a, T> {
        let occupied = self.inner.insert(t);
        OccupiedEntry { inner: occupied }
    }
}

pub struct AbsentEntry<'a, T> {
    table: &'a mut IncHashTable<T>,
}

impl<T: fmt::Debug> fmt::Debug for AbsentEntry<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AbsentEntry")
    }
}

impl<'a, T> AbsentEntry<'a, T> {
    pub fn into_table(self) -> &'a mut IncHashTable<T> {
        self.table
    }
}

pub struct ExtractIf<'a, T, F> {
    next: Option<&'a mut HashTable<T>>,
    extract: Option<hash_table::ExtractIf<'a, T, F>>,
}

impl<T, F> Iterator for ExtractIf<'_, T, F>
where
    F: FnMut(&mut T) -> bool,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        let mut e = self.extract.take()?;
        let res = match e.next() {
            Some(res) => Some(res),
            None => match self.next.take() {
                Some(next) => {
                    e = next.extract_if(e.into_predicate());
                    e.next()
                }
                None => None,
            },
        };
        self.extract = Some(e);
        res
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
            let _absent = table
                .find_entry(hasher.hash_one(&i), |&x| x == i)
                .unwrap_err();

            let entry = table.entry(hasher.hash_one(&i), |&x| x == i, |i| hasher.hash_one(i));
            let super::Entry::Vacant(e) = entry else {
                panic!()
            };
            e.insert(i);

            let _occupied = table.find_entry(hasher.hash_one(&i), |&x| x == i).unwrap();
        }

        assert_eq!(table.len(), 10000);
        assert_eq!(table.allocation_size(), 122896);

        // // ceil(10000 / 896) = 12 tables
        // // it's more likely to balance in a binary fashion though, so 16 tables.
        // assert_eq!(table.trie.tables(), 16);
        // // would occupy 16 slots, which means 31 entries in the triangular array
        // assert_eq!(table.trie.trie_len(), 31);

        for i in 0..10000 {
            let entry = table.find_entry(hasher.hash_one(&i), |&x| x == i);
            let Ok(e) = entry else { panic!() };
            if i % 2 == 0 {
                e.remove();
            }
        }

        assert_eq!(table.len(), 5000);
    }
}
