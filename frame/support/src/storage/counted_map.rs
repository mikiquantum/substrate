use codec::{FullCodec, Decode, EncodeLike, Encode};
use crate::{
	storage::{
		StorageAppend, StorageTryAppend, StorageDecodeLength,
		types::{StorageMap, StorageValue, QueryKindTrait, ValueQuery},
	},
	traits::{StorageInstance, Get},
};
use sp_std::prelude::*;
use sp_runtime::traits::Saturating;

/// A wrapper around a `StorageMap` and a `StorageValue<u32>` to keep track of how many items are in
/// a map, without needing to iterate all the values.
///
/// This storage item has additional storage read and write overhead when manipulating values
/// compared to a regular storage map.
///
/// For functions where we only add or remove a value, a single storage read is needed to check if
/// that value already exists. For mutate functions, two storage reads are used to check if the
/// value existed before and after the mutation.
///
/// Whenever the counter needs to be updated, an additional read and write occurs to update that
/// counter.
pub struct CountedStorageMap<Map, Counter>(
	core::marker::PhantomData<(Map, Counter)>
);

/// Helper to get access to map and counter of `CountedStorageMap`.
trait Helper {
	type Map;
	type Counter;
}

impl<MapPrefix, MapHasher, MapKey, MapValue, MapQueryKind, MapOnEmpty, MapMaxValues, CounterPrefix>
	Helper for
	CountedStorageMap<StorageMap<MapPrefix, MapHasher, MapKey, MapValue, MapQueryKind, MapOnEmpty, MapMaxValues>, StorageValue<CounterPrefix, u32, ValueQuery>>
where
	MapPrefix: StorageInstance,
	MapHasher: crate::hash::StorageHasher,
	MapKey: FullCodec,
	MapValue: FullCodec,
	MapQueryKind: QueryKindTrait<MapValue, MapOnEmpty>,
	MapOnEmpty: Get<MapQueryKind::Query> + 'static,
	MapMaxValues: Get<Option<u32>>,
	CounterPrefix: StorageInstance,
{
	type Map = StorageMap<MapPrefix, MapHasher, MapKey, MapValue, MapQueryKind, MapOnEmpty, MapMaxValues>;
	type Counter = StorageValue<CounterPrefix, u32, ValueQuery>;
}

impl<MapPrefix, MapHasher, MapKey, MapValue, MapQueryKind, MapOnEmpty, MapMaxValues, CounterPrefix>
	CountedStorageMap<StorageMap<MapPrefix, MapHasher, MapKey, MapValue, MapQueryKind, MapOnEmpty, MapMaxValues>, StorageValue<CounterPrefix, u32, ValueQuery>>
