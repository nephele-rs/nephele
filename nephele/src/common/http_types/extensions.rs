use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::hash::{BuildHasherDefault, Hasher};

#[derive(Default)]
pub struct Extensions {
    map: Option<HashMap<TypeId, Box<dyn Any + Send + Sync>, BuildHasherDefault<IdHasher>>>,
}

impl Extensions {
    #[inline]
    pub(crate) fn new() -> Self {
        Self { map: None }
    }

    pub fn insert<T: Send + Sync + 'static>(&mut self, val: T) -> Option<T> {
        self.map
            .get_or_insert_with(Default::default)
            .insert(TypeId::of::<T>(), Box::new(val))
            .and_then(|boxed| (boxed as Box<dyn Any>).downcast().ok().map(|boxed| *boxed))
    }

    pub fn contains<T: 'static>(&self) -> bool {
        self.map
            .as_ref()
            .and_then(|m| m.get(&TypeId::of::<T>()))
            .is_some()
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map
            .as_ref()
            .and_then(|m| m.get(&TypeId::of::<T>()))
            .and_then(|boxed| (&**boxed as &(dyn Any)).downcast_ref())
    }

    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.map
            .as_mut()
            .and_then(|m| m.get_mut(&TypeId::of::<T>()))
            .and_then(|boxed| (&mut **boxed as &mut (dyn Any)).downcast_mut())
    }

    pub fn remove<T: 'static>(&mut self) -> Option<T> {
        self.map
            .as_mut()
            .and_then(|m| m.remove(&TypeId::of::<T>()))
            .and_then(|boxed| (boxed as Box<dyn Any>).downcast().ok().map(|boxed| *boxed))
    }

    #[inline]
    pub fn clear(&mut self) {
        self.map = None;
    }
}

impl fmt::Debug for Extensions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Extensions").finish()
    }
}

#[derive(Default)]
struct IdHasher(u64);

impl Hasher for IdHasher {
    fn write(&mut self, _: &[u8]) {
        unreachable!("TypeId calls write_u64");
    }

    #[inline]
    fn write_u64(&mut self, id: u64) {
        self.0 = id;
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}
