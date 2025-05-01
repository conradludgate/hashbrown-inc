use std::borrow::Borrow;
use std::hash::{BuildHasher, Hash, RandomState};

use crate::Equivalent;
use crate::IncHashTable;
use crate::hash_table;
// use crate::hash_table;
// use crate::IncHashTable;

pub struct IncHashMap<K, V, S = RandomState> {
    table: IncHashTable<(K, V)>,
    hasher: S,
}

impl<K, V> IncHashMap<K, V, RandomState> {
    pub fn new() -> Self {
        Self::with_hasher(RandomState::default())
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_and_hasher(capacity, RandomState::default())
    }
}

impl<K, V, S> IncHashMap<K, V, S> {
    pub const fn with_hasher(hasher: S) -> Self {
        Self {
            table: IncHashTable::new(),
            hasher,
        }
    }

    pub fn with_capacity_and_hasher(capacity: usize, hasher: S) -> Self {
        Self {
            table: IncHashTable::with_capacity(capacity),
            hasher,
        }
    }

    pub fn hasher(&self) -> &S {
        &self.hasher
    }

    pub fn capacity(&self) -> usize {
        self.table.capacity()
    }

    pub fn len(&self) -> usize {
        self.table.len()
    }

    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    pub fn clear(&mut self) {
        self.table.clear();
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.table.iter().map(|(k, _v)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.table.iter().map(|(_k, v)| v)
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.table.iter_mut().map(|(_k, v)| v)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.table.iter().map(|(k, v)| (k, v))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&mut K, &mut V)> {
        self.table.iter_mut().map(|(k, v)| (k, v))
    }

    /// Clears the set, returning all elements in an iterator.
    pub fn drain(&mut self) -> impl Iterator<Item = (K, V)> + '_ {
        self.table.drain()
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
    pub fn extract_if<F>(&mut self, mut f: F) -> impl Iterator<Item = (K, V)>
    where
        F: FnMut(&K, &mut V) -> bool,
    {
        self.table.extract_if(move |(k, v)| f(k, v))
    }
}

impl<K: Hash + Eq, V, S: BuildHasher> IncHashMap<K, V, S> {
    pub fn shrink_to_fit(&mut self) {
        self.table.shrink_to_fit(|(k, _v)| self.hasher.hash_one(k));
    }

    pub fn retain(&mut self, mut f: impl FnMut(&K, &mut V) -> bool) {
        self.table
            .retain_rehash(|(k, v)| f(k, v), |(k, _)| self.hasher.hash_one(k));
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        Q: Hash + Equivalent<K> + ?Sized,
    {
        self.get_key_value(key).map(|(_k, v)| v)
    }

    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        Q: Hash + Equivalent<K> + ?Sized,
    {
        self.get_key_value_mut(key).map(|(_k, v)| v)
    }

    pub fn get_key_value<Q>(&self, key: &Q) -> Option<(&K, &V)>
    where
        Q: Hash + Equivalent<K> + ?Sized,
    {
        self.table
            .find(self.hasher.hash_one(key), |(k, _v)| key.equivalent(k))
            .map(|(k, v)| (k, v))
    }

    pub fn get_key_value_mut<Q>(&mut self, key: &Q) -> Option<(&K, &mut V)>
    where
        Q: Hash + Equivalent<K> + ?Sized,
    {
        self.table
            .find_entry(self.hasher.hash_one(key), |(k, _v)| key.equivalent(k))
            .ok()
            .map(|e| {
                let (k, v) = e.into_mut();
                (&*k, v)
            })
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        Q: Hash + Equivalent<K> + ?Sized,
    {
        self.get_key_value(key).is_some()
    }

    pub fn entry(&mut self, key: K) -> Entry<'_, K, V> {
        let hash = self.hasher.hash_one(&key);
        let entry = self
            .table
            .entry(hash, |(k, _v)| *k == key, |(k, _v)| self.hasher.hash_one(k));
        match entry {
            hash_table::Entry::Occupied(inner) => Entry::Occupied(OccupiedEntry { inner }),
            hash_table::Entry::Vacant(inner) => Entry::Vacant(VacantEntry { key, inner }),
        }
    }

    pub fn entry_ref<'a, 'b, Q>(&'a mut self, key: &'b Q) -> EntryRef<'a, 'b, K, Q, V>
    where
        Q: Hash + Equivalent<K> + ?Sized,
    {
        let hash = self.hasher.hash_one(&key);
        let entry = self.table.entry(
            hash,
            |(k, _v)| key.equivalent(k),
            |(k, _v)| self.hasher.hash_one(k),
        );
        match entry {
            hash_table::Entry::Occupied(inner) => EntryRef::Occupied(OccupiedEntry { inner }),
            hash_table::Entry::Vacant(inner) => EntryRef::Vacant(VacantEntryRef { key, inner }),
        }
    }

    pub fn insert(&mut self, k: K, v: V) -> Option<V> {
        match self.entry(k) {
            Entry::Occupied(mut e) => Some(e.insert(v)),
            Entry::Vacant(e) => {
                e.insert(v);
                None
            }
        }
    }

    pub fn try_insert(&mut self, key: K, value: V) -> Result<&mut V, OccupiedError<'_, K, V>> {
        match self.entry(key) {
            Entry::Occupied(entry) => Err(OccupiedError { entry, value }),
            Entry::Vacant(e) => Ok(e.insert(value)),
        }
    }