where
	MapPrefix: StorageInstance,
	MapHasher: crate::hash::StorageHasher,
	MapKey: FullCodec,
	MapValue: FullCodec,
	MapQueryKind: QueryKindTrait<MapValue, MapOnEmpty>,
	MapOnEmpty: Get<MapQueryKind::Query> + 'static,
	MapMaxValues: Get<Option<u32>>,
	CounterPrefix: StorageInstance,
{
	// Internal helper function to track the counter as a value is mutated.
	fn mutate_counter<
		KeyArg: EncodeLike<MapKey> + Clone,
		M: FnOnce(KeyArg, F) -> R,
		F: FnOnce(I) -> R,
		I,
		R,
	>(m: M, key: KeyArg, f: F) -> R {
		let val_existed = <Self as Helper>::Map::contains_key(key.clone());
		let res = m(key.clone(), f);
		let val_exists = <Self as Helper>::Map::contains_key(key);

		if val_existed && !val_exists {
			// Value was deleted
			<Self as Helper>::Counter::mutate(|value| value.saturating_dec());
		} else if !val_existed && val_exists {
			// Value was added
			<Self as Helper>::Counter::mutate(|value| value.saturating_inc());
		}

		res
	}

	/// Get the storage key used to fetch a value corresponding to a specific key.
	pub fn hashed_key_for<KeyArg: EncodeLike<MapKey>>(key: KeyArg) -> Vec<u8> {
		<Self as Helper>::Map::hashed_key_for(key)
	}

	/// Does the value (explicitly) exist in storage?
	pub fn contains_key<KeyArg: EncodeLike<MapKey>>(key: KeyArg) -> bool {
		<Self as Helper>::Map::contains_key(key)
	}

	/// Load the value associated with the given key from the map.
	pub fn get<KeyArg: EncodeLike<MapKey>>(key: KeyArg) -> MapQueryKind::Query {
		<Self as Helper>::Map::get(key)
	}

	/// Try to get the value for the given key from the map.
	///
	/// Returns `Ok` if it exists, `Err` if not.
	pub fn try_get<KeyArg: EncodeLike<MapKey>>(key: KeyArg) -> Result<MapValue, ()> {
		<Self as Helper>::Map::try_get(key)
	}

	/// Swap the values of two keys.
	pub fn swap<KeyArg1: EncodeLike<MapKey>, KeyArg2: EncodeLike<MapKey>>(key1: KeyArg1, key2: KeyArg2) {
		<Self as Helper>::Map::swap(key1, key2)
	}

	/// Store a value to be associated with the given key from the map.
	pub fn insert<KeyArg: EncodeLike<MapKey> + Clone, ValArg: EncodeLike<MapValue>>(key: KeyArg, val: ValArg) {
		if !<Self as Helper>::Map::contains_key(key.clone()) {
			<Self as Helper>::Counter::mutate(|value| value.saturating_inc());
		}
		<Self as Helper>::Map::insert(key, val)
	}

	/// Remove the value under a key.
	pub fn remove<KeyArg: EncodeLike<MapKey> + Clone>(key: KeyArg) {
		if <Self as Helper>::Map::contains_key(key.clone()) {
			<Self as Helper>::Counter::mutate(|value| value.saturating_dec());
		}
		<Self as Helper>::Map::remove(key)
	}

	/// Mutate the value under a key.
	pub fn mutate<KeyArg: EncodeLike<MapKey> + Clone, R, F: FnOnce(&mut MapQueryKind::Query) -> R>(
		key: KeyArg,
		f: F
	) -> R {
		Self::mutate_counter(
			<Self as Helper>::Map::mutate,
			key,
			f,
		)
	}

	/// Mutate the item, only if an `Ok` value is returned.
	pub fn try_mutate<KeyArg, R, E, F>(key: KeyArg, f: F) -> Result<R, E>
	where
		KeyArg: EncodeLike<MapKey> + Clone,
		F: FnOnce(&mut MapQueryKind::Query) -> Result<R, E>,
	{
		Self::mutate_counter(
			<Self as Helper>::Map::try_mutate,
			key,
			f,
		)
	}

	/// Mutate the value under a key. Deletes the item if mutated to a `None`.
	pub fn mutate_exists<KeyArg: EncodeLike<MapKey> + Clone, R, F: FnOnce(&mut Option<MapValue>) -> R>(
		key: KeyArg,
		f: F
	) -> R {
		Self::mutate_counter(
			<Self as Helper>::Map::mutate_exists,
			key,
			f,
		)
	}

	/// Mutate the item, only if an `Ok` value is returned. Deletes the item if mutated to a `None`.
	pub fn try_mutate_exists<KeyArg, R, E, F>(key: KeyArg, f: F) -> Result<R, E>
	where
		KeyArg: EncodeLike<MapKey> + Clone,
		F: FnOnce(&mut Option<MapValue>) -> Result<R, E>,
	{
		Self::mutate_counter(
			<Self as Helper>::Map::try_mutate_exists,
			key,
			f,
		)
	}

	/// Take the value under a key.
	pub fn take<KeyArg: EncodeLike<MapKey> + Clone>(key: KeyArg) -> MapQueryKind::Query {
		if <Self as Helper>::Map::contains_key(key.clone()) {
			<Self as Helper>::Counter::mutate(|value| value.saturating_dec());
		}
		<Self as Helper>::Map::take(key)
	}

	/// Append the given items to the value in the storage.
	///
	/// `MapValue` is required to implement `codec::EncodeAppend`.
	///
	/// # Warning
	///
	/// If the storage item is not encoded properly, the storage will be overwritten and set to
	/// `[item]`. Any default value set for the storage item will be ignored on overwrite.
	pub fn append<Item, EncodeLikeItem, EncodeLikeKey>(key: EncodeLikeKey, item: EncodeLikeItem)
	where
		EncodeLikeKey: EncodeLike<MapKey> + Clone,
		Item: Encode,
		EncodeLikeItem: EncodeLike<Item>,
		MapValue: StorageAppend<Item>
	{
		if !<Self as Helper>::Map::contains_key(key.clone()) {
			<Self as Helper>::Counter::mutate(|value| value.saturating_inc());
		}
		<Self as Helper>::Map::append(key, item)
	}

	/// Read the length of the storage value without decoding the entire value under the given
	/// `key`.
	///
	/// `MapValue` is required to implement [`StorageDecodeLength`].
	///
	/// If the value does not exists or it fails to decode the length, `None` is returned. Otherwise
	/// `Some(len)` is returned.
	///
	/// # Warning
	///
	/// `None` does not mean that `get()` does not return a value. The default value is completly
	/// ignored by this function.
	pub fn decode_len<KeyArg: EncodeLike<MapKey>>(key: KeyArg) -> Option<usize>
		where MapValue: StorageDecodeLength,
	{
		<Self as Helper>::Map::decode_len(key)
	}

	/// Migrate an item with the given `key` from a defunct `OldHasher` to the current hasher.
	///
	/// If the key doesn't exist, then it's a no-op. If it does, then it returns its value.
	pub fn migrate_key<OldHasher: crate::hash::StorageHasher, KeyArg: EncodeLike<MapKey>>(
		key: KeyArg
	) -> Option<MapValue> {
		<Self as Helper>::Map::migrate_key::<OldHasher, _>(key)
	}

	/// Remove all value of the storage.
	pub fn remove_all() {
		<Self as Helper>::Counter::set(0u32);
		<Self as Helper>::Map::remove_all()
	}

	/// Iter over all value of the storage.
	///
	/// NOTE: If a value failed to decode becaues storage is corrupted then it is skipped.
	pub fn iter_values() -> crate::storage::PrefixIterator<MapValue> {
		<Self as Helper>::Map::iter_values()
	}

	/// Translate the values of all elements by a function `f`, in the map in no particular order.
	///
	/// By returning `None` from `f` for an element, you'll remove it from the map.
	///
	/// NOTE: If a value fail to decode because storage is corrupted then it is skipped.
	///
	/// # Warning
	///
	/// This function must be used with care, before being updated the storage still contains the
	/// old type, thus other calls (such as `get`) will fail at decoding it.
	///
	/// # Usage
	///
	/// This would typically be called inside the module implementation of on_runtime_upgrade.
	pub fn translate_values<OldValue: Decode, F: FnMut(OldValue) -> Option<MapValue>>(f: F) {
		<Self as Helper>::Map::translate_values(f)
	}

	/// Try and append the given item to the value in the storage.
	///
	/// Is only available if `MapValue` of the storage implements [`StorageTryAppend`].
	pub fn try_append<KArg, Item, EncodeLikeItem>(
		key: KArg,
		item: EncodeLikeItem,
	) -> Result<(), ()>
	where
		KArg: EncodeLike<MapKey> + Clone,
		Item: Encode,
		EncodeLikeItem: EncodeLike<Item>,
		MapValue: StorageTryAppend<Item>,
	{
		todo!()
		// <
		// 	Self as crate::storage::TryAppendMap<MapKey, MapValue, Item>
		// >::try_append(key, item)
	}

	/// Initialize the counter with the actual number of items in the map.
	///
	/// This function iterates through all the items in the map and sets the counter. This operation
	/// can be very heavy, so use with caution.
	///
	/// Returns the number of items in the map which is used to set the counter.
	pub fn initialize_counter() -> u32 {
		let count = Self::iter_values().count() as u32;
		<Self as Helper>::Counter::set(count);
		count
	}

	/// Return the count.
	pub fn count() -> u32 {
		<Self as Helper>::Counter::get()
	}
}
