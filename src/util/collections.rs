use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use std::cmp::Ord;

use std::collections::BTreeMap;
use std::ops::{Index, IndexMut};
use std::slice::{self, SliceIndex};

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct NonEmptyVec<T>(Vec<T>);

impl<T> NonEmptyVec<T> {
    pub(crate) fn try_new(vec: Vec<T>) -> Option<Self> {
        if vec.is_empty() {
            None
        } else {
            Some(NonEmptyVec(vec))
        }
    }

    pub(crate) fn last(&self) -> &T {
        self.0.last().unwrap()
    }

    pub(crate) fn iter(&self) -> slice::Iter<T> {
        self.0.iter()
    }

    pub(crate) fn max<R: Ord>(&self, f: impl Fn(&T) -> R) -> R {
        self.0.iter().map(&f).max().unwrap()
    }
}

impl<T: Default> Default for NonEmptyVec<T> {
    fn default() -> Self {
        NonEmptyVec(vec![T::default()])
    }
}

impl<T, I: SliceIndex<[T]>> Index<I> for NonEmptyVec<T> {
    type Output = I::Output;

    fn index(&self, index: I) -> &I::Output {
        &self.0[index]
    }
}

impl<T, I: SliceIndex<[T]>> IndexMut<I> for NonEmptyVec<T> {
    fn index_mut(&mut self, index: I) -> &mut <Self as Index<I>>::Output {
        &mut self.0[index]
    }
}

impl<'a, T> IntoIterator for &'a NonEmptyVec<T> {
    type Item = &'a T;
    type IntoIter = slice::Iter<'a, T>;

    fn into_iter(self) -> slice::Iter<'a, T> {
        self.0.iter()
    }
}

#[cfg_attr(test, derive(Debug, PartialEq))]
#[derive(Clone)]
pub(crate) struct SingleKeyValue<K, V> {
    pub(crate) key: K,
    pub(crate) value: V,
}

impl<K: Serialize, V: Serialize> Serialize for SingleKeyValue<K, V> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(&self.key, &self.value)?;
        map.end()
    }
}

impl<'de, K: Ord + Deserialize<'de>, V: Deserialize<'de>> Deserialize<'de>
    for SingleKeyValue<K, V>
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let map = BTreeMap::<K, V>::deserialize(deserializer)?;
        if map.len() == 1 {
            let (key, value) = map.into_iter().next().unwrap();
            Ok(Self { key, value })
        } else {
            Err(serde::de::Error::custom("expected single key-value pair"))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::util::collections::{NonEmptyVec, SingleKeyValue};

    #[test]
    fn test_non_empty_vec() {
        assert_eq!(NonEmptyVec::<()>::try_new(vec![]), None);
        assert_eq!(NonEmptyVec::try_new(vec![()]), Some(NonEmptyVec(vec![()])));
        assert_eq!(*NonEmptyVec::try_new(vec![42]).unwrap().last(), 42);
        assert_eq!(
            NonEmptyVec::try_new(vec![(); 10]).unwrap().iter().count(),
            10
        );
        assert_eq!(
            NonEmptyVec::try_new(vec![3, 1, 2])
                .unwrap()
                .max(|&x| 10 - x),
            9
        );
        assert_eq!(&mut NonEmptyVec::<()>::default()[0], &mut ());
        assert_eq!(NonEmptyVec::<()>::default(), NonEmptyVec(vec![()]));
        assert_eq!(
            (&NonEmptyVec::try_new(vec![(); 10]).unwrap())
                .into_iter()
                .count(),
            10
        );
    }

    #[test]
    fn test_ser_single_key_value() -> serde_json::Result<()> {
        let serialized = serde_json::to_string(&SingleKeyValue {
            key: "key",
            value: "value",
        })?;
        assert_eq!(serialized, r#"{"key":"value"}"#);
        Ok(())
    }

    #[test]
    fn test_de_single_key_value() -> serde_json::Result<()> {
        let deserialized =
            serde_json::from_str::<SingleKeyValue<String, String>>(r#"{"key":"value"}"#)?;
        assert_eq!(
            deserialized,
            SingleKeyValue {
                key: "key".to_owned(),
                value: "value".to_owned(),
            }
        );
        let err = serde_json::from_str::<SingleKeyValue<String, String>>(
            r#"{"key1":"value1","key2":"value2"}"#,
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "expected single key-value pair");
        Ok(())
    }
}