    pub fn remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        Q: Hash + Equivalent<K> + ?Sized,
    {
        self.remove_entry(key).map(|(_k, v)| v)
    }

    pub fn remove_entry<Q>(&mut self, key: &Q) -> Option<(K, V)>
    where
        Q: Hash + Equivalent<K> + ?Sized,
    {
        match self
            .table
            .find_entry(self.hasher.hash_one(key), |(k, _v)| key.equivalent(k))
        {
            Ok(e) => Some(e.remove().0),
            Err(_) => None,
        }
    }

    pub fn allocation_size(&self) -> usize {
        self.table.allocation_size()
    }
}

pub enum Entry<'a, K, V> {
    Occupied(OccupiedEntry<'a, K, V>),
    Vacant(VacantEntry<'a, K, V>),
}

impl<'a, K, V> Entry<'a, K, V> {
    pub fn insert(self, v: V) -> OccupiedEntry<'a, K, V> {
        match self {
            Self::Occupied(mut e) => {
                e.insert(v);
                e
            }
            Self::Vacant(e) => e.insert_entry(v),
        }
    }

    pub fn or_insert(self, default: V) -> &'a mut V {
        match self {
            Self::Occupied(e) => e.into_mut(),
            Self::Vacant(e) => e.insert(default),
        }
    }

    pub fn or_insert_with(self, default: impl FnOnce() -> V) -> &'a mut V {
        match self {
            Self::Occupied(e) => e.into_mut(),
            Self::Vacant(e) => e.insert(default()),
        }
    }

    pub fn or_insert_with_key(self, default: impl FnOnce(&K) -> V) -> &'a mut V {
        match self {
            Self::Occupied(e) => e.into_mut(),
            Self::Vacant(e) => {
                let v = default(e.key());
                e.insert(v)
            }
        }
    }

    pub fn key(&self) -> &K {
        match self {
            Self::Occupied(e) => e.key(),
            Self::Vacant(e) => e.key(),
        }
    }

    pub fn and_modify(mut self, f: impl FnOnce(&mut V)) -> Self {
        if let Self::Occupied(ref mut occupied_entry) = self {
            f(occupied_entry.get_mut())
        }
        self
    }

    pub fn and_replace_entry_with(self, f: impl FnOnce(&K, V) -> Option<V>) -> Self {
        match self {
            Self::Occupied(e) => {
                let ((key, v), inner) = e.inner.remove();
                match f(&key, v) {
                    Some(v) => Self::Occupied(OccupiedEntry {
                        inner: inner.insert((key, v)),
                    }),
                    None => Self::Vacant(VacantEntry { key, inner }),
                }
            }
            Self::Vacant(e) => Self::Vacant(e),
        }
    }

    pub fn or_default(self) -> &'a mut V
    where
        V: Default,
    {
        self.or_insert_with(V::default)
    }
}

pub struct OccupiedEntry<'a, K, V> {
    inner: hash_table::OccupiedEntry<'a, (K, V)>,
}

impl<'a, K, V> OccupiedEntry<'a, K, V> {
    pub fn remove(self) -> V {
        self.remove_entry().1
    }

    pub fn remove_entry(self) -> (K, V) {
        let ((k, v), _) = self.inner.remove();
        (k, v)
    }

    pub fn insert(&mut self, v: V) -> V {
        core::mem::replace(self.get_mut(), v)
    }

    pub fn key(&self) -> &K {
        &self.inner.get().0
    }

    pub fn get(&self) -> &V {
        &self.inner.get().1
    }

    pub fn get_mut(&mut self) -> &mut V {
        &mut self.inner.get_mut().1
    }

