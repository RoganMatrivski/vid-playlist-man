use std::rc::Rc;

use anyhow::Result;
use worker::Cache;

#[derive(Clone)]
pub struct WorkerCache(Rc<Cache>);

impl WorkerCache {
    pub fn new() -> Self {
        Self(Rc::new(Cache::default()))
    }

    pub async fn get_json<T>(&self, key: impl AsRef<str>) -> Result<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        todo!()
    }

    pub async fn get_text(&self, key: impl AsRef<str>) -> Result<Option<String>> {
        todo!()
    }

    pub async fn set<T>(&self, key: impl AsRef<str>, value: T, ttl: u64) -> Result<()>
    where
        T: serde::ser::Serialize,
    {
        todo!()
    }

    pub async fn set_text(
        &self,
        key: impl AsRef<str>,
        value: impl ToString,
        ttl: u64,
    ) -> Result<()> {
        todo!()
    }
}