    pub fn into_mut(self) -> &'a mut V {
        &mut self.inner.into_mut().1
    }
}

pub struct VacantEntry<'a, K, V> {
    key: K,
    inner: hash_table::VacantEntry<'a, (K, V)>,
}

impl<'a, K, V> VacantEntry<'a, K, V> {
    pub fn insert_entry(self, v: V) -> OccupiedEntry<'a, K, V> {
        let inner = self.inner.insert((self.key, v));
        OccupiedEntry { inner }
    }

    pub fn insert(self, v: V) -> &'a mut V {
        self.insert_entry(v).into_mut()
    }

    pub fn key(&self) -> &K {
        &self.key
    }

    pub fn into_key(self) -> K {
        self.key
    }
}

pub enum EntryRef<'a, 'b, K, Q: ?Sized, V> {
    Occupied(OccupiedEntry<'a, K, V>),
    Vacant(VacantEntryRef<'a, 'b, K, Q, V>),
}

impl<'a, 'b, K, Q: ?Sized, V> EntryRef<'a, 'b, K, Q, V> {
    pub fn insert(self, v: V) -> OccupiedEntry<'a, K, V>
    where
        K: From<&'b Q>,
    {
        match self {
            Self::Occupied(mut e) => {
                e.insert(v);
                e
            }
            Self::Vacant(e) => e.insert_entry(v),
        }
    }

    pub fn or_insert(self, default: V) -> &'a mut V
    where
        K: From<&'b Q>,
    {
        match self {
            Self::Occupied(e) => e.into_mut(),
            Self::Vacant(e) => e.insert(default),
        }
    }

    pub fn or_insert_with(self, default: impl FnOnce() -> V) -> &'a mut V
    where
        K: From<&'b Q>,
    {
        match self {
            Self::Occupied(e) => e.into_mut(),
            Self::Vacant(e) => e.insert(default()),
        }
    }

    pub fn or_insert_with_key(self, default: impl FnOnce(&Q) -> V) -> &'a mut V
    where
        K: From<&'b Q>,
    {
        match self {
            Self::Occupied(e) => e.into_mut(),
            Self::Vacant(e) => {
                let v = default(e.key());
                e.insert(v)
            }
        }
    }

    pub fn key(&self) -> &Q
    where
        K: Borrow<Q>,
    {
        match self {
            Self::Occupied(e) => e.key().borrow(),
            Self::Vacant(e) => e.key(),
        }
    }

    pub fn and_modify(mut self, f: impl FnOnce(&mut V)) -> Self {
        if let Self::Occupied(ref mut occupied_entry) = self {
            f(occupied_entry.get_mut())
        }
        self
    }

    pub fn or_default(self) -> &'a mut V
    where
        V: Default,
        K: From<&'b Q>,
    {
        self.or_insert_with(V::default)
    }
}

pub struct VacantEntryRef<'a, 'b, K, Q: ?Sized, V> {
    key: &'b Q,
    inner: hash_table::VacantEntry<'a, (K, V)>,
}

impl<'a, 'b, K, Q: ?Sized, V> VacantEntryRef<'a, 'b, K, Q, V> {
    pub fn insert_entry(self, value: V) -> OccupiedEntry<'a, K, V>
    where
        K: From<&'b Q>,
    {
        let key = K::from(self.key);
        let inner = self.inner.insert((key, value));
        OccupiedEntry { inner }
    }

    pub fn insert(self, value: V) -> &'a mut V
    where
        K: From<&'b Q>,
    {
        self.insert_entry(value).into_mut()
    }

    pub fn key(&self) -> &'b Q {
        self.key
    }
}

pub struct OccupiedError<'a, K, V> {
    pub entry: OccupiedEntry<'a, K, V>,
    pub value: V,
}

impl<K, V, S> Extend<(K, V)> for IncHashMap<K, V, S>
where
    K: Hash + Eq,
    S: BuildHasher,
{
    fn extend<T: IntoIterator<Item = (K, V)>>(&mut self, iter: T) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
}

#[cfg(test)]
mod tests {
    use foldhash::fast::FixedState;

    use super::IncHashMap;

    #[test]
    fn works() {
        for i in 0..200 {
            let mut table = IncHashMap::<i32, i32, _>::with_hasher(FixedState::with_seed(i));

            for i in 0..10000 {
                let entry = table.entry(i);
                let super::Entry::Vacant(e) = entry else {
                    panic!()
                };
                e.insert(i);
            }

            assert_eq!(table.len(), 10000);

            for i in 0..10000 {
                let entry = table.entry(i);
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
}
